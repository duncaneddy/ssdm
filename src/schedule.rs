//! Product fetch cadence: a recurring elapsed interval, or a UTC
//! weekday + time-of-day anchor. Pure epoch-ms math, no date library.

use std::time::Duration;

const DAY_MS: u64 = 86_400_000;
const WEEK_MS: u64 = 7 * DAY_MS;

/// Day of week in UTC. Ordering matches `(epoch_day + 3) % 7` (Monday = 0),
/// since epoch day 0 (1970-01-01) was a Thursday.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weekday {
    Mon = 0,
    Tue = 1,
    Wed = 2,
    Thu = 3,
    Fri = 4,
    Sat = 5,
    Sun = 6,
}

/// When a product is due to be fetched.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Schedule {
    /// Recurring: due once `interval` has elapsed since the last attempt.
    Every(Duration),
    /// Anchored: due once per week, at `weekday` on/after `time` into the UTC day.
    WeeklyAt { weekday: Weekday, time: Duration },
}

/// A failed fetch retries on this cadence until it succeeds, rather than waiting
/// out the product's full interval.
///
/// This matters most for the longest intervals: without it, one transient
/// failure on a 30-day star catalog — a dropped part, or a timed-out 500 MB
/// upload — would leave that product stale for a further 30 days while the
/// daemon sat idle, because a failed attempt is still an attempt.
const RETRY_MS: u64 = 3_600_000; // 1 hour

/// Did the most recent attempt succeed? `apply_update` stamps both `last_checked`
/// and `last_attempt` on success, while `record_attempt` advances only
/// `last_attempt`, so a failure leaves `last_checked` behind.
fn last_attempt_succeeded(last_attempt: Option<u64>, last_checked: Option<u64>) -> bool {
    match (last_attempt, last_checked) {
        (Some(la), Some(lc)) => lc >= la,
        _ => false,
    }
}

impl Schedule {
    /// True when the product should be fetched at `now_ms`.
    ///
    /// `last_attempt` is the last fetch attempt (success or failure); `last_checked`
    /// is the last *successful* download. Both schedule kinds key due-ness off
    /// success, so a failed attempt does not consume the interval.
    pub fn is_due(&self, last_attempt: Option<u64>, last_checked: Option<u64>, now_ms: u64) -> bool {
        match self {
            Schedule::Every(interval) => match last_attempt {
                None => true,
                Some(t) => now_ms.saturating_sub(t) >= every_wait_ms(*interval, last_attempt, last_checked),
            },
            Schedule::WeeklyAt { weekday, time } => {
                let anchor = most_recent_anchor(*weekday, time.as_millis() as u64, now_ms);
                // already fetched successfully since this week's anchor?
                if last_checked.is_some_and(|c| c >= anchor) {
                    return false;
                }
                // need a fetch this week: due on the first try, then retry on a backoff.
                match last_attempt {
                    None => true,
                    Some(t) => now_ms.saturating_sub(t) >= RETRY_MS,
                }
            }
        }
    }

    /// Milliseconds until next due; 0 when already due.
    pub fn remaining_ms(&self, last_attempt: Option<u64>, last_checked: Option<u64>, now_ms: u64) -> u64 {
        match self {
            Schedule::Every(interval) => match last_attempt {
                None => 0,
                Some(t) => every_wait_ms(*interval, last_attempt, last_checked)
                    .saturating_sub(now_ms.saturating_sub(t)),
            },
            Schedule::WeeklyAt { weekday, time } => {
                let anchor = most_recent_anchor(*weekday, time.as_millis() as u64, now_ms);
                if last_checked.is_some_and(|c| c >= anchor) {
                    // succeeded this week — wait for next week's anchor.
                    return (anchor + WEEK_MS).saturating_sub(now_ms);
                }
                // need a fetch: due now on the first try, else count down the retry backoff.
                match last_attempt {
                    None => 0,
                    Some(t) => (t + RETRY_MS).saturating_sub(now_ms),
                }
            }
        }
    }

