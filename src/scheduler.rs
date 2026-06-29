//! Per-product due selection and the daemon run loop.

use std::time::Duration;

use log::info;

use crate::config::Config;
use crate::keys::object_key;
use crate::products::Product;
use crate::ratelimit::RateLimiter;
use crate::status::Status;

/// Indices of active products that are due to be fetched at `now_ms`.
pub fn due_indices(all: &[Product], status: &Status, now_ms: u64) -> Vec<usize> {
    all.iter()
        .enumerate()
        .filter(|(_, p)| {
            p.active && {
                let e = status.get(&object_key(p));
                let la = e.map(|e| e.last_attempt);
                let lc = e.map(|e| e.last_checked);
                p.schedule.is_due(la, lc, now_ms)
            }
        })
        .map(|(i, _)| i)
        .collect()
}

/// Milliseconds until the next product becomes due, clamped to `cap_ms`.
/// Returns 0 when something is already due.
pub fn sleep_until_due_ms(all: &[Product], status: &Status, now_ms: u64, cap_ms: u64) -> u64 {
    let mut soonest = cap_ms;
    for p in all.iter().filter(|p| p.active) {
        let key = object_key(p);
        let e = status.get(&key);
        let la = e.map(|e| e.last_attempt);
        let lc = e.map(|e| e.last_checked);
        soonest = soonest.min(p.schedule.remaining_ms(la, lc, now_ms));
    }
    soonest
}

const WAKE_CAP: Duration = Duration::from_secs(3600);
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Run forever: on each wake, sync the due products, then sleep until the next one.
pub async fn run_daemon(cfg: &Config) -> anyhow::Result<()> {
    use crate::fetch::HttpFetcher;
    use crate::products::products;
    use crate::store::R2Store;

    let fetcher = HttpFetcher::new(FETCH_TIMEOUT, &cfg.site_domain)?;
    let store = R2Store::new(cfg)?;
    let mut rate = RateLimiter::new(cfg.host_min_interval, cfg.stagger_jitter);
    let all = products();

    // Seed local status from R2 on a fresh volume so the first pass doesn't
    // truncate an existing remote status.json before all products are synced.
    crate::sync::bootstrap_status(&store, &cfg.data_dir).await;

    if cfg.run_on_start {
        info!("RUN_ON_START set — forcing a full sync");
        let refs: Vec<&Product> = all.iter().filter(|p| p.active).collect();
        crate::sync::run_sync(&all, &refs, &fetcher, &store, &mut rate, &cfg.data_dir, &cfg.site_domain, now_ms()).await;
    }

    loop {
        let status = crate::local::load_status(&cfg.data_dir);
        let now = now_ms();
        let due = due_indices(&all, &status, now);
        if !due.is_empty() {
            let refs: Vec<&Product> = due.iter().map(|&i| &all[i]).collect();
            info!("{} product(s) due", refs.len());
            crate::sync::run_sync(&all, &refs, &fetcher, &store, &mut rate, &cfg.data_dir, &cfg.site_domain, now).await;
        }
        let status = crate::local::load_status(&cfg.data_dir);
        let sleep_ms = sleep_until_due_ms(&all, &status, now_ms(), WAKE_CAP.as_millis() as u64);
        info!("sleeping {}s until next due", sleep_ms / 1000);
        tokio::time::sleep(Duration::from_millis(sleep_ms.max(1000))).await;
    }
}

/// Current epoch milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::object_key;
    use crate::products::products;
    use crate::status::{apply_update, Status};

    // A realistic wall-clock instant (2023-11-14 ~22:13 UTC, a Tuesday): well past
    // any weekly anchor, as `now_ms` always is in production.
    const NOW: u64 = 1_700_000_000_000;

    #[test]
    fn everything_due_when_status_empty() {
        let all = products();
        let s = Status::new();
        let active = all.iter().filter(|p| p.active).count();
        assert_eq!(due_indices(&all, &s, NOW).len(), active);
        assert_eq!(sleep_until_due_ms(&all, &s, NOW, 3_600_000), 0);
    }

    #[test]
    fn sleeps_until_soonest_interval() {
        let all = products();
        let mut s = Status::new();
        for p in all.iter().filter(|p| p.active) {
            apply_update(&mut s, &object_key(p), "h", 1, NOW);
        }
        assert!(due_indices(&all, &s, NOW).is_empty(), "all just attempted => none due");
        // soonest cadence is the 8h CelesTrak/space-weather groups
        let sleep = sleep_until_due_ms(&all, &s, NOW, 24 * 3_600_000);
        assert_eq!(sleep, 8 * 3_600_000);
    }

    #[test]
    fn sleep_is_capped() {
        let all = products();
        let mut s = Status::new();
        for p in all.iter().filter(|p| p.active) {
            apply_update(&mut s, &object_key(p), "h", 1, NOW);
        }
        assert_eq!(sleep_until_due_ms(&all, &s, NOW, 60_000), 60_000, "clamped to cap");
    }

    #[test]
    fn weekly_failed_attempt_is_retried_via_last_checked() {
        use crate::schedule::{Schedule, Weekday};
        use crate::status::StatusEntry;
        use std::time::Duration;

        let weekly = Product {
            category: "eop", source: "usno", name: "finals_test",
            url: "https://h/finals".into(), filename: "finals.all".into(),
            content_type: "text/plain", active: true, alias_name: None,
            info_url: None, cadence_label: None,
            schedule: Schedule::WeeklyAt {
                weekday: Weekday::Thu,
                time: Duration::from_secs(18 * 3600 + 15 * 60),
            },
        };
        let all = vec![weekly];
        let key = object_key(&all[0]);

        // A fetch was attempted recently and FAILED: last_attempt is fresh (NOW),
        // but the last successful download was over a week ago.
        let mut s = Status::new();
        s.insert(key, StatusEntry {
            last_attempt: NOW,
            last_checked: NOW - 8 * 86_400_000,
            last_updated: NOW - 8 * 86_400_000,
            hash: "h".into(),
            size: 1,
        });

        // An hour after the failed attempt (the retry backoff), it must be due again.
        let later = NOW + 3_600_000;
        assert_eq!(
            due_indices(&all, &s, later),
            vec![0],
            "a weekly product that only failed must be retried, not skipped for a week"
        );
    }
}
