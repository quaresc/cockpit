//! What one rebuild reads: the documents the hub pushes, the muted repos and Claude's spend per ticket, read once and
//! handed to the model, so the model itself never touches a file and tests can feed it fixtures.

use std::collections::HashMap;
use std::path::PathBuf;

use cockpit_agents::ledger;
use cockpit_core::files;
use cockpit_core::hub::Docs;
use serde_json::{Value, json};

#[derive(Clone, Debug, Default)]
pub struct Inputs {
    /// inbox.json; None before the mod first wrote it.
    pub data: Option<Value>,
    pub ci: Value,
    pub brief: Option<Value>,
    pub muted: Vec<String>,
    /// What each ticket cost, in US dollars, summed over the sessions that worked on it.
    pub costs: HashMap<String, f64>,
    /// The linked worktrees of the repos: {path, repo, branch, changes}.
    pub worktrees: Vec<Value>,
    /// Seconds agents spent on each ticket this week, from the agent-time ledger.
    pub secs: HashMap<String, u64>,
    /// The hub's routines document, for the runs Today says need a look.
    pub routines: Option<Value>,
    /// The doctor's last report, read while there is no inbox yet: what is left of getting started.
    pub started: Value,
    /// Start work's runs the hub keeps, for the ones still running or failed.
    pub starts: Vec<Value>,
}

/// The documents the inbox follows through the hub.
pub const DOCS: [&str; 4] = ["inbox", "ci", "brief", "routines"];

impl Inputs {
    /// The TUI's inputs: the worktrees as last scanned, a new scan running off the keys' thread when due.
    pub fn read(docs: &Docs) -> Self {
        let routines = docs.get("routines").cloned();
        let data = docs.get("inbox").cloned();
        let worktrees = crate::worktrees::scan_later(&files::config(), merged(data.as_ref()));
        Self { routines, ..Self::with(data, docs.get("ci").cloned(), docs.get("brief").cloned(), worktrees) }
    }

    /// The documents given, with what the inbox reads on disk beside them.
    pub fn of(data: Option<Value>, ci: Option<Value>, brief: Option<Value>) -> Self {
        let worktrees = crate::worktrees::scan(&files::config(), merged(data.as_ref()));
        Self::with(data, ci, brief, worktrees)
    }

    fn with(data: Option<Value>, ci: Option<Value>, brief: Option<Value>, worktrees: Vec<Value>) -> Self {
        Self {
            data,
            ci: ci.unwrap_or_else(|| json!({"runs": []})),
            brief,
            muted: muted(),
            costs: ticket_costs(),
            worktrees,
            secs: ticket_week(crate::worktrees::now_ms()),
            routines: None,
            started: cockpit_core::started::report(),
            starts: starts(),
        }
    }
}

/// The code host's merged pull requests, from the inbox document.
fn merged(data: Option<&Value>) -> &[Value] {
    data.and_then(|d| d["merged"].as_array()).map_or(&[], Vec::as_slice)
}

/// Start work's runs as the hub keeps them in start-runs.json.
pub fn starts() -> Vec<Value> {
    files::load(&files::cache().join("start-runs.json")).and_then(|v| v["runs"].as_array().cloned()).unwrap_or_default()
}

/// Seconds agents spent on each ticket since Monday 00:00 local, from the agents' ledger.
pub fn ticket_week(now: u64) -> HashMap<String, u64> {
    let (_, monday) = ledger::day_and_week_start(now);
    let totals = ledger::totals(&ledger::read(&ledger::path(), monday), now);
    totals.tickets.into_iter().map(|(ticket, (_, week))| (ticket.to_uppercase(), week)).collect()
}

fn muted() -> Vec<String> {
    files::load(&files::config_dir().join("muted.json"))
        .and_then(|v| v.as_array().map(|a| a.iter().filter_map(|r| r.as_str().map(str::to_string)).collect()))
        .unwrap_or_default()
}

/// What each ticket cost, as the hub's count estimates it everywhere (`usage::ticket_estimates`).
fn ticket_costs() -> HashMap<String, f64> {
    cockpit_core::usage::ticket_estimates()
}

pub fn json_files(dir: &str) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(files::cache().join(dir))
        .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect())
        .unwrap_or_default();
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_rows_read_the_agents_ledger() {
        crate::test_home::home();
        let path = ledger::path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let now = crate::worktrees::now_ms();
        let lines = [
            json!({"start": now - 3_600_000, "end": now - 1_800_000, "ticket": "ENG-1", "terminal": "app:t1"}),
            json!({"start": now - 2_700_000, "end": now - 900_000, "ticket": "ENG-1", "terminal": "app:t2"}),
            json!({"start": now - 600_000, "end": now, "ticket": "eng-2", "terminal": "app:t3"}),
        ];
        std::fs::write(&path, lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
        let week = ticket_week(now);
        // Stretches that started before Monday count from Monday on: early in a week ENG-1 may hold less.
        assert!(week.get("ENG-1").is_some_and(|s| *s <= 2700));
        assert!(week.get("ENG-2").is_some_and(|s| *s <= 600));
        assert_eq!(week.len(), 2);
    }

    #[test]
    fn ticket_costs_are_the_hub_s_estimates() {
        let home = crate::test_home::home();
        let cache = home.join(".cache/cockpit");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("estimates.json"), r#"{"at": 1, "sessions": {"s-a": 4.0}, "tickets": {"ENG-1": 3.0, "ENG-2": 1.0}}"#).unwrap();
        let got = ticket_costs();
        assert_eq!((got["ENG-1"], got["ENG-2"], got.len()), (3.0, 1.0, 2), "{got:?}");
    }
}
