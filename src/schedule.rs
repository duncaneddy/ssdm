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

impl Schedule {
    /// True when the product should be fetched at `now_ms`.
    pub fn is_due(&self, last_attempt: Option<u64>, now_ms: u64) -> bool {
        match self {
            Schedule::Every(interval) => match last_attempt {
                None => true,
                Some(t) => now_ms.saturating_sub(t) >= interval.as_millis() as u64,
            },
            Schedule::WeeklyAt { weekday, time } => {
                let anchor = most_recent_anchor(*weekday, time.as_millis() as u64, now_ms);
                match last_attempt {
                    None => true,
                    Some(t) => t < anchor,
                }
            }
        }
    }

    /// Milliseconds until next due; 0 when already due.
    pub fn remaining_ms(&self, last_attempt: Option<u64>, now_ms: u64) -> u64 {
        match self {
            Schedule::Every(interval) => match last_attempt {
                None => 0,
                Some(t) => {
                    (interval.as_millis() as u64).saturating_sub(now_ms.saturating_sub(t))
                }
            },
            Schedule::WeeklyAt { weekday, time } => {
                if self.is_due(last_attempt, now_ms) {
                    return 0;
                }
                let anchor = most_recent_anchor(*weekday, time.as_millis() as u64, now_ms);
                (anchor + WEEK_MS).saturating_sub(now_ms)
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

/// Most recent instant of `weekday` at `tod_ms` into the UTC day, at or before `now_ms`.
fn most_recent_anchor(weekday: Weekday, tod_ms: u64, now_ms: u64) -> u64 {
    let day = now_ms / DAY_MS;
    let current_weekday = ((day + 3) % 7) as u8; // epoch day 0 was Thursday
    let wd = weekday as u8;
    let delta = (current_weekday + 7 - wd) % 7; // whole days since the target weekday
    let candidate = day.saturating_sub(delta as u64) * DAY_MS + tod_ms;
    if candidate > now_ms {
        candidate.saturating_sub(WEEK_MS) // today is the target weekday but before `tod_ms`
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
        assert!(s.is_due(None, 1000), "absent => due");
        assert!(!s.is_due(Some(1000), 1000 + 59_000), "before interval => not due");
        assert!(s.is_due(Some(1000), 1000 + 60_000), "at interval => due");
    }

    #[test]
    fn weekly_due_when_never_attempted() {
        assert!(weekly_thu().is_due(None, 3 * DAY + THU_1815));
    }

    #[test]
    fn weekly_not_due_before_anchor_time_on_target_day() {
        // day 7 is a Thursday; 01:00 UTC is before the 18:15 anchor.
        let now = 7 * DAY + 3_600_000;
        // attempted at last week's anchor (day 0 18:15); this week's anchor not reached.
        assert!(!weekly_thu().is_due(Some(THU_1815), now));
    }

    #[test]
    fn weekly_due_after_anchor_time_on_target_day() {
        let now = 7 * DAY + THU_1815 + 1; // just past Thursday 18:15
        assert!(weekly_thu().is_due(Some(THU_1815), now));
    }

    #[test]
    fn weekly_remaining_counts_down_to_next_thursday() {
        // attempted at day 0 18:15; now day 1 18:15 (Friday) => 6 days left.
        let now = 1 * DAY + THU_1815;
        assert_eq!(weekly_thu().remaining_ms(Some(THU_1815), now), 6 * DAY);
    }

    #[test]
    fn weekly_remaining_zero_when_due() {
        let now = 7 * DAY + THU_1815 + 1;
        assert_eq!(weekly_thu().remaining_ms(Some(THU_1815), now), 0);
    }

    #[test]
    fn nominal_period_every_is_self_weekly_is_seven_days() {
        assert_eq!(Schedule::Every(Duration::from_secs(7200)).nominal_period(),
                   Duration::from_secs(7200));
        assert_eq!(weekly_thu().nominal_period(), Duration::from_secs(7 * 24 * 3600));
    }

    #[test]
    fn anchor_does_not_underflow_for_early_epoch_non_thursday_target() {
        // now = day 2 (Saturday, since epoch day 0 = Thursday), well within the first week.
        let now = 2 * DAY + 3_600_000;
        // Sunday target => delta (2 - 6 mod 7) = 6 would drive day-delta negative pre-fix.
        let anchor = most_recent_anchor(Weekday::Sun, 0, now);
        assert!(anchor <= now, "anchor must be at or before now");
    }
}
