//! The week as the mod caches it in ~/.cache/cockpit/meetings.json, the times meetings.py words, and their briefs.

use std::path::PathBuf;

use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeDelta, Utc};
use cockpit_core::files;
use serde_json::Value;

/// The time now and the zone it is shown in: the machine's, or a fixed one for tests.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub now: DateTime<Utc>,
    pub zone: Option<FixedOffset>,
}

impl Clock {
    pub fn system() -> Self {
        Self { now: Utc::now(), zone: None }
    }

    /// The wall-clock time of an instant here.
    pub fn local(&self, at: DateTime<Utc>) -> NaiveDateTime {
        match self.zone {
            Some(zone) => at.with_timezone(&zone).naive_local(),
            None => at.with_timezone(&Local).naive_local(),
        }
    }

    pub fn now_local(&self) -> NaiveDateTime {
        self.local(self.now)
    }

    pub fn today(&self) -> NaiveDate {
        self.now_local().date()
    }

    /// The minute it is, which the "in N min" texts change with.
    pub fn minute(&self) -> i64 {
        self.now.timestamp() / 60
    }
}

#[derive(Clone, Debug)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub url: String,
    pub join: Option<String>,
    pub response: String,
    pub attendees: u64,
    pub people: Vec<String>,
    pub briefable: bool,
    /// The meeting as the cache has it, for the actions run in Python.
    pub raw: Value,
}

fn instant(value: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.as_str()?).ok().map(|t| t.with_timezone(&Utc))
}

impl Meeting {
    fn parse(raw: &Value) -> Option<Self> {
        Some(Self {
            id: raw["id"].as_str()?.to_string(),
            title: raw["title"].as_str().unwrap_or_default().to_string(),
            start: instant(&raw["start"])?,
            end: instant(&raw["end"])?,
            url: raw["url"].as_str().unwrap_or_default().to_string(),
            join: raw["join"].as_str().filter(|j| !j.is_empty()).map(str::to_string),
            response: raw["response"].as_str().unwrap_or("accepted").to_string(),
            attendees: raw["attendees"].as_u64().unwrap_or(0),
            people: raw["people"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
            briefable: raw["briefable"].as_bool().unwrap_or(false),
            raw: raw.clone(),
        })
    }

    pub fn is_past(&self, clock: &Clock) -> bool {
        self.end <= clock.now
    }

    pub fn is_now(&self, clock: &Clock) -> bool {
        self.start <= clock.now && clock.now < self.end
    }

    /// Starting within five minutes: the row pulses.
    pub fn is_soon(&self, clock: &Clock) -> bool {
        self.start > clock.now && self.start - clock.now <= TimeDelta::minutes(5)
    }

    pub fn minutes(&self) -> i64 {
        (self.end - self.start).num_seconds() / 60
    }

    /// "15 min", or "1.5 h" from an hour and a half.
    pub fn duration_text(&self) -> String {
        let minutes = self.minutes();
        if minutes < 90 { format!("{minutes} min") } else { format!("{:.1} h", minutes as f64 / 60.0) }
    }

    pub fn span_text(&self, clock: &Clock) -> String {
        format!("{}–{}", clock.local(self.start).format("%H:%M"), clock.local(self.end).format("%H:%M"))
    }
}

/// What meetings.json holds; None before the mod first wrote it.
#[derive(Clone, Debug, Default)]
pub struct Week {
    pub at: Option<i64>,
    pub error: Option<String>,
    pub meetings: Vec<Meeting>,
}

impl Week {
    pub fn parse(data: &Value) -> Self {
        let mut meetings: Vec<Meeting> = data["meetings"].as_array().into_iter().flatten().filter_map(Meeting::parse).collect();
        meetings.sort_by_key(|m| m.start);
        Self {
            at: data["at"].as_i64().or_else(|| data["at"].as_f64().map(|f| f as i64)),
            error: data["error"].as_str().filter(|e| !e.is_empty()).map(str::to_string),
            meetings,
        }
    }

    /// The meetings shown: from an hour ago on, by start.
    pub fn shown(&self, clock: &Clock) -> Vec<&Meeting> {
        self.meetings.iter().filter(|m| m.end > clock.now - TimeDelta::hours(1)).collect()
    }

    /// The meetings not over yet, the running one first.
    pub fn upcoming(&self, clock: &Clock) -> Vec<&Meeting> {
        self.meetings.iter().filter(|m| m.end > clock.now).collect()
    }
}

pub fn cache_path() -> PathBuf {
    files::cache().join("meetings.json")
}

/// "in 12 min", "in 2 h 05", else the time, as meetings.py says when a meeting starts.
pub fn until_text(start: DateTime<Utc>, clock: &Clock) -> String {
    let minutes = (start - clock.now).num_seconds().div_euclid(60);
    if minutes < 60 {
        format!("in {minutes} min")
    } else if minutes < 6 * 60 {
        format!("in {} h {:02}", minutes / 60, minutes % 60)
    } else {
        clock.local(start).format("%H:%M").to_string()
    }
}

/// A day's heading: Today, Tomorrow, else "Friday 30 October".
pub fn day_label(day: NaiveDate, clock: &Clock) -> String {
    let today = clock.today();
    if day == today {
        "Today".into()
    } else if Some(day) == today.succ_opt() {
        "Tomorrow".into()
    } else {
        day.format("%A %d %B").to_string()
    }
}

/// A short day label for the week view's columns: "Today", "Thu 29".
pub fn short_day(day: NaiveDate, clock: &Clock) -> String {
    if day == clock.today() { "Today".into() } else { format!("{} {}", day.format("%a"), day.day()) }
}

// ---------------------------------------------------------------------------------------------------------------------
// Briefs

pub fn briefs_dir() -> PathBuf {
    files::cache().join("meeting-briefs")
}

/// The brief's file, its name the id with anything unusual made an underscore, as meeting-brief.sh names it.
pub fn brief_path(id: &str, suffix: &str) -> PathBuf {
    let name: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    briefs_dir().join(format!("{name}{suffix}"))
}

pub fn read_brief(id: &str) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(brief_path(id, ".md")).ok()?;
    Some(text.lines().map(|l| l.trim_end().to_string()).collect())
}

pub fn is_preparing(id: &str) -> bool {
    brief_path(id, ".pending").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(at: &str) -> Clock {
        let now = DateTime::parse_from_rfc3339(at).unwrap();
        Clock { now: now.with_timezone(&Utc), zone: Some(*now.offset()) }
    }

    #[test]
    fn words_times_as_meetings_py() {
        let c = clock("2026-10-28T10:06:30+01:00");
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        assert_eq!(until_text(at("2026-10-28T10:15:00+01:00"), &c), "in 8 min");
        assert_eq!(until_text(at("2026-10-28T11:15:00+01:00"), &c), "in 1 h 08");
        assert_eq!(until_text(at("2026-10-28T18:00:00+01:00"), &c), "18:00");
        assert_eq!(day_label(NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(), &c), "Friday 30 October");
        assert_eq!(day_label(NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(), &c), "Tomorrow");
    }
}
