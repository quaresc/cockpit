//! What changed between two rebuilds, for the rows to show it: CI turning red or green, a PR approved, a new review
//! request or PR, a session's agent finishing work on a ticket's branch. Each rebuild takes a snapshot of the states
//! that matter and diffs it with the one before.

use std::collections::{BTreeMap, HashMap, HashSet};

use cockpit_core::anim;
use serde_json::Value;

use crate::facts::{has_ticket, is_failing, list, s};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Broken,
    Good,
    New,
    Done,
}

impl Change {
    pub fn effect(self) -> anim::Kind {
        match self {
            Change::Broken => anim::Kind::BROKEN,
            Change::Good => anim::Kind::GOOD,
            Change::New => anim::Kind::NEW,
            Change::Done => anim::Kind::DONE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ci {
    Red,
    Green,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Agent {
    Working,
    Idle,
    Other,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// My PRs by URL: their CI, and whether they are approved.
    ci: HashMap<String, Ci>,
    approved: HashSet<String>,
    /// Review requests by URL.
    requests: HashSet<String>,
    /// Tickets by id: what the agents on their branches are doing.
    agents: HashMap<String, Agent>,
}

/// A PR's CI as the eye reads it: red when GitHub or its latest finished run says so, green when either says it passed.
fn ci_state(pr: &Value, run: Option<&Value>) -> Ci {
    let finished = run.filter(|r| !r["isRunning"].as_bool().unwrap_or(false));
    if is_failing(pr) || finished.is_some_and(|r| s(r, "outcome") == "failed") {
        Ci::Red
    } else if s(pr, "ci") == "SUCCESS" || finished.is_some_and(|r| s(r, "outcome") == "succeeded") {
        Ci::Green
    } else {
        Ci::Other
    }
}

fn agent_state(status: &str) -> Agent {
    match status {
        "working" => Agent::Working,
        "idle" | "done" => Agent::Idle,
        _ => Agent::Other,
    }
}

/// The states that matter, from the inbox cache, the CI runs and the live sessions as (repo, branch, agent status).
pub fn snapshot(data: &Value, ci: &Value, agents: &[(String, String, String)]) -> Snapshot {
    let runs: HashMap<(&str, &str), &Value> = list(ci, "runs").iter().map(|r| ((s(r, "repo"), s(r, "branch")), r)).collect();
    let mut snap = Snapshot::default();
    for pr in list(data, "mine") {
        let url = s(pr, "url").to_string();
        snap.ci.insert(url.clone(), ci_state(pr, runs.get(&(s(pr, "repo"), s(pr, "branch"))).copied()));
        if s(pr, "review") == "APPROVED" {
            snap.approved.insert(url);
        }
    }
    snap.requests = list(data, "direct").iter().chain(list(data, "team")).map(|pr| s(pr, "url").to_string()).collect();
    for ticket in list(data, "tickets") {
        let id = s(ticket, "id");
        let states: Vec<Agent> = agents.iter().filter(|(_, branch, _)| has_ticket(branch, id)).map(|(_, _, status)| agent_state(status)).collect();
        let state = if states.contains(&Agent::Working) {
            Agent::Working
        } else if states.contains(&Agent::Idle) {
            Agent::Idle
        } else {
            Agent::Other
        };
        snap.agents.insert(id.to_string(), state);
    }
    snap
}

/// What changed from `before` to `after`, one change per key (a PR URL or a ticket id), sorted by key.
pub fn diff(before: &Snapshot, after: &Snapshot) -> Vec<(String, Change)> {
    let mut out: BTreeMap<String, Change> = BTreeMap::new();
    for (url, now) in &after.ci {
        match before.ci.get(url) {
            None => {
                out.insert(url.clone(), Change::New);
            }
            Some(was) if *was != Ci::Red && *now == Ci::Red => {
                out.insert(url.clone(), Change::Broken);
            }
            Some(was) if *was != Ci::Green && *now == Ci::Green => {
                out.insert(url.clone(), Change::Good);
            }
            _ => {}
        }
    }
    for url in after.approved.difference(&before.approved).filter(|u| before.ci.contains_key(*u)) {
        out.entry(url.clone()).or_insert(Change::Good);
    }
    for url in after.requests.difference(&before.requests) {
        out.entry(url.clone()).or_insert(Change::New);
    }
    for (id, now) in &after.agents {
        if before.agents.get(id) == Some(&Agent::Working) && *now == Agent::Idle {
            out.entry(id.clone()).or_insert(Change::Done);
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pr(url: &str, ci: &str, review: &str) -> Value {
        json!({"url": url, "repo": "orders", "branch": format!("eng-1-{url}"), "ci": ci, "review": review})
    }

    fn snap(mine: Vec<Value>, requests: &[&str], runs: Vec<Value>, agents: &[(&str, &str)]) -> Snapshot {
        let direct: Vec<Value> = requests.iter().map(|u| json!({"url": u})).collect();
        let data = json!({"mine": mine, "direct": direct, "team": [], "tickets": [{"id": "ENG-1"}, {"id": "ENG-2"}]});
        let agents: Vec<(String, String, String)> = agents.iter().map(|(b, st)| ("orders".to_string(), b.to_string(), st.to_string())).collect();
        snapshot(&data, &json!({"runs": runs}), &agents)
    }

    #[test]
    fn nothing_changes_nothing_plays() {
        let a = snap(vec![pr("a", "FAILURE", "")], &["r"], vec![], &[("eng-1-x", "working")]);
        assert!(diff(&a, &a.clone()).is_empty());
    }

    #[test]
    fn ci_turning_red_is_broken_and_green_is_good() {
        let before = snap(vec![pr("a", "PENDING", ""), pr("b", "FAILURE", "")], &[], vec![], &[]);
        let after = snap(vec![pr("a", "FAILURE", ""), pr("b", "SUCCESS", "")], &[], vec![], &[]);
        assert_eq!(diff(&before, &after), vec![("a".into(), Change::Broken), ("b".into(), Change::Good)]);
    }

    #[test]
    fn a_finished_run_counts_before_github_catches_up() {
        let running = json!({"repo": "orders", "branch": "eng-1-a", "isRunning": true, "outcome": ""});
        let failed = json!({"repo": "orders", "branch": "eng-1-a", "isRunning": false, "outcome": "failed"});
        let before = snap(vec![pr("a", "PENDING", "")], &[], vec![running], &[]);
        let after = snap(vec![pr("a", "PENDING", "")], &[], vec![failed], &[]);
        assert_eq!(diff(&before, &after), vec![("a".into(), Change::Broken)]);
    }

    #[test]
    fn an_approval_is_good_once() {
        let before = snap(vec![pr("a", "SUCCESS", "REVIEW_REQUIRED")], &[], vec![], &[]);
        let after = snap(vec![pr("a", "SUCCESS", "APPROVED")], &[], vec![], &[]);
        assert_eq!(diff(&before, &after), vec![("a".into(), Change::Good)]);
        assert!(diff(&after, &after).is_empty());
    }

    #[test]
    fn new_requests_and_prs_sweep_in_and_gone_ones_do_nothing() {
        let before = snap(vec![pr("a", "", "")], &["r1"], vec![], &[]);
        let after = snap(vec![pr("a", "", ""), pr("b", "", "")], &["r2"], vec![], &[]);
        assert_eq!(diff(&before, &after), vec![("b".into(), Change::New), ("r2".into(), Change::New)]);
    }

    #[test]
    fn an_agent_going_idle_on_a_tickets_branch_is_done() {
        let before = snap(vec![], &[], vec![], &[("me/eng-1-login", "working"), ("me/eng-2-x", "idle")]);
        let after = snap(vec![], &[], vec![], &[("me/eng-1-login", "idle"), ("me/eng-2-x", "working")]);
        assert_eq!(diff(&before, &after), vec![("ENG-1".into(), Change::Done)]);
        let gone = snap(vec![], &[], vec![], &[]);
        assert!(diff(&before, &gone).is_empty(), "a session that went away did not finish anything");
    }

    #[test]
    fn red_staying_red_and_blocked_agents_do_not_play() {
        let before = snap(vec![pr("a", "FAILURE", "")], &[], vec![], &[("eng-1-x", "working")]);
        let after = snap(vec![pr("a", "ERROR", "")], &[], vec![], &[("eng-1-x", "blocked")]);
        assert!(diff(&before, &after).is_empty());
    }
}
