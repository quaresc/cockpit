//! agent-browser on the Chromium side browser: one agent-browser session per browser, pinned to one tab's debugging
//! target before each command, so a command never lands on the toolbar, on DevTools or on another session's page.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cockpit_core::browser;
use serde_json::{Value, json};

use super::install;
use crate::proc::{self, Out};

/// The commands agent-browser takes that would leave the browser cockpit runs, or end it.
const REFUSED: [&str; 8] = ["close", "quit", "exit", "connect", "launch", "install", "session", "disconnect"];
const REFUSED_FLAGS: [&str; 6] = ["--cdp", "--session", "--profile", "--headed", "--executable-path", "--auto-connect"];

/// Why a command is not passed on, when it is one cockpit keeps for itself.
pub fn refusal(command: &[String]) -> Option<String> {
    let first = command.first()?;
    if REFUSED.contains(&first.as_str()) {
        return Some(format!("{first} is not for the side browser: cockpit starts, connects and ends it"));
    }
    command
        .iter()
        .find(|a| REFUSED_FLAGS.contains(&a.as_str()))
        .map(|flag| format!("{flag} is not for the side browser: cockpit picks its session and its page"))
}

/// What a command asks of the browser's own tabs, which the shell does rather than agent-browser.
#[derive(Debug, PartialEq, Eq)]
pub enum Tabs {
    List { json: bool },
    New(Option<String>),
    Close(Option<String>),
    Switch(String),
}

/// A `tab …` command as a request for the shell; None for every other command.
pub fn tabs(command: &[String]) -> Option<Tabs> {
    let words: Vec<&str> = command.iter().map(String::as_str).filter(|a| *a != "--json").collect();
    let json = command.iter().any(|a| a == "--json");
    match words.as_slice() {
        ["tab"] | ["tab", "list"] => Some(Tabs::List { json }),
        ["tab", "new"] => Some(Tabs::New(None)),
        ["tab", "new", url] if !url.starts_with('-') => Some(Tabs::New(Some(url.to_string()))),
        ["tab", "close"] => Some(Tabs::Close(None)),
        ["tab", "close", which] => Some(Tabs::Close(Some(which.to_string()))),
        ["tab", which] if !which.starts_with('-') => Some(Tabs::Switch(which.to_string())),
        _ => None,
    }
}

/// The shell's tab an agent names: `t2` counts from one in the row, anything else is the tab's own id.
pub fn tab_named<'a>(tabs: &'a [Value], which: &str) -> Option<&'a str> {
    let by_place = which.strip_prefix('t').and_then(|n| n.parse::<usize>().ok()).and_then(|n| tabs.get(n.checked_sub(1)?));
    by_place.or_else(|| tabs.iter().find(|t| t["id"] == which)).and_then(|t| t["id"].as_str())
}

/// Where agent-browser keeps its sockets: short enough for a unix socket path, whatever the home folder.
pub fn socket_dir() -> PathBuf {
    let dir = browser::cache().join("ab");
    if dir.as_os_str().len() < 70 {
        return dir;
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    dir.hash(&mut hasher);
    std::env::temp_dir().join(format!("ck-ab-{:08x}", hasher.finish() as u32))
}

/// Whether a session's background process runs and is pinned to this debugging target, as its own files say.
pub fn bound(sockets: &Path, session: &str, target: &str) -> bool {
    let read = |ext: &str| std::fs::read_to_string(sockets.join(format!("{session}.{ext}"))).ok();
    let alive = read("pid").and_then(|pid| pid.trim().parse::<i32>().ok()).is_some_and(|pid| {
        // SAFETY: signal 0 sends nothing; it only asks whether the process exists.
        pid > 0 && unsafe { libc::kill(pid, 0) } == 0
    });
    let pinned = read("target").and_then(|text| serde_json::from_str::<Value>(&text).ok()).is_some_and(|t| t["targetId"] == target && t["pinned"] == true);
    alive && pinned
}

/// Ends the agent-browser processes of a shell that is gone and removes their files: each was bound to its
/// debugging port, which the next shell does not have.
pub fn prune() {
    let sockets = socket_dir();
    for entry in std::fs::read_dir(&sockets).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "pid")
            && let Some(pid) = std::fs::read_to_string(&path).ok().and_then(|p| p.trim().parse::<i32>().ok()).filter(|pid| *pid > 1)
        {
            // SAFETY: the signal goes to the one process whose id agent-browser wrote in its own pid file.
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
        let _ = std::fs::remove_file(&path);
    }
}

/// agent-browser bound to one tab of a session's browser.
pub struct Pinned {
    bin: String,
    pub session: String,
    pub tab: String,
    sockets: String,
    config: String,
}

impl Pinned {
    /// The variables every call gets: its own socket folder, and an empty config in place of the person's own.
    fn env(&self) -> Vec<(&str, &str)> {
        vec![("AGENT_BROWSER_SOCKET_DIR", self.sockets.as_str()), ("AGENT_BROWSER_CONFIG", self.config.as_str()), ("AGENT_BROWSER_NO_AUTO_DIALOG", "1")]
    }

