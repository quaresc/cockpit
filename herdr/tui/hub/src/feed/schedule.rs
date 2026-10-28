//! When a source fetches next: every minute, at once when asked, later and later after errors (up to 15 minutes), and
//! not before a rate limit's reset, even when asked.

use chrono::TimeZone;

use super::http::Failure;

const EVERY: u64 = 60;
const LONGEST: u64 = 15 * 60;

#[derive(Debug, Default)]
pub struct Schedule {
    next: u64,
    failures: u32,
    limited_until: u64,
}

impl Schedule {
    pub fn is_due(&self, now: u64, is_asked: bool) -> bool {
        now >= self.limited_until && (is_asked || now >= self.next)
    }

    /// Plans the next fetch from this one's outcome; the error line to show, if any.
    pub fn after(&mut self, outcome: &Result<(), Failure>, now: u64) -> Option<String> {
        match outcome {
            Ok(()) => {
                (self.failures, self.limited_until, self.next) = (0, 0, now + EVERY);
                None
            }
            Err(Failure::Limited(until)) => {
                self.limited_until = *until;
                self.next = (*until).max(now + EVERY);
                Some(format!("rate limited, next try at {}", clock(self.next)))
            }
            Err(Failure::Other(message)) => {
                self.failures += 1;
                let delay = (EVERY << (self.failures - 1).min(4)).min(LONGEST);
                self.next = now + delay;
                Some(if delay > EVERY { format!("{message}; next try in {} min", delay / 60) } else { message.clone() })
            }
        }
    }
}

/// A time as the inbox shows its own, HH:MM local.
fn clock(secs: u64) -> String {
    chrono::Local.timestamp_opt(secs as i64, 0).single().map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_minute_or_at_once_when_asked() {
        let mut s = Schedule::default();
        assert!(s.is_due(1000, false));
        assert_eq!(s.after(&Ok(()), 1000), None);
        assert!(!s.is_due(1059, false));
        assert!(s.is_due(1059, true));
        assert!(s.is_due(1060, false));
    }

    #[test]
    fn errors_back_off_up_to_fifteen_minutes() {
        let mut s = Schedule::default();
        let fail = Err(Failure::Other("HTTP 502: bad gateway".into()));
        assert_eq!(s.after(&fail, 0).as_deref(), Some("HTTP 502: bad gateway"));
        assert!(s.is_due(60, false));
        assert_eq!(s.after(&fail, 60).as_deref(), Some("HTTP 502: bad gateway; next try in 2 min"));
        assert!(!s.is_due(179, false) && s.is_due(180, false));
        let delays: Vec<u64> = (0..5).map(|_| (s.after(&fail, 0), s.next).1).collect();
        assert_eq!(delays, [240, 480, 900, 900, 900]);
        assert!(s.is_due(1, true), "asking tries again at once after an error");
        s.after(&Ok(()), 0);
        assert_eq!(s.failures, 0);
    }

    #[test]
    fn a_rate_limit_waits_for_its_reset_even_when_asked() {
        let mut s = Schedule::default();
        let line = s.after(&Err(Failure::Limited(1000 + 600)), 1000).unwrap();
        assert!(line.starts_with("rate limited, next try at "), "{line}");
        assert!(!s.is_due(1599, true));
        assert!(s.is_due(1600, false));
        s.after(&Err(Failure::Limited(1010)), 1000);
        assert!(!s.is_due(1059, false) && s.is_due(1060, false), "a minute at least");
    }
}
