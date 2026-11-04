//! What runs in each pinned herdr space, listed under its sidebar row: one line per terminal with its agent or program
//! and state, reported as `$term_N` tokens from the pane listing the agents poller already made.

use std::collections::HashMap;
use std::sync::Mutex;

use cockpit_core::agent;
use cockpit_core::frontend::{self, status_mark};
use cockpit_core::text::fit;
use serde_json::Value;

/// Lines under a space; when more terminals run, the last one counts the rest.
const SLOTS: usize = 4;
/// Pinned spaces whose first pane runs cockpit's own app, left out of their list.
const APP_SPACES: [&str; 7] = ["inbox", "to-do", "runbook", "meetings", "monitor", "routines", "mail"];

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub waiting: bool,
}

/// A shell's title without its `user@host:` prefix, the folder saying more.
fn short_title(title: &str) -> &str {
    match title.split_once(':') {
        Some((who, rest)) if who.contains('@') && !who.contains(char::is_whitespace) => rest.trim_start(),
        _ => title,
    }
}

/// A pane's line: its agent's name and state mark, else its title after a prompt sign.
fn line(pane: &Value) -> Line {
    let title = short_title(pane["terminal_title_stripped"].as_str().unwrap_or_default()).trim();
    let Some(program) = pane["agent"].as_str().filter(|a| !a.is_empty()) else {
        return Line { text: format!("› {}", if title.is_empty() { "shell" } else { title }), waiting: false };
    };
    let name = agent::of_program(program).map(|a| a.name).unwrap_or_else(|| program.to_string());
    let status = pane["agent_status"].as_str().unwrap_or("idle");
    let mark = status_mark(status).unwrap_or("○");
    let text = if title.is_empty() || title == name { format!("{mark} {name}") } else { format!("{mark} {name} · {title}") };
    Line { text, waiting: status == "blocked" }
}

/// Each pinned space's lines by workspace id, waiting ones first; the actions space lists its sessions itself.
pub fn lines(spaces: &Value, panes: &Value, pinned: &[String]) -> Vec<(String, Vec<Line>)> {
    let mut out = Vec::new();
    for w in spaces["result"]["workspaces"].as_array().into_iter().flatten() {
        let label = w["label"].as_str().unwrap_or_default();
        let Some(name) = pinned.iter().find(|n| *n != "actions" && label.ends_with(&format!("  {n}"))) else { continue };
        let Some(id) = w["workspace_id"].as_str() else { continue };
        let skip = usize::from(APP_SPACES.contains(&name.as_str()));
        let mut found: Vec<Line> = panes["result"]["panes"].as_array().into_iter().flatten().filter(|p| p["workspace_id"] == id).skip(skip).map(line).collect();
        found.sort_by_key(|l| !l.waiting);
        out.push((id.to_string(), found));
    }
    out
}

/// The herdr arguments that list a space's lines under its row: one token per slot, a waiting line under its own name
/// so it stands out, the unused ones cleared.
pub fn tokens(ws: &str, lines: &[Line], room: usize) -> Vec<String> {
    let mut args: Vec<String> = ["workspace", "report-metadata", ws, "--source", "cockpit-terms"].map(String::from).to_vec();
    for i in 0..SLOTS {
        let more = i == SLOTS - 1 && lines.len() > SLOTS;
        let line = lines.get(i).filter(|_| !more);
        let text = match line {
            Some(l) => Some(fit(&l.text, room).trim_end().to_string()),
            None if more => Some(format!("  +{} more", lines.len() - i)),
            None => None,
        };
        let waiting = line.is_some_and(|l| l.waiting);
        for (name, value) in [(format!("term_{}", i + 1), text.clone().filter(|_| !waiting)), (format!("term_{}_wait", i + 1), text.filter(|_| waiting))] {
            match value {
                Some(value) => args.extend(["--token".to_string(), format!("{name}={value}")]),
                None => args.extend(["--clear-token".to_string(), name]),
            }
        }
    }
    args
}

