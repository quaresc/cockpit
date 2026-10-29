//! `cockpit focus`: what a click on a cockpit notification does besides opening a URL, then the terminal hosting herdr
//! brought forward on macOS.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cockpit_core::files;
use serde_json::{Value, json};

use crate::proc;

const USAGE: &str = "What a click on a cockpit notification does.

    cockpit focus workspace:<id>                    the space of that agent
    cockpit focus tab:<id>                          the tab of that agent, with its space
    cockpit focus space:<name>                      a pinned space, e.g. space:inbox
    cockpit focus runbook:<slug>[:<step>,<step>]    the runbook space, that runbook open and its steps played as done
    cockpit focus meeting:<event id>                the meetings space, that meeting's brief open
    cockpit focus monitor:<app>                     the monitor space, that app (its id or name) selected and unfolded
    cockpit focus update                            cockpit updated, in the background
    cockpit focus act:<review|ci|rerun|plan>:<ref>  a notification button: cockpit act with that action, then the terminal
    cockpit focus http(s)://…                       opened in the browser";

const TIMEOUT: Duration = Duration::from_secs(10);

pub fn main(args: &[String]) {
    let [target] = args else { proc::fail(USAGE) };
    if target.starts_with("http://") || target.starts_with("https://") {
        proc::open_url(target);
        return;
    }
    if target == "update" {
        update();
        return;
    }
    if let Some(id) = target.strip_prefix("meeting:") {
        request("meetings-focus.json", &json!({"id": id, "at": now()}));
        space("meetings");
    } else if let Some(app) = target.strip_prefix("monitor:") {
        request("monitor-focus.json", &json!({"app": app, "at": now()}));
        space("monitor");
    } else if let Some(rest) = target.strip_prefix("act:") {
        act(rest);
    } else if let Some(id) = target.strip_prefix("workspace:") {
        herdr(&["workspace", "focus", id]);
    } else if let Some(id) = target.strip_prefix("tab:") {
        herdr(&["tab", "focus", id]);
    } else if let Some(name) = target.strip_prefix("space:") {
        space(name);
    } else if let Some(rest) = target.strip_prefix("runbook:") {
        let (slug, steps) = rest.split_once(':').unwrap_or((rest, ""));
        let steps: Vec<&str> = steps.split(',').filter(|s| !s.is_empty()).collect();
        request("runbook-focus.json", &json!({"slug": slug, "steps": steps, "at": now()}));
        space("runbook");
    }
    terminal_forward();
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or_default()
}

/// A request the space's app picks up when the file changes.
fn request(name: &str, value: &Value) {
    let _ = files::write(&files::cache().join(name), value);
}

fn herdr_bin() -> String {
    std::env::var("HERDR_BIN_PATH").ok().filter(|p| !p.is_empty()).or_else(|| proc::which("herdr")).unwrap_or_else(|| "herdr".into())
}

fn herdr(args: &[&str]) -> Option<proc::Out> {
    let bin = herdr_bin();
    let argv: Vec<&str> = std::iter::once(bin.as_str()).chain(args.iter().copied()).collect();
    proc::run(&argv, &[], TIMEOUT)
}

/// Focuses the pinned space named `name`, labelled "<icon>  <name>".
fn space(name: &str) {
    let listed = herdr(&["workspace", "list"]).and_then(|out| serde_json::from_str::<Value>(&out.stdout).ok()).unwrap_or(Value::Null);
    let suffix = format!("  {name}");
    let found = listed["result"]["workspaces"].as_array().into_iter().flatten().find(|w| w["label"].as_str().is_some_and(|l| l.ends_with(&suffix)));
    if let Some(id) = found.and_then(|w| w["workspace_id"].as_str()) {
        herdr(&["workspace", "focus", id]);
    }
}

/// cockpit updated by update.sh, left running in the background with its output in update.log.
fn update() {
    let Ok(log) = std::fs::File::create(files::cache().join("update.log")) else { return };
    let Ok(err) = log.try_clone() else { return };
    let mut command = Command::new("sh");
    command.arg(files::herdr_dir().join("update.sh")).env("PATH", proc::path()).stdin(Stdio::null()).stdout(log).stderr(err);
    // SAFETY: setsid only detaches the child from the terminal, between fork and exec.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = command.spawn();
}

/// A notification button: `cockpit act <action> <ref>`, its output in act.log.
fn act(rest: &str) {
    let (action, reference) = (rest.split(':').next().unwrap_or(rest), rest.split_once(':').map_or(rest, |(_, r)| r));
    let Ok(log) = std::fs::File::create(files::cache().join("act.log")) else { return };
    let Ok(err) = log.try_clone() else { return };
    let Ok(own) = std::env::current_exe() else { return };
    let _ =
        Command::new(own).args(["act", action, reference]).env("COCKPIT_HERDR_DIR", files::herdr_dir()).stdin(Stdio::null()).stdout(log).stderr(err).status();
}

/// The terminal app hosting herdr, as the last notification recorded it, brought forward on macOS: in Ghostty, the very
/// window and tab where herdr runs, else the app.
fn terminal_forward() {
    let terminal = std::fs::read_to_string(files::cache().join("terminal-app")).unwrap_or_default();
    let terminal = terminal.trim_end_matches('\n');
    if !cfg!(target_os = "macos") || terminal.is_empty() {
        return;
    }
    if terminal == GHOSTTY && herdr_tty().is_some_and(|tty| ghostty_focus(&tty)) {
        return;
    }
    proc::run(&["open", "-b", terminal], &[], TIMEOUT);
}

const GHOSTTY: &str = "com.mitchellh.ghostty";

/// The terminal herdr's client draws in, as `/dev/ttys012`: its server runs detached, with none.
fn herdr_tty() -> Option<String> {
    let out = proc::run(&["ps", "-axo", "tty=,args="], &[], TIMEOUT)?;
    client_tty(&out.stdout)
}

fn client_tty(ps: &str) -> Option<String> {
    ps.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        let (tty, program, next) = (words.next()?, words.next()?, words.next());
        let is_client = program.rsplit('/').next() == Some("herdr") && next.is_none_or(|w| w.starts_with('-'));
        (is_client && tty != "??").then(|| format!("/dev/{tty}"))
    })
}

/// Focuses the Ghostty terminal on this tty, with its window and tab (Ghostty 1.3.2 and later script it); false when
/// none is, or Ghostty says no.
fn ghostty_focus(tty: &str) -> bool {
    let script = format!("tell application \"Ghostty\" to focus (first terminal whose tty is \"{tty}\")");
    proc::run(&["osascript", "-e", &script], &[], TIMEOUT).is_some_and(|out| out.ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn herdrs_client_is_the_one_on_a_terminal() {
        let ps = "??       /opt/homebrew/bin/herdr server\nttys003  -zsh\nttys000  herdr\nttys004  herdr agent prompt w1:p1 hi\n";
        assert_eq!(client_tty(ps), Some("/dev/ttys000".into()));
        assert_eq!(client_tty("ttys000  /usr/local/bin/herdr --session work\n"), Some("/dev/ttys000".into()));
        assert_eq!(client_tty("??  herdr server\nttys001  herdrd\n"), None);
    }
}
