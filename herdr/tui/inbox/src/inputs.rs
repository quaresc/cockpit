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
}

/// The documents the inbox follows through the hub.
pub const DOCS: [&str; 3] = ["inbox", "ci", "brief"];

impl Inputs {
    pub fn read(docs: &Docs) -> Self {
        Self::of(docs.get("inbox").cloned(), docs.get("ci").cloned(), docs.get("brief").cloned())
    }

    /// The documents given, with what the inbox reads on disk beside them.
    pub fn of(data: Option<Value>, ci: Option<Value>, brief: Option<Value>) -> Self {
        Self {
            data,
            ci: ci.unwrap_or_else(|| json!({"runs": []})),
            brief,
            muted: muted(),
            costs: ticket_costs(),
            worktrees: crate::worktrees::scan(&files::config()),
            secs: ticket_week(crate::worktrees::now_ms()),
        }
    }
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

fn ticket_costs() -> HashMap<String, f64> {
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
    fn ticket_costs_sum_every_session() {
        let costs = crate::test_home::home().join(".cache/cockpit/costs");
        std::fs::create_dir_all(&costs).unwrap();
        std::fs::write(costs.join("a.json"), r#"{"ENG-1": 1.5, "ENG-2": 0.25}"#).unwrap();
        std::fs::write(costs.join("b.json"), r#"{"ENG-1": 2.0}"#).unwrap();
        std::fs::write(costs.join("notes.txt"), r#"{"ENG-1": 100}"#).unwrap();
        assert_eq!(ticket_costs(), HashMap::from([("ENG-1".to_string(), 3.5), ("ENG-2".to_string(), 0.25)]));
    }
}
