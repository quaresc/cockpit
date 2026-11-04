//! `cockpit focus`: what a click on a cockpit notification does besides opening a URL, then the terminal hosting herdr
//! brought forward on macOS.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cockpit_core::{files, frontend};
use serde_json::{Value, json};

use crate::proc;

const USAGE: &str = "What a click on a cockpit notification does.

    cockpit focus workspace:<id>                    the space of that agent
    cockpit focus tab:<id>                          the tab of that agent, with its space
    cockpit focus space:<name>                      a pinned space, e.g. space:inbox
    cockpit focus runbook:<slug>[:<step>,<step>]    the runbook space, that runbook open and its steps played as done
    cockpit focus meeting:<event id>                the meetings space, that meeting's brief open
    cockpit focus monitor:<app>                     the monitor space, that app (its id or name) selected and unfolded
    cockpit focus routine:<slug>                    the routines space, that routine selected
    cockpit focus mail:<message id>                 the mail space, that message open
    cockpit focus inbox:<PR url|ticket id>          the inbox space, that row selected
    cockpit focus herdr-app:<target>                that target in the Herdr app (its notifications' clicks)
    cockpit focus update                            cockpit updated, in the background
    cockpit focus resume                            the agents a restart stopped, resumed where they were
    cockpit focus act:<review|ci|rerun|plan|merge|routine>:<ref>
                                                    a notification button: cockpit act with that action, then the terminal
    cockpit focus http(s)://…                       opened in the browser";

const TIMEOUT: Duration = Duration::from_secs(10);

pub fn main(args: &[String]) {
    let [target] = args else { proc::fail(USAGE) };
    if let Some(rest) = target.strip_prefix(cockpit_core::notify::IN_APP) {
        return frontend::with(Some(frontend::Frontend::App), || main(&[rest.to_string()]));
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        proc::open_url(target);
        return;
    }
    if target == "update" {
        update();
        return;
    }
    if target == "resume" {
        let _ = cockpit_core::resume::resume_all();
        terminal_forward();
        return;
    }
    if frontend::current() == frontend::Frontend::App
        && let Some(found) = app_target(target)
    {
        // The hub shows the app's block and asks its view to bring the item into view.
        let _ = cockpit_core::hub::request(&json!({"op": "go", "target": found, "frontend": "app"}), TIMEOUT);
        terminal_forward();
        return;
    }
    if let Some(id) = target.strip_prefix("meeting:") {
        request("meetings-focus.json", &json!({"id": id, "at": now()}));
        space("meetings");
    } else if let Some(app) = target.strip_prefix("monitor:") {
        request("monitor-focus.json", &json!({"app": app, "at": now()}));
        space("monitor");
    } else if let Some(key) = target.strip_prefix("inbox:") {
        cockpit_core::focus::request("inbox", &json!({"app": "inbox", "key": key}));
        space("inbox");
    } else if let Some(id) = target.strip_prefix("mail:") {
        cockpit_core::focus::request("mail", &json!({"app": "mail", "key": id}));
        space("mail");
    } else if let Some(slug) = target.strip_prefix("routine:") {
        cockpit_core::focus::request("routines", &json!({"app": "routines", "key": slug}));
        space("routines");
    } else if let Some(rest) = target.strip_prefix("act:") {
        act(rest);
    } else if let Some(id) = target.strip_prefix("workspace:") {
        match id.strip_prefix(frontend::APP) {
            Some(id) => app_select("space.select", id),
            None => _ = herdr(&["workspace", "focus", id]),
        }
    } else if let Some(id) = target.strip_prefix("tab:") {
        match id.strip_prefix(frontend::APP) {
            Some(id) => app_select("tab.select", id),
            None => _ = herdr(&["tab", "focus", id]),
        }
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

/// What a click asks of an app's view in the Herdr app: {app, key, ...}, as the palette's targets are.
fn app_target(target: &str) -> Option<Value> {
    let (kind, rest) = target.split_once(':')?;
    Some(match kind {
        "meeting" => json!({"app": "meetings", "key": rest}),
        "monitor" => json!({"app": "monitor", "key": rest}),
        "routine" => json!({"app": "routines", "key": rest}),
        "inbox" => json!({"app": "inbox", "key": rest}),
        "todo" => json!({"app": "todo", "key": rest}),
        "mail" => json!({"app": "mail", "key": rest}),
        "runbook" => {
            let (slug, steps) = rest.split_once(':').unwrap_or((rest, ""));
            json!({"app": "runbook", "key": slug, "steps": steps.split(',').filter(|s| !s.is_empty()).collect::<Vec<_>>()})
        }
        _ => return None,
    })
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

/// A space or tab of the Herdr app selected, in the work area.
fn app_select(command: &str, id: &str) {
    frontend::app_show_work();
    frontend::app_do(command, &json!({"id": id}));
}

/// Focuses the pinned space named `name`, labelled "<icon>  <name>"; in the Herdr app, that app's block.
fn space(name: &str) {
    if frontend::current() == frontend::Frontend::App {
        frontend::app_show(if name == "to-do" { "todo" } else { name });
        return;
    }
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
    let mut command = Command::new(own);
    command.args(["act", action, reference]).env("COCKPIT_HERDR_DIR", files::herdr_dir()).env("COCKPIT_FRONTEND", frontend::current().name());
    let _ = command.stdin(Stdio::null()).stdout(log).stderr(err).status();
}

/// The terminal app hosting herdr, as the last notification recorded it, brought forward on macOS: in Ghostty, the very
/// window and tab where herdr runs, else the app.
fn terminal_forward() {
    let terminal = std::fs::read_to_string(files::cache().join("terminal-app")).unwrap_or_default();
    let terminal = terminal.trim_end_matches('\n');
    if !cfg!(target_os = "macos") {
        return;
    }
    if frontend::current() == frontend::Frontend::App {
        proc::run(&["open", "-a", "Herdr"], &[], TIMEOUT);
        return;
    }
    if terminal.is_empty() {
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