    /// Nominal period for display/freshness: the interval, or one week for weekly.
    pub fn nominal_period(&self) -> Duration {
        match self {
            Schedule::Every(d) => *d,
            Schedule::WeeklyAt { .. } => Duration::from_secs(WEEK_MS / 1000),
        }
    }
}

/// How long an `Every` product waits before its next attempt: its full interval
/// after a success, or the shorter retry cadence after a failure. Capped by the
/// interval so a sub-hour product is never slowed down by the retry floor.
fn every_wait_ms(interval: Duration, last_attempt: Option<u64>, last_checked: Option<u64>) -> u64 {
    let interval_ms = interval.as_millis() as u64;
    if last_attempt_succeeded(last_attempt, last_checked) {
        interval_ms
    } else {
        RETRY_MS.min(interval_ms)
    }
}

/// Most recent instant of `weekday` at `tod_ms` into the UTC day, at or before `now_ms`.
fn most_recent_anchor(weekday: Weekday, tod_ms: u64, now_ms: u64) -> u64 {
    let day = now_ms / DAY_MS;
    let current_weekday = ((day + 3) % 7) as u8; // epoch day 0 was Thursday
    let wd = weekday as u8;
    let delta = (current_weekday + 7 - wd) % 7; // whole days since the target weekday
    let candidate = (day - delta as u64) * DAY_MS + tod_ms;
    if candidate > now_ms {
        candidate - WEEK_MS // today is the target weekday but before `tod_ms`
    } else {
        candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400_000;
    const THU_1815: u64 = (18 * 3600 + 15 * 60) * 1000; // 65_700_000

    fn weekly_thu() -> Schedule {
        Schedule::WeeklyAt { weekday: Weekday::Thu, time: Duration::from_millis(THU_1815) }
    }

    #[test]
    fn epoch_day_zero_is_thursday() {
        // most-recent Thursday-at-00:00 anchor for now=0 is 0 itself.
        assert_eq!(most_recent_anchor(Weekday::Thu, 0, 0), 0);
    }

    #[test]
    fn every_matches_legacy_interval_semantics() {
        let s = Schedule::Every(Duration::from_secs(60));
        assert!(s.is_due(None, None, 1000), "absent => due");
        assert!(!s.is_due(Some(1000), None, 1000 + 59_000), "before interval => not due");
        assert!(s.is_due(Some(1000), None, 1000 + 60_000), "at interval => due");
    }

    #[test]
    fn every_retries_a_failure_instead_of_waiting_out_a_long_interval() {
        // The star catalogs poll monthly. A failed attempt is still an attempt, so
        // keying purely off last_attempt would park a failed 500 MB upload for a
        // further 30 days with the daemon idle. A failure must retry on the short
        // cadence; only a success consumes the interval.
        let monthly = Schedule::Every(Duration::from_secs(30 * 24 * 3600));
        let t = 1_000_000_000_000;

        // Attempted at t, never succeeded (record_attempt leaves last_checked behind).
        assert!(!monthly.is_due(Some(t), Some(0), t + RETRY_MS - 1), "retry not yet due");
        assert!(
            monthly.is_due(Some(t), Some(0), t + RETRY_MS),
            "a failed attempt retries within the hour, not in 30 days"
        );
        assert_eq!(monthly.remaining_ms(Some(t), Some(0), t), RETRY_MS);

        // Succeeded at t: the interval applies in full.
        let month = 30 * DAY;
        assert!(!monthly.is_due(Some(t), Some(t), t + month - 1), "success consumes the interval");
        assert!(monthly.is_due(Some(t), Some(t), t + month));
        assert_eq!(monthly.remaining_ms(Some(t), Some(t), t), month);
    }

    #[test]
    fn every_retry_floor_never_slows_a_short_interval() {
        // The retry cadence is a ceiling on waiting, not a floor: a product that
        // polls faster than the retry interval keeps its own cadence after a failure.
        let fast = Schedule::Every(Duration::from_secs(60));
        let t = 1_000_000_000_000;
        assert!(fast.is_due(Some(t), Some(0), t + 60_000), "still due at its own 60s interval");
        assert_eq!(fast.remaining_ms(Some(t), Some(0), t), 60_000);
    }

    #[test]
    fn weekly_due_when_never_attempted() {
        assert!(weekly_thu().is_due(None, None, 3 * DAY + THU_1815));
    }

    #[test]
    fn weekly_not_due_before_anchor_time_on_target_day() {
        // day 7 is a Thursday; 01:00 UTC is before the 18:15 anchor.
        let now = 7 * DAY + 3_600_000;
        // succeeded at last week's anchor (day 0 18:15); this week's anchor not reached.
        assert!(!weekly_thu().is_due(Some(THU_1815), Some(THU_1815), now));
    }

    #[test]
    fn weekly_due_after_anchor_time_on_target_day() {
        let now = 7 * DAY + THU_1815 + 1; // just past Thursday 18:15
        // last success was a week ago at day 0 18:15.
        assert!(weekly_thu().is_due(Some(THU_1815), Some(THU_1815), now));
    }

    #[test]
    fn weekly_remaining_counts_down_to_next_thursday() {
        // succeeded at day 0 18:15; now day 1 18:15 (Friday) => 6 days left.
        let now = DAY + THU_1815;
        assert_eq!(weekly_thu().remaining_ms(Some(THU_1815), Some(THU_1815), now), 6 * DAY);
    }

    #[test]
    fn weekly_remaining_zero_when_due() {
        let now = 7 * DAY + THU_1815 + 1;
        assert_eq!(weekly_thu().remaining_ms(Some(THU_1815), Some(THU_1815), now), 0);
    }

    #[test]
    fn weekly_retries_after_failed_attempt() {
        // This week's anchor (day 7 18:15) passed; a fetch was attempted 1 min later
        // and FAILED, so last_attempt advanced past the anchor but no success was
        // recorded since last week (last_checked = day 0 18:15).
        let anchor = 7 * DAY + THU_1815;
        let failed_at = anchor + 60_000;
        // Immediately after the failure: backoff not elapsed => not due yet.
        assert!(
            !weekly_thu().is_due(Some(failed_at), Some(THU_1815), failed_at + 60_000),
            "should back off briefly, not hammer"
        );
        // After the retry backoff elapses: due again despite last_attempt > anchor.
        let now = failed_at + RETRY_MS;
        assert!(
            weekly_thu().is_due(Some(failed_at), Some(THU_1815), now),
            "a failed weekly fetch must be retried, not skipped until next week"
        );
    }

    #[test]
    fn weekly_retry_backoff_counts_down_after_failure() {
        let anchor = 7 * DAY + THU_1815;
        let failed_at = anchor + 60_000;
        let now = failed_at + 60_000; // 1 min after the failed attempt
        assert_eq!(
            weekly_thu().remaining_ms(Some(failed_at), Some(THU_1815), now),
            RETRY_MS - 60_000,
            "remaining counts down the retry backoff, not a full week"
        );
    }

    #[test]
    fn weekly_not_due_after_success_this_week() {
        // Succeeded right at this week's anchor; not due again until next week.
        let anchor = 7 * DAY + THU_1815;
        let now = anchor + 3_600_000; // 1h after the successful fetch
        assert!(!weekly_thu().is_due(Some(anchor), Some(anchor), now));
        assert_eq!(
            weekly_thu().remaining_ms(Some(anchor), Some(anchor), now),
            WEEK_MS - 3_600_000
        );
    }

    #[test]
    fn nominal_period_every_is_self_weekly_is_seven_days() {
        assert_eq!(Schedule::Every(Duration::from_secs(7200)).nominal_period(),
                   Duration::from_secs(7200));
        assert_eq!(weekly_thu().nominal_period(), Duration::from_secs(7 * 24 * 3600));
    }
}
