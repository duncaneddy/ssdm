//! Per-host minimum-interval gate plus a small randomized stagger, so many
//! same-host products coming due together do not burst against one upstream.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rand::Rng;

/// Lowercased host (authority) of a URL, or None if it does not parse.
pub fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok()?.host_str().map(|h| h.to_lowercase())
}

/// Remaining wait so a same-host request is at least `min` after the last one.
pub fn delay_needed(last: Option<Instant>, now: Instant, min: Duration) -> Duration {
    match last {
        None => Duration::ZERO,
        Some(t) => {
            let elapsed = now.saturating_duration_since(t);
            min.saturating_sub(elapsed)
        }
    }
}

pub struct RateLimiter {
    last: HashMap<String, Instant>,
    min_interval: Duration,
    jitter: Duration,
}

impl RateLimiter {
    pub fn new(min_interval: Duration, jitter: Duration) -> Self {
        Self { last: HashMap::new(), min_interval, jitter }
    }

    /// Sleep the per-host gap (if any) plus a random 0..jitter, then mark `now`.
    pub async fn throttle(&mut self, url: &str) {
        let host = host_of(url).unwrap_or_default();
        let wait = delay_needed(self.last.get(&host).copied(), Instant::now(), self.min_interval);
        let jitter = if self.jitter.is_zero() {
            Duration::ZERO
        } else {
            Duration::from_millis(rand::thread_rng().gen_range(0..=self.jitter.as_millis() as u64))
        };
        let total = wait + jitter;
        if !total.is_zero() {
            tokio::time::sleep(total).await;
        }
        self.last.insert(host, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn host_of_extracts_authority() {
        assert_eq!(host_of("https://celestrak.org/NORAD/x?y=1").as_deref(), Some("celestrak.org"));
        assert_eq!(host_of("https://datacenter.iers.org/data/f.txt").as_deref(), Some("datacenter.iers.org"));
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn delay_needed_enforces_min_interval() {
        let now = Instant::now();
        let min = Duration::from_secs(2);
        assert_eq!(delay_needed(None, now, min), Duration::ZERO, "first call: no wait");
        let last = now - Duration::from_millis(500);
        assert_eq!(delay_needed(Some(last), now, min), Duration::from_millis(1500));
        let old = now - Duration::from_secs(10);
        assert_eq!(delay_needed(Some(old), now, min), Duration::ZERO, "interval already passed");
    }
}
