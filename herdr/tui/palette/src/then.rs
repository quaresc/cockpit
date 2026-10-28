//! What the palette does once its popup has closed: a jump, a hub action, or text for the Claude session it opened over.

use std::process::{Command, Stdio};
use std::time::Duration;

use cockpit_core::{actions, files, hub};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum Then {
    /// A found item, brought into view in its space.
    Go(Value),
    /// A pinned space, by name.
    Space(String),
    RefreshInbox,
    Open(String),
    /// Typed at the session's prompt, not sent.
    Insert { pane: String, text: String },
    /// Sent to the session as a prompt.
    Ask { pane: String, text: String },
    /// A fork of the session in a tab of the actions space, asked to look into an item.
    Fork { session: String, cwd: String, title: String, prompt: String },
}

impl Then {
    /// Does it; what went wrong otherwise, for a notification, since the popup is gone by then.
    pub fn run(&self) -> Result<(), String> {
        match self {
            Then::Go(target) => hub::request(&json!({"op": "go", "target": target}), Duration::from_secs(10)).map(drop),
            Then::Space(name) => space(name),
            Then::RefreshInbox => {
                let request = json!({"op": "act", "app": "inbox", "action": "request_refresh", "args": [], "armed": null});
                hub::request(&request, Duration::from_secs(10)).map(drop)
            }
            Then::Open(url) => {
                files::open_url(url);
                Ok(())
            }
            Then::Insert { pane, text } => herdr(&["pane", "send-text", pane, text]),
            Then::Ask { pane, text } => herdr(&["agent", "prompt", pane, text]),
            Then::Fork { session, cwd, title, prompt } => fork(session, cwd, title, prompt),
        }
    }
}

/// A herdr command run to its end; its error output when it fails.
fn herdr(args: &[&str]) -> Result<(), String> {
    let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let out = Command::new(bin).args(args).stdin(Stdio::null()).output().map_err(|e| format!("herdr: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("herdr {}: {}", args[..2].join(" "), String::from_utf8_lossy(&out.stderr).trim())) }
}

/// Focuses the pinned space labelled "<icon>  <name>".
fn space(name: &str) -> Result<(), String> {
    let listed = actions::herdr(&["workspace", "list"]).ok_or("herdr lists no spaces")?;
    let suffix = format!("  {name}");
    let id = listed["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|w| w["label"].as_str().is_some_and(|l| l.ends_with(&suffix)))
        .and_then(|w| w["workspace_id"].as_str())
        .ok_or_else(|| format!("no {name} space is open"))?;
    herdr(&["workspace", "focus", id])
}

/// A word the shell reads back as is.
fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        return s.into();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// The command line that resumes the session as a fork and asks it the prompt.
pub fn fork_line(session: &str, prompt: &str) -> String {
    ["claude", "--resume", session, "--fork-session", prompt].map(quote).join(" ")
}

fn fork(session: &str, cwd: &str, title: &str, prompt: &str) -> Result<(), String> {
    if session.is_empty() || !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("not a session id: {session:?}"));
    }
    let short: String = title.chars().take(28).collect();
    let label = if short.len() < title.len() { format!("⑂ {short}…") } else { format!("⑂ {title}") };
    let pane = actions::open_tab(&actions::pinned(), cwd, &label, None)
        .or_else(|| {
            let created = actions::herdr(&["workspace", "create", "--cwd", cwd, "--label", &label, "--focus"])?;
            created["root_pane"]["pane_id"].as_str().map(str::to_string)
        })
        .ok_or("herdr opened no tab for the fork")?;
    herdr(&["pane", "run", &pane, &fork_line(session, prompt)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fork_line_quotes_its_prompt() {
        assert_eq!(fork_line("4f1c-77", "look at it's state"), "claude --resume 4f1c-77 --fork-session 'look at it'\"'\"'s state'");
    }
}