    pub fn argv<'a>(&'a self, command: &[&'a str]) -> Vec<&'a str> {
        [self.bin.as_str(), "--session", self.session.as_str(), "--pin-tab"].into_iter().chain(command.iter().copied()).collect()
    }

    /// One command run to its end.
    pub fn run(&self, command: &[&str], timeout: Duration) -> Result<Out, String> {
        proc::run(&self.argv(command), &self.env(), timeout)
    }

    /// One command with this terminal's own input and output, and its exit code.
    pub fn pass(&self, command: &[String]) -> i32 {
        let argv: Vec<&str> = command.iter().map(String::as_str).collect();
        let argv = self.argv(&argv);
        let mut child = std::process::Command::new(argv[0]);
        child.args(&argv[1..]).env("PATH", proc::path()).envs(self.env());
        child.status().ok().and_then(|s| s.code()).unwrap_or(1)
    }
}

/// agent-browser pinned to a tab of the browser `owner` names (its active one when none is named); `suffix` names an
/// agent-browser session of its own for work that must not move the agent's.
pub fn pinned(owner: &str, tab: Option<&str>, suffix: &str) -> Result<Pinned, String> {
    let pins = install::pins()?;
    let bin = install::agent_browser(&pins);
    if !proc::executable(&bin) {
        return Err("agent-browser is not installed: run cockpit browser install".into());
    }
    let endpoint = browser::request(&json!({"op": "agent.endpoint", "owner": owner, "tab": tab}), Duration::from_secs(10))?;
    let (port, target) = (endpoint["port"].as_u64().unwrap_or(0).to_string(), endpoint["target"].as_str().unwrap_or_default().to_string());
    let sockets = socket_dir();
    let run = browser::cache().join("run");
    let config = run.join("agent-browser.json");
    std::fs::create_dir_all(&sockets).and_then(|()| std::fs::create_dir_all(&run)).and_then(|()| std::fs::write(&config, "{}\n")).map_err(|e| e.to_string())?;
    let found = Pinned {
        bin: bin.to_string_lossy().into_owned(),
        session: format!("{}{suffix}", endpoint["session"].as_str().unwrap_or("ck")),
        tab: endpoint["tab"].as_str().unwrap_or_default().to_string(),
        sockets: sockets.to_string_lossy().into_owned(),
        config: config.to_string_lossy().into_owned(),
    };
    // The first call starts agent-browser's background process: it runs from cockpit's own folder, never from a
    // worktree, which a program working in it would keep from being removed. The second binds the session to the tab.
    let call = |args: &[&str]| -> Result<(), String> {
        let mut command = std::process::Command::new(&found.bin);
        command.args(["--session", &found.session]).args(args).current_dir(&run).env("PATH", proc::path()).envs(found.env());
        let out = command.stdin(std::process::Stdio::null()).output().map_err(|e| format!("agent-browser did not start: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let said = String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).trim().lines().last().unwrap_or_default().to_string();
        Err(format!("agent-browser could not reach the page: {said}"))
    };
    // A session already bound to this tab is left as it is: binding again would forget the page's element refs.
    if !bound(&sockets, &found.session, &target) {
        call(&["--cdp", &port, "tab", "list", "--json"])?;
        call(&["--pin-tab", "tab", &target])?;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn what_would_leave_or_end_the_browser_is_refused() {
        assert!(refusal(&words("close")).unwrap().starts_with("close is not for the side browser"));
        assert!(refusal(&words("snapshot -i --cdp 9222")).unwrap().starts_with("--cdp is not"));
        assert_eq!(refusal(&words("snapshot -i")), None);
        assert_eq!(refusal(&words("click @e3")), None);
        assert_eq!(refusal(&[]), None);
    }

    #[test]
    fn tab_commands_go_to_the_shell() {
        assert_eq!(tabs(&words("tab list --json")), Some(Tabs::List { json: true }));
        assert_eq!(tabs(&words("tab")), Some(Tabs::List { json: false }));
        assert_eq!(tabs(&words("tab new http://localhost:3000/")), Some(Tabs::New(Some("http://localhost:3000/".into()))));
        assert_eq!(tabs(&words("tab close")), Some(Tabs::Close(None)));
        assert_eq!(tabs(&words("tab t2")), Some(Tabs::Switch("t2".into())));
        assert_eq!(tabs(&words("snapshot -i")), None);
        assert_eq!(tabs(&words("tab new --label docs http://localhost:3000/")), None, "a labelled tab is agent-browser's own");
    }

    #[test]
    fn a_session_is_bound_while_its_process_lives_on_the_same_target() {
        let dir = tempfile::tempdir().unwrap();
        let write = |ext: &str, text: String| std::fs::write(dir.path().join(format!("ck-acme.{ext}")), text).unwrap();
        assert!(!bound(dir.path(), "ck-acme", "T1"), "no files, no session");
        write("pid", std::process::id().to_string());
        write("target", json!({"targetId": "T1", "url": "", "pinned": true}).to_string());
        assert!(bound(dir.path(), "ck-acme", "T1"));
        assert!(!bound(dir.path(), "ck-acme", "T2"), "another tab's target");
        write("target", json!({"targetId": "T1", "pinned": false}).to_string());
        assert!(!bound(dir.path(), "ck-acme", "T1"), "not pinned");
        write("target", json!({"targetId": "T1", "pinned": true}).to_string());
        write("pid", "999999999".into());
        assert!(!bound(dir.path(), "ck-acme", "T1"), "its process is gone");
    }

    #[test]
    fn a_tab_is_named_by_its_place_or_its_id() {
        let listed = [json!({"id": "tab-a"}), json!({"id": "tab-b"})];
        assert_eq!(tab_named(&listed, "t2"), Some("tab-b"));
        assert_eq!(tab_named(&listed, "tab-a"), Some("tab-a"));
        assert_eq!(tab_named(&listed, "t3"), None);
        assert_eq!(tab_named(&listed, "t0"), None);
    }
}
