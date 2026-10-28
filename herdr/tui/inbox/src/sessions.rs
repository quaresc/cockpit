//! What the inbox reads about the live Claude sessions: their heartbeats, herdr's spaces and each agent's status, listed
//! again at most every 10 s since every rebuild asks.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cockpit_core::files;
use regex::Regex;
use serde_json::Value;

use crate::facts::{Config, list, s};
use crate::inputs::json_files;

const FRESH: Duration = Duration::from_secs(10);
/// A heartbeat older than this belongs to a session that is gone.
const LIVE_MS: f64 = 3.0 * 60_000.0;

#[derive(Default)]
pub struct Local {
    spaces: Option<(Instant, HashMap<String, Value>)>,
    beats: Option<(Instant, Vec<Value>)>,
    panes: Option<(Instant, HashMap<String, String>)>,
    /// Set by `fixed`: what was given stays, nothing is listed again.
    fixed: bool,
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

impl Local {
    /// Sessions and spaces given once, for tests: herdr and the cache are never asked.
    pub fn fixed(spaces: HashMap<String, Value>, beats: Vec<Value>) -> Self {
        let at = Instant::now();
        Self { spaces: Some((at, spaces)), beats: Some((at, beats)), panes: Some((at, HashMap::new())), fixed: true }
    }

    fn stale<T>(&self, slot: &Option<(Instant, T)>) -> bool {
        !self.fixed && slot.as_ref().is_none_or(|(at, _)| at.elapsed() > FRESH)
    }

    /// herdr's spaces by id, each label stripped of its "[N] " prefix.
    pub fn spaces(&mut self) -> &HashMap<String, Value> {
        if self.stale(&self.spaces) {
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

    /// The heartbeats of the sessions still running.
    fn heartbeats(&mut self) -> Vec<Value> {
        if self.stale(&self.beats) {
            let beats = json_files("sessions").iter().map(|p| files::load(p).unwrap_or(Value::Null)).collect();
            self.beats = Some((Instant::now(), beats));
        }
        let fixed = self.fixed;
        let beats = &self.beats.as_ref().expect("just read").1;
        beats.iter().filter(|b| fixed || now_ms() - b["at"].as_f64().unwrap_or(0.0) < LIVE_MS).cloned().collect()
    }

    /// Each pane's agent status as herdr reports it: working, idle, blocked or unknown.
    fn panes(&mut self) -> &HashMap<String, String> {
        if self.stale(&self.panes) {
            let listed = files::herdr(&["pane", "list"]).unwrap_or(Value::Null);
            let by_id = list(&listed["result"], "panes").iter().map(|p| (s(p, "pane_id").to_string(), s(p, "agent_status").to_string())).collect();
            self.panes = Some((Instant::now(), by_id));
        }
        &self.panes.as_ref().expect("just listed").1
    }

    /// The live sessions on a branch as (repo, branch, agent status): the heartbeat's own status when it carries one,
    /// else herdr's for its pane, which is only asked when some session works on a branch.
    pub fn agents(&mut self) -> Vec<(String, String, String)> {
        let beats: Vec<Value> = self.heartbeats().into_iter().filter(|b| !s(b, "branch").is_empty()).collect();
        let mut out = Vec::new();
        for beat in beats {
            let status = match s(&beat, "status") {
                "" => self.panes().get(s(&beat, "pane")).cloned().unwrap_or_default(),
                own => own.to_string(),
            };
            out.push((s(&beat, "repo").to_string(), s(&beat, "branch").to_string(), status));
        }
        out
    }

    /// Live sessions whose branch has merged with no PR of it still open: their space can close, or their tab
    /// ("tab:<id>") when they sit in a pinned space.
    pub fn merged_sessions(&mut self, merged: &HashSet<(String, String)>, mine: &[Value], config: &Config) -> Vec<(String, (String, String))> {
        let still_open: HashSet<(String, String)> = mine.iter().map(|pr| (s(pr, "repo").into(), s(pr, "branch").into())).collect();
        let mut out: Vec<(String, (String, String))> = Vec::new();
        for beat in self.heartbeats() {
            let key = (s(&beat, "repo").to_string(), s(&beat, "branch").to_string());
            if s(&beat, "space").is_empty() || !merged.contains(&key) || still_open.contains(&key) {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_merged_session_in_a_pinned_space_closes_its_tab_only() {
        let now = now_ms();
        let beats = vec![
            json!({"at": now, "space": "w2", "tab": "w2:t3", "repo": "api", "branch": "b"}),
            json!({"at": now, "space": "w5", "tab": "w5:t1", "repo": "api", "branch": "c"}),
        ];
        let spaces = HashMap::from([("w2".to_string(), json!({"label": "\u{f0e7}  actions"})), ("w5".to_string(), json!({"label": "eng-4"}))]);
        let config = Config::from(json!({"spaces": ["inbox", "actions"]}));
        let merged = HashSet::from([("api".to_string(), "b".to_string()), ("api".to_string(), "c".to_string())]);
        let closing = Local::fixed(spaces, beats).merged_sessions(&merged, &[], &config);
        let pair = |space: &str, branch: &str| (space.to_string(), ("api".to_string(), branch.to_string()));
        assert_eq!(closing, vec![pair("tab:w2:t3", "b"), pair("w5", "c")]);
    }

    #[test]
    fn a_branch_whose_pr_is_still_open_keeps_its_session() {
        let beats = vec![json!({"at": now_ms(), "space": "w5", "tab": "w5:t1", "repo": "api", "branch": "c"})];
        let spaces = HashMap::from([("w5".to_string(), json!({"label": "eng-4"}))]);
        let merged = HashSet::from([("api".to_string(), "c".to_string())]);
        let mine = [json!({"repo": "api", "branch": "c"})];
        assert!(Local::fixed(spaces, beats).merged_sessions(&merged, &mine, &Config::from(json!({}))).is_empty());
    }
}
