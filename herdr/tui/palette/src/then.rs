//! What the palette does once its popup has closed: a jump, a hub action, or text for the agent session it opened over.

use std::process::{Command, Stdio};
use std::time::Duration;

use cockpit_core::{actions, agent, files, frontend, hub};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum Then {
    /// A found item, brought into view in its space.
    Go(Value),
    /// A pinned space, by name.
    Space(String),
    RefreshInbox,
    Open(String),
    /// A folder in the person's editor.
    Editor(String),
    /// Typed at the session's prompt, not sent.
    Insert {
        pane: String,
        text: String,
    },
    /// Sent to the session as a prompt.
    Ask {
        pane: String,
        text: String,
    },
    /// A fork of the session in a tab of the actions space, asked to look into an item (no prompt: just the fork).
    Fork {
        agent: String,
        session: String,
        cwd: String,
        title: String,
        prompt: String,
    },
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
            Then::Editor(cwd) => editor(cwd),
            Then::Insert { pane, text } => match pane.strip_prefix(frontend::APP) {
                Some(terminal) => frontend::app_type(terminal, text).then_some(()).ok_or_else(|| format!("could not type into {terminal}")),
                None => herdr(&["pane", "send-text", pane, text]),
            },
            Then::Ask { pane, text } => match pane.strip_prefix(frontend::APP) {
                Some(terminal) => frontend::app_prompt(terminal, text).then_some(()).ok_or_else(|| format!("could not send to {terminal}")),
                None => herdr(&["agent", "prompt", pane, text]),
            },
            Then::Fork { agent, session, cwd, title, prompt } => fork(agent, session, cwd, title, prompt),
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

/// The command line that forks the agent's session and asks it the prompt, if any; an error when it cannot fork.
pub fn fork_line(agent: &str, session: &str, prompt: &str) -> Result<String, String> {
    let agent = agent::get(agent).ok_or_else(|| format!("no agent {agent:?}"))?;
    let argv = agent.fork_with(session, Some(prompt)).ok_or_else(|| format!("{} cannot fork a session", agent.name))?;
    Ok(agent.typed(&argv).iter().map(|w| quote(w)).collect::<Vec<_>>().join(" "))
}

/// The config's `editor` (code, cursor, zed, ...), else VS Code: its command line, else on macOS the app of that name.
fn editor(cwd: &str) -> Result<(), String> {
    let config = files::config();
    let name = config["editor"].as_str().filter(|e| !e.is_empty()).unwrap_or("code");
    if Command::new(name).arg(cwd).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()) {
        return Ok(());
    }
    let app = match name {
        "code" => "Visual Studio Code",
        "cursor" => "Cursor",
        "zed" => "Zed",
        "windsurf" => "Windsurf",
        "subl" => "Sublime Text",
        other => other,
    };
    // An editor whose command isn't on the PATH still opens by its app name on macOS.
    let opened = cfg!(target_os = "macos")
        && Command::new("open").args(["-a", app, cwd]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if opened { Ok(()) } else { Err(format!("{app} is not installed (set \"editor\" in the cockpit config)")) }
}

fn fork(agent: &str, session: &str, cwd: &str, title: &str, prompt: &str) -> Result<(), String> {
    if session.is_empty() || !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("not a session id: {session:?}"));
    }
    let line = fork_line(agent, session, prompt)?;
    let short: String = title.chars().take(28).collect();
    let label = if short.len() < title.len() { format!("⑂ {short}…") } else { format!("⑂ {title}") };
    let pane =
        actions::open_tab(&actions::pinned(), cwd, &label, None).or_else(|| frontend::new_space(cwd, &label)).ok_or("herdr opened no tab for the fork")?;
    match pane.strip_prefix(frontend::APP) {
        Some(_) => frontend::run_in(&pane, &line).then_some(()).ok_or_else(|| "the fork's tab took no command".to_string()),
        None => herdr(&["pane", "run", &pane, &line]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fork_line_quotes_its_prompt() {
        let line =
            |agent: &str, prompt: &str| fork_line(agent, "4f1c-77", prompt).map(|l| l.split_once(' ').map(|(_, rest)| rest.to_string()).unwrap_or_default());
        assert_eq!(line("claude", "look at it's state"), Ok("--resume 4f1c-77 --fork-session 'look at it'\"'\"'s state'".into()));
        assert_eq!(line("claude", ""), Ok("--resume 4f1c-77 --fork-session".into()));
        assert_eq!(line("codex", "go"), Ok("fork 4f1c-77 go".into()));
        assert_eq!(fork_line("copilot", "4f1c-77", ""), Err("GitHub Copilot CLI cannot fork a session".into()));
    }
}
