//! Elapsed-based retry/backoff/drop policy for the queue consumer.
//!
//! worker 0.8.5 `Message` does not expose an attempt counter, so backoff is a
//! function of elapsed time since the message was enqueued. `next_delay_secs`
//! clamps the elapsed time into [BASE, CAP], which produces a capped-exponential
//! (doubling) curve: each wait is ~the total elapsed so far, up to the cap.

pub const BACKOFF_BASE_SECS: u64 = 300; // 5 min
pub const BACKOFF_CAP_SECS: u64 = 3600; // 1 h
pub const DROP_AFTER_SECS: u64 = 28_800; // 8 h

/// Delay before the next delivery, given seconds elapsed since enqueue.
pub fn next_delay_secs(elapsed_secs: u64) -> u32 {
    elapsed_secs.clamp(BACKOFF_BASE_SECS, BACKOFF_CAP_SECS) as u32
}

/// True once the message has lived past the 8h window and must be dropped.
pub fn should_drop(elapsed_secs: u64) -> bool {
    elapsed_secs >= DROP_AFTER_SECS
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_starts_at_base_and_caps() {
        assert_eq!(next_delay_secs(0), 300);
        assert_eq!(next_delay_secs(300), 300);
        assert_eq!(next_delay_secs(1200), 1200);
        assert_eq!(next_delay_secs(5000), 3600); // capped at 1h
    }

    #[test]
    fn drops_only_after_eight_hours() {
        assert!(!should_drop(0));
        assert!(!should_drop(28_799));
        assert!(should_drop(28_800));
        assert!(should_drop(40_000));
    }

}
