//! What the inbox reads beside its caches: Claude's spend per ticket, the live sessions' heartbeats and herdr's spaces,
//! the last two listed again at most every 10 s since every rebuild asks.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cockpit_core::files;
use regex::Regex;
use serde_json::Value;

use crate::facts::{Config, list, s};

const FRESH: Duration = Duration::from_secs(10);

/// What each ticket cost, in US dollars, summed over the sessions that worked on it.
pub fn ticket_costs() -> HashMap<String, f64> {
    let mut total = HashMap::new();
    for path in json_files("costs") {
        if let Some(Value::Object(spent)) = files::load(&path) {
            for (ticket, usd) in spent {
                *total.entry(ticket).or_insert(0.0) += usd.as_f64().unwrap_or(0.0);
            }
        }
    }
    total
}

fn json_files(dir: &str) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(files::cache().join(dir))
        .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect())
        .unwrap_or_default();
    paths.sort();
    paths
}

#[derive(Default)]
pub struct Local {
    spaces: Option<(Instant, HashMap<String, Value>)>,
    beats: Option<(Instant, Vec<Value>)>,
}

impl Local {
    /// herdr's spaces by id, each label stripped of its "[N] " prefix.
    pub fn spaces(&mut self) -> &HashMap<String, Value> {
        if self.spaces.as_ref().is_none_or(|(at, _)| at.elapsed() > FRESH) {
            let prefix = Regex::new(r"^\[\d+\]\s*").expect("a valid regex");
            let listed = files::herdr(&["workspace", "list"]).unwrap_or(Value::Null);
            let by_id = list(&listed["result"], "workspaces")
                .iter()
                .map(|w| {
                    let mut w = w.clone();
                    w["label"] = Value::from(prefix.replace(s(&w, "label"), "").into_owned());
                    (s(&w, "workspace_id").to_string(), w)
                })
                .collect();
            self.spaces = Some((Instant::now(), by_id));
        }
        &self.spaces.as_ref().expect("just listed").1
    }

    fn heartbeats(&mut self) -> Vec<Value> {
        if self.beats.as_ref().is_none_or(|(at, _)| at.elapsed() > FRESH) {
            let beats = json_files("sessions").iter().map(|p| files::load(p).unwrap_or(Value::Null)).collect();
            self.beats = Some((Instant::now(), beats));
        }
        self.beats.as_ref().expect("just read").1.clone()
    }

    /// Live sessions whose branch has merged with no PR of it still open: their space can close, or their tab
    /// ("tab:<id>") when they sit in a pinned space.
    pub fn merged_sessions(&mut self, merged: &HashSet<(String, String)>, mine: &[Value], config: &Config) -> Vec<(String, (String, String))> {
        let still_open: HashSet<(String, String)> = mine.iter().map(|pr| (s(pr, "repo").into(), s(pr, "branch").into())).collect();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0);
        let mut out: Vec<(String, (String, String))> = Vec::new();
        for beat in self.heartbeats() {
            let key = (s(&beat, "repo").to_string(), s(&beat, "branch").to_string());
            let is_live = now - beat["at"].as_f64().unwrap_or(0.0) < 3.0 * 60_000.0;
            if !(is_live && !s(&beat, "space").is_empty() && merged.contains(&key) && !still_open.contains(&key)) {
                continue;
            }
            let label = self.spaces().get(s(&beat, "space")).map(|w| s(w, "label").to_string()).unwrap_or_default();
            let space = if !config.is_pinned(&label) {
                s(&beat, "space").to_string()
            } else if !s(&beat, "tab").is_empty() {
                format!("tab:{}", s(&beat, "tab"))
            } else {
                continue;
            };
            match out.iter_mut().find(|(known, _)| *known == space) {
                Some(entry) => entry.1 = key,
                None => out.push((space, key)),
            }
        }
        out
    }
}
