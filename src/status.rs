//! The `status.json` model: per-product freshness + content hash, and the
//! change-detection logic that decides whether to re-write a product to R2.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
pub struct StatusEntry {
    pub last_checked: u64,  // epoch ms — every successful download
    pub last_updated: u64,  // epoch ms — only when the content hash changed
    pub last_attempt: u64,  // epoch ms — every fetch attempt, success or failure
    pub hash: String,       // md5 hex of the current content
    pub size: u64,          // bytes
}

pub type Status = BTreeMap<String, StatusEntry>;

/// Parse `status.json`; an absent/corrupt file yields an empty map.
pub fn parse_status(bytes: &[u8]) -> Status {
    serde_json::from_slice(bytes).unwrap_or_default()
}

/// Serialize the status map deterministically (BTreeMap → sorted keys).
pub fn serialize_status(s: &Status) -> String {
    serde_json::to_string(s).expect("status serializes")
}

/// md5 hex digest, used for change detection and as a copyable fingerprint.
pub fn content_hash(bytes: &[u8]) -> String {
    format!("{:x}", md5::compute(bytes))
}

/// Record a successful fetch. Always bumps `last_checked`; bumps `last_updated`
/// and the stored hash only when the content changed (or is new). Returns
/// `true` when the caller should write the bytes to R2 (content changed/new).
pub fn apply_update(
    status: &mut Status,
    key: &str,
    new_hash: &str,
    size: u64,
    now_ms: u64,
) -> bool {
    let prior = status.get(key).cloned();
    let changed = prior.as_ref().map(|e| e.hash != new_hash).unwrap_or(true);
    let last_updated = if changed {
        now_ms
    } else {
        prior.as_ref().map(|e| e.last_updated).unwrap_or(now_ms)
    };
    status.insert(
        key.to_string(),
        StatusEntry {
            last_checked: now_ms,
            last_updated,
            last_attempt: now_ms,
            hash: new_hash.to_string(),
            size,
        },
    );
    changed
}

/// Record a fetch attempt (success or failure) without touching success fields.
pub fn record_attempt(status: &mut Status, key: &str, now_ms: u64) {
    let mut e = status.get(key).cloned().unwrap_or_default();
    e.last_attempt = now_ms;
    status.insert(key.to_string(), e);
}

/// True when a product should be fetched: never attempted, or its interval elapsed.
pub fn is_due(status: &Status, key: &str, interval: std::time::Duration, now_ms: u64) -> bool {
    match status.get(key) {
        None => true,
        Some(e) => now_ms.saturating_sub(e.last_attempt) >= interval.as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_known_md5() {
        assert_eq!(content_hash(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(content_hash(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn new_key_writes_data_and_sets_both_times() {
        let mut s = Status::new();
        let wrote = apply_update(&mut s, "k", "hash1", 10, 1000);
        assert!(wrote);
        let e = &s["k"];
        assert_eq!(e.last_checked, 1000);
        assert_eq!(e.last_updated, 1000);
        assert_eq!(e.hash, "hash1");
        assert_eq!(e.size, 10);
    }

    #[test]
    fn unchanged_hash_bumps_checked_only_and_skips_write() {
        let mut s = Status::new();
        apply_update(&mut s, "k", "hash1", 10, 1000);
        let wrote = apply_update(&mut s, "k", "hash1", 10, 5000);
        assert!(!wrote, "unchanged content must not re-write");
        let e = &s["k"];
        assert_eq!(e.last_checked, 5000);
        assert_eq!(e.last_updated, 1000, "last_updated frozen when unchanged");
    }

    #[test]
    fn changed_hash_writes_and_bumps_updated() {
        let mut s = Status::new();
        apply_update(&mut s, "k", "hash1", 10, 1000);
        let wrote = apply_update(&mut s, "k", "hash2", 12, 5000);
        assert!(wrote);
        let e = &s["k"];
        assert_eq!(e.last_checked, 5000);
        assert_eq!(e.last_updated, 5000);
        assert_eq!(e.hash, "hash2");
    }

    #[test]
    fn round_trip_through_json() {
        let mut s = Status::new();
        apply_update(&mut s, "k", "hash1", 10, 1000);
        let json = serialize_status(&s);
        assert_eq!(parse_status(json.as_bytes()), s);
    }

    #[test]
    fn missing_file_parses_to_empty() {
        assert!(parse_status(b"").is_empty());
        assert!(parse_status(b"not json").is_empty());
    }

    #[test]
    fn apply_update_sets_last_attempt() {
        let mut s = Status::new();
        apply_update(&mut s, "k", "h", 1, 1000);
        assert_eq!(s["k"].last_attempt, 1000);
    }

    #[test]
    fn record_attempt_bumps_only_last_attempt() {
        let mut s = Status::new();
        apply_update(&mut s, "k", "h", 1, 1000);
        record_attempt(&mut s, "k", 5000);
        let e = &s["k"];
        assert_eq!(e.last_attempt, 5000);
        assert_eq!(e.last_checked, 1000, "success fields untouched on a bare attempt");
        assert_eq!(e.hash, "h");
    }

    #[test]
    fn record_attempt_creates_minimal_entry_when_absent() {
        let mut s = Status::new();
        record_attempt(&mut s, "new", 5000);
        let e = &s["new"];
        assert_eq!(e.last_attempt, 5000);
        assert_eq!(e.last_checked, 0);
        assert!(e.hash.is_empty());
    }

    #[test]
    fn is_due_when_absent_or_interval_elapsed() {
        use std::time::Duration;
        let mut s = Status::new();
        assert!(is_due(&s, "k", Duration::from_secs(60), 1000), "absent => due");
        apply_update(&mut s, "k", "h", 1, 1000);
        assert!(!is_due(&s, "k", Duration::from_secs(60), 1000 + 59_000));
        assert!(is_due(&s, "k", Duration::from_secs(60), 1000 + 60_000));
    }
}
