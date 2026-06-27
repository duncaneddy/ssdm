//! Short, bounded retry policy for a single product fetch within one sync pass.
//! The product's own interval is the long-term retry; in-run retries only smooth
//! over transient blips, so they are few and quick.

use std::time::Duration;

/// Total fetch attempts before giving up on a product for this pass.
pub const MAX_ATTEMPTS: u32 = 5;

const BASE: Duration = Duration::from_secs(1);
const CAP: Duration = Duration::from_secs(30);

/// Delay before retry `attempt` (1-based). Capped exponential: 1s, 2s, 4s, 8s, …≤30s.
pub fn backoff_delay(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(20);
    let secs = BASE.as_secs().saturating_mul(1u64 << shift);
    Duration::from_secs(secs).min(CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_capped_exponential() {
        assert_eq!(backoff_delay(1), Duration::from_secs(1));
        assert_eq!(backoff_delay(2), Duration::from_secs(2));
        assert_eq!(backoff_delay(3), Duration::from_secs(4));
        assert_eq!(backoff_delay(4), Duration::from_secs(8));
        assert_eq!(backoff_delay(5), Duration::from_secs(16));
        assert_eq!(backoff_delay(99), Duration::from_secs(30), "capped at 30s");
    }
}
