//! Whose changes the space reviews: the agents of the hub's agents view that work in a git worktree, the most recently
//! active first, and the one the comments go to.

use cockpit_core::agent;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    /// The handle the review is sent to: a herdr pane id, or an app terminal's.
    pub terminal: String,
    /// Its task, else its agent's name.
    pub name: String,
    /// What its agent is called: Claude Code, Codex.
    pub agent: String,
    pub status: String,
    /// It waits on a question or a permission: what is typed into it would answer that instead.
    pub waiting: bool,
    /// A session cockpit drives itself, which takes the review without a terminal.
    pub driven: bool,
    /// When its state began, in ms.
    pub since: u64,
    /// The worktree's folder, as git names it.
    pub repo: String,
    pub branch: String,
    /// Its space's name: how an agent a review started is found again.
    pub space: String,
    /// It runs in a herdr TUI pane, which the TUI's review space follows; the others only get reviews.
    pub tui: bool,
}

impl Agent {
    /// The worktree's folder name.
    pub fn repo_name(&self) -> &str {
        self.repo.rsplit('/').find(|p| !p.is_empty()).unwrap_or(&self.repo)
    }

    /// How the worktree picker lists it, the worktree and the agent first: "api · Daily quotas · working · me/eng-412".
    pub fn label(&self) -> String {
        let branch = if self.branch.is_empty() { String::new() } else { format!(" · {}", self.branch) };
        format!("{} · {} · {}{branch}", self.repo_name(), self.name, self.status)
    }

    /// How the recipient picker lists it: "Daily quotas · Claude Code, working", "… · in chat, working".
    pub fn recipient(&self) -> String {
        let how = if self.driven { "in chat".to_string() } else { self.agent.clone() };
        format!("{} · {how}, {}", self.name, self.status)
    }
}

/// The agents the view lists that work in a git worktree (`toplevel` names a folder's), the most recently active
/// first: working (active now), then blocked or done by their latest change of state, then idle. Sessions cockpit
/// drives itself are among them: `deliver` hands them the review.
pub fn agents(view: &Value, toplevel: &dyn Fn(&str) -> Option<String>) -> Vec<Agent> {
    let text = |row: &Value, key: &str| row[key].as_str().unwrap_or_default().trim().to_string();
    let mut out: Vec<Agent> = view["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let cwd = row["cwd"].as_str().filter(|c| c.starts_with('/'))?;
            let repo = toplevel(cwd)?;
            let terminal = Some(text(row, "terminal")).filter(|t| !t.is_empty())?;
            let program = text(row, "program");
            let agent = agent::of_program(&program).map(|a| a.name).unwrap_or(if program.is_empty() { "agent".to_string() } else { program });
            let title = Some(text(row, "title")).filter(|t| !t.is_empty());
            let status = Some(text(row, "status")).filter(|s| !s.is_empty()).unwrap_or_else(|| "idle".into());
            Some(Agent {
                terminal,
                name: title.unwrap_or_else(|| agent.clone()),
                agent,
                waiting: status == "blocked" || row["question"].is_object(),
                driven: row["driven"] == true,
                status,
                since: row["since"].as_u64().unwrap_or(0),
                repo,
                branch: text(row, "branch"),
                space: row["space"]["name"].as_str().unwrap_or_default().to_string(),
                tui: row["frontend"] == "tui",
            })
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse((a.status != "idle", a.status == "working", a.since)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_most_recently_active_agent_in_a_worktree_comes_first() {
        let view = json!({"agents": [
            {"terminal": "w1:p1", "program": "claude", "title": "Old idle", "status": "idle", "since": 900, "cwd": "/r/api"},
            {"terminal": "w2:p1", "program": "codex", "title": "", "status": "working", "since": 100, "cwd": "/r/web/src", "branch": "me/eng-413"},
            {"terminal": "w3:p1", "program": "claude", "title": "Quotas", "status": "done", "since": 500, "cwd": "/r/api", "branch": "me/eng-412"},
            {"terminal": "w4:p1", "program": "claude", "title": "Elsewhere", "status": "working", "since": 999, "cwd": "/tmp/not-a-repo"},
            {"terminal": "", "program": "claude", "status": "working", "since": 999, "cwd": "/r/api"},
            {"terminal": "claude:1793611400000", "program": "claude", "status": "working", "since": 999, "cwd": "/r/api", "driven": true},
            {"terminal": "w5:p1", "program": "claude", "title": "Asks", "status": "blocked", "since": 400, "cwd": "/r/api", "question": {"text": "Allow?"}},
        ]});
        let toplevel = |cwd: &str| cwd.starts_with("/r/").then(|| cwd.split('/').take(3).collect::<Vec<_>>().join("/"));
        let found = agents(&view, &toplevel);
        let order: Vec<&str> = found.iter().map(|a| a.terminal.as_str()).collect();
        assert_eq!(order, ["claude:1793611400000", "w2:p1", "w3:p1", "w5:p1", "w1:p1"], "a driven session is one of them");
        assert!(found[0].driven && found[3].waiting && !found[1].waiting);
        assert_eq!(found[0].recipient(), "Claude Code · in chat, working");
        assert_eq!((found[1].name.as_str(), found[1].agent.as_str(), found[1].repo.as_str()), ("Codex", "Codex", "/r/web"));
        assert_eq!(found[1].label(), "web · Codex · working · me/eng-413");
        assert_eq!(found[2].label(), "api · Quotas · done · me/eng-412");
    }
}