/// Reports what changed since the last report: each pinned space's lines, cleared for a space no longer listed.
pub fn report(spaces: &Value, panes: &Value, pinned: &[String]) {
    static SENT: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);
    let room = frontend::sidebar_width().saturating_sub(4);
    let mut guard = SENT.lock().unwrap_or_else(|e| e.into_inner());
    let sent = guard.get_or_insert_with(HashMap::new);
    let now = lines(spaces, panes, pinned);
    let gone: Vec<String> = sent.keys().filter(|ws| !now.iter().any(|(id, _)| id == *ws)).cloned().collect();
    for ws in gone {
        let args = tokens(&ws, &[], room);
        frontend::herdr(&args.iter().map(String::as_str).collect::<Vec<_>>());
        sent.remove(&ws);
    }
    for (ws, found) in now {
        let args = tokens(&ws, &found, room);
        if sent.get(&ws) == Some(&args) {
            continue;
        }
        if frontend::herdr(&args.iter().map(String::as_str).collect::<Vec<_>>()).is_some() {
            sent.insert(ws, args);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pinned() -> Vec<String> {
        ["inbox", "terminal", "actions"].map(String::from).to_vec()
    }

    fn listing(panes: Value) -> (Value, Value) {
        let spaces = json!({"result": {"workspaces": [
            {"workspace_id": "w1", "label": "[1] ◆  inbox"},
            {"workspace_id": "w2", "label": "[2] ◆  terminal"},
            {"workspace_id": "w3", "label": "[3] ◆  actions"},
            {"workspace_id": "w4", "label": "[4] ENG-412 fix the login"},
        ]}});
        (spaces, json!({"result": {"panes": panes}}))
    }

    #[test]
    fn a_pinned_space_lists_its_agents_and_shells_waiting_first() {
        let (spaces, panes) = listing(json!([
            {"workspace_id": "w1", "terminal_title_stripped": "inbox"},
            {"workspace_id": "w1", "terminal_title_stripped": "me@laptop:~/acme/api"},
            {"workspace_id": "w2", "terminal_title_stripped": "me@laptop:~/acme/web"},
            {"workspace_id": "w2", "agent": "claude", "agent_status": "working", "terminal_title_stripped": "Store and enforce"},
            {"workspace_id": "w2", "agent": "codex", "agent_status": "blocked", "terminal_title_stripped": ""},
            {"workspace_id": "w3", "agent": "claude", "agent_status": "idle"},
            {"workspace_id": "w4", "agent": "claude", "agent_status": "working"},
        ]));
        let got = lines(&spaces, &panes, &pinned());
        let text = |ws: &str| got.iter().find(|(id, _)| id == ws).map(|(_, l)| l.iter().map(|l| l.text.as_str()).collect::<Vec<_>>());
        assert_eq!(text("w1"), Some(vec!["› ~/acme/api"]));
        assert_eq!(text("w2"), Some(vec!["⏸ Codex", "› ~/acme/web", "◐ Claude Code · Store and enforce"]));
        assert_eq!(text("w3"), None);
        assert_eq!(text("w4"), None);
        assert!(got.iter().find(|(id, _)| id == "w2").unwrap().1[0].waiting);
    }

    #[test]
    fn one_token_per_line_a_waiting_one_apart() {
        let lines = [Line { text: "⏸ Codex".into(), waiting: true }, Line { text: "› ~/acme/web".into(), waiting: false }];
        let line = tokens("w2", &lines, 30).join(" | ");
        assert!(line.starts_with("workspace | report-metadata | w2 | --source | cockpit-terms"));
        assert!(line.contains("--clear-token | term_1 | --token | term_1_wait=⏸ Codex"));
        assert!(line.contains("--token | term_2=› ~/acme/web | --clear-token | term_2_wait"));
        assert!(line.ends_with("--clear-token | term_4 | --clear-token | term_4_wait"));
    }

    #[test]
    fn a_long_line_is_cut_and_the_last_slot_counts_the_rest() {
        let lines: Vec<Line> = (1..=6).map(|n| Line { text: format!("› tail -f logs/service-{n}.log"), waiting: false }).collect();
        let args = tokens("w2", &lines, 12);
        assert!(args.contains(&"term_1=› tail -f l…".to_string()));
        assert!(args.contains(&"term_4=  +3 more".to_string()));
        let four = tokens("w2", &lines[..4], 40);
        assert!(four.contains(&"term_4=› tail -f logs/service-4.log".to_string()));
    }
}
