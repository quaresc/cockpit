//! What the hub remembers of each agent across restarts: since when its status holds (done and idle count as one:
//! looking at a finished agent turns done into idle, which is not the agent doing anything), whether it was seen in
//! that status, and its snooze. One JSON file in the cache.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, NaiveTime, TimeZone, Timelike};
use cockpit_core::files;
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snooze {
    /// ms; None: until its status changes.
    pub until: Option<u64>,
    /// The status class it was snoozed in.
    pub class: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub class: String,
    pub since: u64,
    /// The `since` it was seen at: seen while it equals `since`.
    pub seen: Option<u64>,
    pub snooze: Option<Snooze>,
    /// ms it was last listed, to forget gone ones.
    pub last: u64,
}

impl Entry {
    pub fn seen(&self) -> bool {
        self.seen == Some(self.since)
    }
}

/// done and idle are one class: resting.
pub fn class(status: &str) -> &str {
    match status {
        "done" | "idle" => "rest",
        other => other,
    }
}

#[derive(Default)]
pub struct State {
    pub agents: BTreeMap<String, Entry>,
}

const FORGET_MS: u64 = 7 * 86_400_000;

pub fn path() -> PathBuf {
    files::cache().join("agents/state.json")
}

impl State {
    pub fn load(path: &Path) -> State {
        let v = files::load(path).unwrap_or(Value::Null);
        let agents = v["agents"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, e)| {
                let snooze = e["snooze"].is_object().then(|| Snooze { until: e["snooze"]["until"].as_u64(), class: e["snooze"]["class"].as_str().unwrap_or_default().to_string() });
                let entry = Entry {
                    class: e["class"].as_str().unwrap_or_default().to_string(),
                    since: e["since"].as_u64().unwrap_or(0),
                    seen: e["seen"].as_u64(),
                    snooze,
                    last: e["last"].as_u64().unwrap_or(0),
                };
                (k.clone(), entry)
            })
            .collect();
        State { agents }
    }

    pub fn save(&self, path: &Path) {
        let agents: serde_json::Map<String, Value> = self
            .agents
            .iter()
            .map(|(k, e)| {
                let snooze = e.snooze.as_ref().map(|s| json!({"until": s.until, "class": s.class}));
                (k.clone(), json!({"class": e.class, "since": e.since, "seen": e.seen, "snooze": snooze, "last": e.last}))
            })
            .collect();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = files::write(path, &json!({"agents": agents}));
    }

    /// Takes in a listed agent's status at `now`: a new class starts a new `since` (the source's own when it knows),
    /// wakes a snooze that waited for it, and done turning idle counts as seen. True when anything changed.
    pub fn observe(&mut self, handle: &str, status: &str, hint: Option<u64>, now: u64) -> bool {
        let c = class(status).to_string();
        let e = self.agents.entry(handle.to_string()).or_insert_with(|| Entry { class: String::new(), since: hint.unwrap_or(now), ..Default::default() });
        e.last = now;
        let mut changed = false;
        if e.class != c {
            let before = std::mem::replace(&mut e.class, c.clone());
            if !before.is_empty() {
                e.since = hint.unwrap_or(now);
            }
            if let Some(s) = &e.snooze {
                // A snooze till a change wakes on any; a timed one when the agent comes to need you.
                if s.until.is_none() || (c == "blocked" && s.class != "blocked") {
                    e.snooze = None;
                }
            }
            changed = true;
        }
        if e.snooze.as_ref().and_then(|s| s.until).is_some_and(|u| u <= now) {
            e.snooze = None;
            changed = true;
        }
        changed
    }

    /// The person looked at a done agent (Herdr made it idle) or asked to mark it.
    pub fn mark_seen(&mut self, handle: &str) -> bool {
        let Some(e) = self.agents.get_mut(handle) else { return false };
        let was = e.seen();
        e.seen = Some(e.since);
        !was
    }

    pub fn snooze(&mut self, handle: &str, until: Option<Option<u64>>) {
        let Some(e) = self.agents.get_mut(handle) else { return };
        e.snooze = until.map(|until| Snooze { until, class: e.class.clone() });
    }

    /// Forgets agents not listed for a week.
    pub fn prune(&mut self, now: u64) {
        self.agents.retain(|_, e| now.saturating_sub(e.last) < FORGET_MS);
    }
}

