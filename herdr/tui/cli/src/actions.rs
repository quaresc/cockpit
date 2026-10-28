//! `cockpit actions`: the pinned actions space's tabs, and its sidebar row listing the Claude sessions running there.

use cockpit_core::actions::{herdr, open_tab, pinned, space};
use cockpit_core::text::fit;
use serde_json::Value;

use crate::proc::fail;

const USAGE: &str = "The pinned actions space: a tab per one-off Claude session the inbox or the runbook starts.

    cockpit actions tab <cwd> <label> [key]   a new tab there, in view, its pane id printed; nothing without the space
    cockpit actions sidebar                    the space's sidebar row lists its sessions again";

/// The sessions the sidebar lists under the actions space, the last line saying how many more there are.
const SLOTS: usize = 6;

fn mark(status: &str) -> Option<&'static str> {
    Some(match status {
        "working" => "◐",
        "blocked" => "⏸",
        "done" => "✓",
        "idle" => "○",
        _ => return None,
    })
}

pub fn main(args: &[String]) {
    match args {
        [cmd] if cmd == "sidebar" => sidebar(),
        [cmd, cwd, label, rest @ ..] if cmd == "tab" && rest.len() <= 1 => {
            if let Some(pane) = open_tab(&pinned(), cwd, label, rest.first().map(String::as_str)) {
                println!("{pane}");
            }
        }
        _ => fail(USAGE),
    }
}

/// herdr's sidebar width from its config, 34 when it sets none.
fn sidebar_width() -> usize {
    let config = std::fs::read_to_string(cockpit_core::files::home().join(".config/herdr/config.toml")).unwrap_or_default();
    width_in(&config).unwrap_or(34)
}

fn width_in(config: &str) -> Option<usize> {
    config.lines().find_map(|line| {
        let value = line.trim_start().strip_prefix("sidebar_width")?.trim_start().strip_prefix('=')?.trim_start();
        let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    })
}

/// The herdr arguments that list the sessions under the space's row: one token per slot, a waiting one under its own
/// name so it stands out, the others cleared.
pub fn tokens(ws: &str, tabs: &[Value], room: usize) -> Vec<String> {
    let mut args: Vec<String> = ["workspace", "report-metadata", ws, "--source", "cockpit-actions"].map(String::from).to_vec();
    for i in 0..SLOTS {
        let mut tab = tabs.get(i);
        let mut text = tab.map(|t| {
            let line = format!("{} {}", mark(t["agent_status"].as_str().unwrap_or_default()).unwrap_or_default(), t["label"].as_str().unwrap_or_default());
            fit(&line, room).trim_end().to_string()
        });
        if i == SLOTS - 1 && tabs.len() > SLOTS {
            tab = None;
            text = Some(format!("  +{} more", tabs.len() - i));
        }
        let waiting = tab.is_some_and(|t| t["agent_status"] == "blocked");
        let text = text.filter(|t| !t.is_empty());
        for (name, value) in [(format!("act_{}", i + 1), if waiting { None } else { text.clone() }), (format!("act_{}_wait", i + 1), if waiting { text.clone() } else { None })] {
            match value {
                Some(value) => args.extend(["--token".to_string(), format!("{name}={value}")]),
                None => args.extend(["--clear-token".to_string(), name]),
            }
        }
    }
    args
}

/// Lists the space's Claude sessions under its sidebar row, one line each with its state; a waiting one stands out.
fn sidebar() {
    let Some(ws) = space(&pinned()) else { return };
    let listed = herdr(&["tab", "list"]).unwrap_or(Value::Null);
    let tabs: Vec<Value> = listed["tabs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["workspace_id"] == ws.as_str() && t["agent_status"].as_str().and_then(mark).is_some())
        .cloned()
        .collect();
    let args = tokens(&ws, &tabs, sidebar_width().saturating_sub(4));
    herdr(&args.iter().map(String::as_str).collect::<Vec<_>>());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tab(label: &str, status: &str) -> Value {
        json!({"workspace_id": "w3", "label": label, "agent_status": status})
    }

    #[test]
    fn the_width_herdr_sets() {
        assert_eq!(width_in("[ui]\n  sidebar_width = 40 # wide\n"), Some(40));
        assert_eq!(width_in("sidebar_widths = 3\n"), None);
        assert_eq!(width_in("theme = \"x\"\n"), None);
    }

    #[test]
    fn one_token_per_session_a_waiting_one_apart() {
        let args = tokens("w3", &[tab("review api#4", "working"), tab("plan eng-4", "blocked")], 30);
        let line = args.join(" | ");
        assert!(line.starts_with("workspace | report-metadata | w3 | --source | cockpit-actions"));
        assert!(line.contains("--token | act_1=◐ review api#4 | --clear-token | act_1_wait"));
        assert!(line.contains("--clear-token | act_2 | --token | act_2_wait=⏸ plan eng-4"));
        assert!(line.ends_with("--clear-token | act_6 | --clear-token | act_6_wait"));
    }

    #[test]
    fn a_long_label_is_cut_and_the_last_slot_counts_the_rest() {
        let tabs: Vec<Value> = (1..=8).map(|n| tab(&format!("talk eng-{n} about a very long thing"), "idle")).collect();
        let args = tokens("w3", &tabs, 12);
        assert!(args.contains(&"act_1=○ talk eng-…".to_string()));
        assert!(args.contains(&"act_6=  +3 more".to_string()));
    }
}
