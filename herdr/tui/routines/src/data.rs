//! The `routines` document the hub writes, read into the routines, what runs and waits, and each one's past runs; the
//! clock that says "in 2 h" and "12 min ago".

use std::collections::HashMap;

use chrono::{DateTime, FixedOffset, Local, NaiveDateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Routine {
    pub slug: String,
    pub name: String,
    pub path: String,
    pub when: String,
    pub when_text: String,
    /// manual, every, schedule, event or broken.
    pub trigger: String,
    pub error: Option<String>,
    /// claude or shell.
    pub run: String,
    pub cwd: String,
    pub model: String,
    pub tools: Vec<String>,
    pub mode: String,
    /// Dollars a week, and what its runs of the last seven days cost, as Claude estimates them.
    pub budget: Option<f64>,
    pub spent_week: f64,
    pub per_run: Option<f64>,
    /// Its week's budget is spent: runs wait.
    pub over: bool,
    pub paused: bool,
    pub prompt: String,
    pub next: Option<u64>,
    pub notify: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Running {
    pub id: String,
    pub started: u64,
    pub why: String,
    pub session: String,
    pub stopping: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Run {
    pub id: String,
    pub started: u64,
    pub ended: u64,
    pub ok: bool,
    pub code: Option<i64>,
    pub why: String,
    pub session: Option<String>,
    pub cost: Option<f64>,
    pub summary: String,
    pub stopped: bool,
    pub timed_out: bool,
}

impl Run {
    pub fn session(&self) -> Option<&str> {
        self.session.as_deref().filter(|s| !s.is_empty())
    }

    pub fn took(&self) -> String {
        duration(self.ended.saturating_sub(self.started))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Queued {
    slug: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Raw {
    version: u64,
    at: Option<u64>,
    routines: Vec<Routine>,
    running: HashMap<String, Running>,
    queued: Vec<Queued>,
    runs: HashMap<String, Vec<Run>>,
}

/// How a routine stands, for its glyph and colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    Waiting,
    Broken,
    Over,
    Paused,
    Failed,
    Ok,
    Never,
}

#[derive(Clone, Debug, Default)]
pub struct Doc {
    pub at: Option<u64>,
    pub routines: Vec<Routine>,
    pub running: HashMap<String, Running>,
    pub queued: Vec<String>,
    pub runs: HashMap<String, Vec<Run>>,
}

impl Doc {
    /// The document as the hub wrote it; None for another shape.
    pub fn parse(value: &Value) -> Option<Doc> {
        let raw: Raw = serde_json::from_value(value.clone()).ok()?;
        (raw.version == 1).then(|| Doc {
            at: raw.at,
            routines: raw.routines,
            running: raw.running,
            queued: raw.queued.into_iter().map(|q| q.slug).collect(),
            runs: raw.runs,
        })
    }

    pub fn routine(&self, slug: &str) -> Option<&Routine> {
        self.routines.iter().find(|r| r.slug == slug)
    }

    pub fn runs(&self, slug: &str) -> &[Run] {
        self.runs.get(slug).map(Vec::as_slice).unwrap_or_default()
    }

    pub fn last(&self, slug: &str) -> Option<&Run> {
        self.runs(slug).first()
    }

    pub fn run(&self, slug: &str, id: &str) -> Option<&Run> {
        self.runs(slug).iter().find(|r| r.id == id)
    }

    pub fn state(&self, r: &Routine) -> State {
        if self.running.contains_key(&r.slug) {
            State::Running
        } else if self.queued.contains(&r.slug) {
            State::Waiting
        } else if r.error.is_some() {
            State::Broken
        } else if r.paused {
            State::Paused
        } else if r.over {
            State::Over
        } else {
            match self.last(&r.slug) {
                Some(run) if run.ok => State::Ok,
                Some(_) => State::Failed,
                None => State::Never,
            }
        }
    }
}

/// "38 s", "4 min", "1 h 05".
pub fn duration(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        _ => format!("{} h {:02}", s / 3600, s / 60 % 60),
    }
}

/// "$0.12", "$1.40".
pub fn dollars(usd: f64) -> String {
    format!("${usd:.2}")
}

/// A cost Claude estimated, not billed on a Claude plan: "~$0.07", "~$0.004".
pub fn estimate(usd: f64) -> String {
    if usd > 0.0 && usd < 0.01 { format!("~${usd:.3}") } else { format!("~${usd:.2}") }
}

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub now: u64,
    pub zone: Option<FixedOffset>,
}

impl Clock {
    pub fn system() -> Self {
        Self { now: Utc::now().timestamp_millis() as u64, zone: None }
    }

    pub fn local(&self, ms: u64) -> NaiveDateTime {
        let at = DateTime::<Utc>::from_timestamp_millis(ms as i64).unwrap_or_default();
        match self.zone {
            Some(zone) => at.with_timezone(&zone).naive_local(),
            None => at.with_timezone(&Local).naive_local(),
        }
    }

    /// "09:00" today, "Thu 09:00" this week, "3 Nov 09:00" later.
    pub fn when(&self, ms: u64) -> String {
        let (at, now) = (self.local(ms), self.local(self.now));
        let days = (at.date() - now.date()).num_days();
        if days == 0 {
            at.format("%H:%M").to_string()
        } else if (-6..=6).contains(&days) {
            at.format("%a %H:%M").to_string()
        } else {
            at.format("%-d %b %H:%M").to_string()
        }
    }

    /// "now", "12 min ago", "2 h ago", "3 d ago".
    pub fn ago(&self, ms: u64) -> String {
        let mins = self.now.saturating_sub(ms) / 60_000;
        match mins {
            0 => "now".into(),
            1..60 => format!("{mins} min ago"),
            60..2880 => format!("{} h ago", mins / 60),
            _ => format!("{} d ago", mins / 1440),
        }
    }

    /// "in 4 min", "in 2 h", "09:00 tomorrow" as "Fri 09:00".
    pub fn until(&self, ms: u64) -> String {
        let mins = ms.saturating_sub(self.now).div_ceil(60_000);
        match mins {
            0 => "now".into(),
            1..60 => format!("in {mins} min"),
            _ => self.when(ms),
        }
    }

    pub fn second(&self) -> u64 {
        self.now / 1000
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_document_reads_with_each_routines_state() {
        let doc = Doc::parse(&json!({"version": 1, "routines": [
            {"slug": "a", "name": "A"}, {"slug": "b", "name": "B"}, {"slug": "c", "name": "C", "paused": true},
            {"slug": "d", "name": "D", "error": "when?"}, {"slug": "e", "name": "E"}, {"slug": "f", "name": "F"}],
            "running": {"a": {"id": "1", "started": 5}}, "queued": [{"slug": "b"}],
            "runs": {"e": [{"id": "2", "ok": false}], "f": [{"id": "3", "ok": true, "started": 1000, "ended": 39_000}]}}))
        .unwrap();
        let states: Vec<State> = doc.routines.iter().map(|r| doc.state(r)).collect();
        assert_eq!(states, [State::Running, State::Waiting, State::Paused, State::Broken, State::Failed, State::Ok]);
        assert_eq!(doc.last("f").unwrap().took(), "38 s");
        assert!(Doc::parse(&json!({"version": 2})).is_none());
    }

    #[test]
    fn times_read_short() {
        let now = DateTime::parse_from_rfc3339("2026-10-29T10:00:00+01:00").unwrap();
        let clock = Clock { now: now.timestamp_millis() as u64, zone: Some(*now.offset()) };
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis() as u64;
        assert_eq!(clock.until(at("2026-10-29T10:03:10+01:00")), "in 4 min");
        assert_eq!(clock.until(at("2026-10-29T17:30:00+01:00")), "17:30");
        assert_eq!(clock.until(at("2026-10-30T09:00:00+01:00")), "Fri 09:00");
        assert_eq!(clock.until(at("2026-11-12T09:00:00+01:00")), "12 Nov 09:00");
        assert_eq!(clock.ago(at("2026-10-29T08:00:00+01:00")), "2 h ago");
        assert_eq!((duration(4 * 60_000 + 5000), duration(3_900_000), dollars(0.123)), ("4 min".into(), "1 h 05".into(), "$0.12".into()));
    }
}
