//! The descriptor the producer enqueues for each product fetch.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct IngestMessage {
    pub key: String,
    pub url: String,
    pub content_type: String,
    pub alias_key: Option<String>,
    pub enqueued_at: u64, // epoch milliseconds, set by the producer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip_preserves_fields() {
        let msg = IngestMessage {
            key: "eop/iers/finals_all/latest/finals.all.iau2000.txt".into(),
            url: "https://example.test/finals".into(),
            content_type: "text/plain".into(),
            alias_key: Some("eop/iers/c04/latest/EOP_C04_one_file_1962-now.txt".into()),
            enqueued_at: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: IngestMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }
}