/// When a snooze asked as `1h`, `30m`, `18:00`, `tomorrow`, `done` or ms ends: Some(None) till a change, None when it
/// cannot tell.
pub fn parse_until(word: &Value, now: u64) -> Option<Option<u64>> {
    if let Some(ms) = word.as_u64() {
        return Some(Some(ms));
    }
    let w = word.as_str()?.trim().to_lowercase();
    let local = Local.timestamp_millis_opt(now as i64).single()?;
    let at = |day: chrono::DateTime<Local>, h: u32, m: u32| -> Option<u64> {
        let t = NaiveTime::from_hms_opt(h, m, 0)?;
        Some(day.with_time(t).single()?.timestamp_millis() as u64)
    };
    match w.as_str() {
        "done" | "change" | "till done" => return Some(None),
        "tomorrow" => return Some(at(local + chrono::Duration::days(1), 9, 0)),
        "monday" => {
            let ahead = (7 - local.weekday().num_days_from_monday()) as i64;
            return Some(at(local + chrono::Duration::days(ahead), 9, 0));
        }
        _ => {}
    }
    if let Some(n) = w.strip_suffix('h').and_then(|n| n.parse::<f64>().ok()) {
        return Some(Some(now + (n * 3_600_000.0) as u64));
    }
    if let Some(n) = w.strip_suffix('m').and_then(|n| n.parse::<f64>().ok()) {
        return Some(Some(now + (n * 60_000.0) as u64));
    }
    let (h, m) = w.split_once(':')?;
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    let today = at(local, h, m)?;
    Some(Some(if today > now { today } else { at(local + chrono::Duration::days(1), h, m)? }))
}

/// How a snooze reads: "until 18:00", "until Tue 09:00", "till it changes".
pub fn until_label(until: Option<u64>, now: u64) -> String {
    let Some(ms) = until else { return "till it changes".into() };
    let (Some(t), Some(n)) = (Local.timestamp_millis_opt(ms as i64).single(), Local.timestamp_millis_opt(now as i64).single()) else { return "snoozed".into() };
    if t.date_naive() == n.date_naive() {
        format!("until {:02}:{:02}", t.hour(), t.minute())
    } else {
        format!("until {} {:02}:{:02}", t.format("%a"), t.hour(), t.minute())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_change_starts_a_new_since_and_done_turning_idle_does_not() {
        let mut s = State::default();
        assert!(s.observe("app:t1", "working", None, 1000));
        assert!(s.observe("app:t1", "done", None, 2000));
        assert_eq!(s.agents["app:t1"].since, 2000);
        assert!(s.mark_seen("app:t1"));
        assert!(!s.observe("app:t1", "idle", None, 3000));
        assert!(s.agents["app:t1"].seen());
        s.observe("app:t1", "blocked", None, 4000);
        assert!(!s.agents["app:t1"].seen());
    }

    #[test]
    fn a_snooze_wakes_on_time_on_a_change_or_when_it_needs_you() {
        let mut s = State::default();
        s.observe("t", "working", None, 0);
        s.snooze("t", Some(Some(10_000)));
        s.observe("t", "done", None, 5000);
        assert!(s.agents["t"].snooze.is_some());
        s.observe("t", "done", None, 11_000);
        assert!(s.agents["t"].snooze.is_none());
        s.snooze("t", Some(None));
        s.observe("t", "working", None, 12_000);
        assert!(s.agents["t"].snooze.is_none());
        s.snooze("t", Some(Some(99_999)));
        s.observe("t", "blocked", None, 13_000);
        assert!(s.agents["t"].snooze.is_none());
    }

    #[test]
    fn snooze_words_read_as_times() {
        let now = Local.with_ymd_and_hms(2026, 11, 2, 15, 0, 0).unwrap().timestamp_millis() as u64;
        assert_eq!(parse_until(&json!("1h"), now), Some(Some(now + 3_600_000)));
        assert_eq!(parse_until(&json!("done"), now), Some(None));
        let six = parse_until(&json!("18:00"), now).unwrap().unwrap();
        assert_eq!(until_label(Some(six), now), "until 18:00");
        let nine = parse_until(&json!("09:00"), now).unwrap().unwrap();
        assert!(until_label(Some(nine), now).starts_with("until Tue"));
        assert_eq!(parse_until(&json!("soon"), now), None);
    }

    #[test]
    fn state_survives_a_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("agents/state.json");
        let mut s = State::default();
        s.observe("app:t2", "blocked", None, 7);
        s.snooze("app:t2", Some(None));
        s.save(&p);
        assert_eq!(State::load(&p).agents["app:t2"], s.agents["app:t2"]);
    }
}
