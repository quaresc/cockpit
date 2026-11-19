//! The Chromium side browser: cockpit's own Electron shell, which the hub starts and every command reaches over a unix
//! socket. One JSON request per line, one JSON answer per line; `subscribe` turns the connection into a stream of events.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::frontend::Frontend;
use crate::{files, options};

/// The browser that shows a page beside an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// terminal-browser in a terminal pane.
    Terminal,
    /// cockpit's Chromium window, docked in the Herdr app's side panel.
    Chromium,
}

/// What a command says when config.json asks for Chromium where it cannot run.
pub const MAC_ONLY: &str = "the Chromium side browser runs on macOS only; cockpit browse uses terminal-browser here";
pub const APP_ONLY: &str = "the Chromium side browser shows in the Herdr app only; cockpit browse uses terminal-browser in herdr";

/// The engine for a config, a frontend and a machine, and why Chromium was asked for and not used.
pub fn engine_for(config: &Value, frontend: Frontend, chromium_here: bool) -> (Engine, Option<&'static str>) {
    if config["sideBrowser"] != options::CHROMIUM {
        return (Engine::Terminal, None);
    }
    match (chromium_here, frontend) {
        (false, _) => (Engine::Terminal, Some(MAC_ONLY)),
        (true, Frontend::Tui) => (Engine::Terminal, Some(APP_ONLY)),
        (true, Frontend::App) => (Engine::Chromium, None),
    }
}

/// The engine this command uses, with the reason said once on stderr when Chromium was asked for and cannot be used.
pub fn engine() -> Engine {
    let (engine, why) = engine_for(&files::config(), crate::frontend::current(), options::CHROMIUM_HERE);
    if let Some(why) = why {
        eprintln!("cockpit: {why}");
    }
    engine
}

/// Where the shell keeps what must last: profiles, sign-ins, each worktree's tabs.
pub fn data() -> PathBuf {
    files::config_dir().join("browser")
}

/// Where the shell keeps what can be made again: the pinned programs, picked elements, its log.
pub fn cache() -> PathBuf {
    files::cache().join("browser")
}

/// ~/.cache/cockpit/browser.sock, or a name in the temp folder when that path is longer than a unix socket allows.
pub fn socket() -> PathBuf {
    let path = files::cache().join("browser.sock");
    if path.as_os_str().len() < 100 {
        return path;
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    files::cache().hash(&mut hasher);
    std::env::temp_dir().join(format!("cockpit-browser-{:016x}.sock", hasher.finish()))
}

/// A connection to the running shell; None when it is not running.
pub fn connect() -> Option<UnixStream> {
    UnixStream::connect(socket()).ok()
}

/// One request and its answer; an error when the shell is not running or answers with `error`.
pub fn request(req: &Value, timeout: Duration) -> Result<Value, String> {
    let stream = connect().ok_or("the Chromium side browser is not running")?;
    ask(stream, req, timeout)
}

/// One request on a connection and its answer.
pub fn ask(mut stream: UnixStream, req: &Value, timeout: Duration) -> Result<Value, String> {
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    writeln!(stream, "{req}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let answer: Value = serde_json::from_str(&line).map_err(|_| "the Chromium side browser sent no answer".to_string())?;
    match answer.get("error").and_then(Value::as_str) {
        Some(error) => Err(error.to_string()),
        None => Ok(answer),
    }
}

/// The worktree a folder belongs to: its git top level, else the folder itself.
pub fn worktree_of(cwd: &Path) -> PathBuf {
    let top = std::process::Command::new("git").arg("-C").arg(cwd).args(["rev-parse", "--show-toplevel"]).stderr(std::process::Stdio::null()).output();
    match top {
        Ok(out) if out.status.success() => PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()),
        _ => cwd.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn chromium_only_in_the_herdr_app_on_a_mac() {
        let asked = json!({"sideBrowser": "chromium"});
        assert_eq!(engine_for(&asked, Frontend::App, true), (Engine::Chromium, None));
        assert_eq!(engine_for(&asked, Frontend::Tui, true), (Engine::Terminal, Some(APP_ONLY)));
        assert_eq!(engine_for(&asked, Frontend::App, false), (Engine::Terminal, Some(MAC_ONLY)));
        assert_eq!(engine_for(&json!({}), Frontend::App, true), (Engine::Terminal, None));
        assert_eq!(engine_for(&json!({"sideBrowser": "terminal-browser"}), Frontend::App, true), (Engine::Terminal, None));
    }

    #[test]
    fn a_folder_outside_git_is_its_own_worktree() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(worktree_of(dir.path()), dir.path());
    }
}
