//! System notifications with the cockpit icon: a click does the notification's action, a URL or a `cockpit focus`
//! target, and each `<label>=<action>` button does its own. Notifications switched off in the config silence every
//! caller, the mod's and the apps' alike.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use std::sync::OnceLock;

use serde_json::{Value, json};

use crate::files;
use crate::frontend::{self, Frontend};

/// Names a program that gets each notification's arguments instead of showing it, for tests.
pub const TEST_PROGRAM: &str = "COCKPIT_NOTIFY_PROGRAM";

/// The notifier app's own bundle id, which is never the terminal to bring forward.
const NOTIFIER: &str = "dev.cockpit.notifier";

/// In the hub's own process, how a notification reaches the Herdr app's cockpit mod: true when a window took it.
pub static GUI: OnceLock<fn(&Value) -> bool> = OnceLock::new();

/// Shows one notification: in the Herdr app as its toast (the mod falls back here when its window is not in front),
/// else the macOS notifier app, else notify-send, else a herdr toast.
pub fn post(title: &str, body: &str, action: &str, buttons: &[String]) {
    if files::config()["notifications"] == false {
        return;
    }
    let to_window = json!({"title": title, "body": body, "action": action, "row": row_target(action), "buttons": buttons});
    let frontend = frontend::for_notice();
    if frontend == Frontend::App && to_app(&to_window) {
        return;
    }
    system(frontend, title, body, action, buttons);
}

/// Prefixes a click's target so `cockpit focus` drives the Herdr app with it, whatever the environment it runs in.
pub const IN_APP: &str = "herdr-app:";

/// Where a click leads in the Herdr app: the row the notification is about, in its view (`inbox:<PR url>`,
/// `inbox:<ticket id>`), where its other buttons are; else the action itself.
pub fn row_target(action: &str) -> String {
    let bare = ["act:review:", "act:ci:", "act:rerun:", "act:plan:"].iter().find_map(|p| action.strip_prefix(p)).unwrap_or(action);
    if let Some(pr) = pr_url(bare) {
        return format!("inbox:{pr}");
    }
    if action.starts_with("act:plan:") && !bare.is_empty() {
        return format!("inbox:{bare}");
    }
    if bare.starts_with("http")
        && let Some(id) = ticket_id(bare)
    {
        return format!("inbox:{id}");
    }
    action.to_string()
}

/// "https://github.com/acme/api/pull/4" out of that PR's URL or one of its pages (`/checks`, `/files`).
fn pr_url(s: &str) -> Option<String> {
    let at = s.find("/pull/")? + "/pull/".len();
    let digits = s[at..].chars().take_while(char::is_ascii_digit).count();
    (s.starts_with("http") && digits > 0).then(|| s[..at + digits].to_string())
}

/// The ticket id in a tracker's URL: "ENG-2" in Linear's `/issue/ENG-2/slug` or Jira's `/browse/ENG-2`.
fn ticket_id(url: &str) -> Option<String> {
    url.split(['/', '?', '#']).find(|part| {
        let Some((key, n)) = part.split_once('-') else { return false };
        key.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && key.chars().all(|c| c.is_ascii_alphanumeric()) && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
    }).map(str::to_string)
}

/// A system notification posted for the Herdr app: its click and buttons lead to rows of the app, not the browser.
fn for_app(action: &str, buttons: &[String]) -> (String, Vec<String>) {
    let lead = |target: &str| if target.is_empty() { String::new() } else { format!("{IN_APP}{}", row_target(target)) };
    let buttons = buttons
        .iter()
        .map(|b| match b.split_once('=') {
            Some((label, target)) if !target.starts_with("http") => format!("{label}={IN_APP}{target}"),
            _ => b.clone(),
        })
        .collect();
    (lead(action), buttons)
}

/// Hands the notification to the cockpit mod through the hub: in-process in the hub, else over its socket when one
/// runs (none is started for it). False when no Herdr window follows the hub.
fn to_app(notification: &Value) -> bool {
    if let Some(deliver) = GUI.get() {
        return deliver(notification);
    }
    if std::os::unix::net::UnixStream::connect(crate::hub::socket()).is_err() {
        return false;
    }
    crate::hub::request(&json!({"op": "notify", "notification": notification}), std::time::Duration::from_secs(2)).is_ok_and(|a| a["delivered"] == true)
}

/// The system's notification, never the Herdr app's: what the mod asks for when its window is not in front.
pub fn post_system(title: &str, body: &str, action: &str, buttons: &[String]) {
    system(frontend::current(), title, body, action, buttons);
}

/// The system's notification, its click and buttons leading into that frontend.
fn system(frontend: Frontend, title: &str, body: &str, action: &str, buttons: &[String]) {
    if files::config()["notifications"] == false {
        return;
    }
    remember_terminal();
    let (action, buttons) = if frontend == Frontend::App { for_app(action, buttons) } else { (action.to_string(), buttons.to_vec()) };
    let (action, buttons) = (action.as_str(), buttons.as_slice());
    if let Some(program) = std::env::var_os(TEST_PROGRAM).filter(|p| !p.is_empty()) {
        let _ = quiet(Command::new(program).args([title, body, action]).args(buttons)).status();
        return;
    }
    let shown = if cfg!(target_os = "macos") { notifier(title, body, action, buttons) } else { notify_send(title, body, action, buttons) };
    if !shown {
        let herdr = std::env::var("HERDR_BIN_PATH").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "herdr".into());
        let _ = quiet(Command::new(herdr).args(["notification", "show", title, "--body", body, "--sound", "request"])).status();
    }
}

/// The terminal app hosting herdr, brought forward when a click focuses a space.
fn remember_terminal() {
    let Ok(terminal) = std::env::var("__CFBundleIdentifier") else { return };
    if !terminal.is_empty() && terminal != NOTIFIER {
        let _ = std::fs::write(files::cache().join("terminal-app"), format!("{terminal}\n"));
    }
}

fn quiet(command: &mut Command) -> &mut Command {
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
}

/// macOS: the Cockpit app, built on first use, posts it with the cockpit icon and its buttons.
fn notifier(title: &str, body: &str, action: &str, buttons: &[String]) -> bool {
    let app = files::config_dir().join("Cockpit.app/Contents/MacOS/cockpit-notify");
    let built = quiet(Command::new("sh").arg(files::herdr_dir().join("notifier/build.sh"))).status().is_ok_and(|s| s.success());
    built
        && executable(&app)
        && Command::new(&app).arg("post").args([title, body, action]).args(buttons).stdin(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Linux: notify-send. Its default action (a click, where the daemon supports it) does the action and each button
/// `b<N>` its own, from a background process that waits for the choice.
fn notify_send(title: &str, body: &str, action: &str, buttons: &[String]) -> bool {
    if !on_path("notify-send") {
        return false;
    }
    let icon = files::herdr_dir().join("notifier/icon.png").to_string_lossy().into_owned();
    if (!action.is_empty() || !buttons.is_empty()) && supports_actions() {
        let mut argv: Vec<String> = ["notify-send", "-a", "Cockpit", "-i", &icon].map(String::from).to_vec();
        let mut cases = String::new();
        if !action.is_empty() {
            argv.extend(["-A".into(), "default=Open".into()]);
            cases += &format!("default) target={} ;; ", quote(action));
        }
        for (n, button) in buttons.iter().enumerate() {
            let (label, target) = button.split_once('=').unwrap_or((button, button));
            argv.extend(["-A".into(), format!("b{}={label}", n + 1)]);
            cases += &format!("b{}) target={} ;; ", n + 1, quote(target));
        }
        argv.extend(["--wait".into(), title.into(), body.into()]);
        let focus = quote(&script("cockpit.sh").to_string_lossy());
        let wait = format!("choice=$(\"$@\" 2>/dev/null); target=; case \"$choice\" in {cases}esac; [ -n \"$target\" ] && exec sh {focus} focus \"$target\"");
        if let Ok(mut child) = quiet(Command::new("sh").arg("-c").arg(wait).arg("sh").args(argv)).process_group(0).spawn() {
            std::thread::spawn(move || child.wait());
            return true;
        }
    }
    quiet(Command::new("notify-send").args(["-a", "Cockpit", "-i", &icon, title, body])).status().is_ok_and(|s| s.success())
}

fn supports_actions() -> bool {
    Command::new("notify-send").arg("--help").stdin(Stdio::null()).output().is_ok_and(|out| {
        let help = [out.stdout, out.stderr].concat();
        String::from_utf8_lossy(&help).contains("--action")
    })
}

/// One of the plugin's scripts, through the link that follows herdr's updates, else in the herdr folder.
fn script(name: &str) -> PathBuf {
    let linked = files::config_dir().join("herdr").join(name);
    if linked.exists() { linked } else { files::herdr_dir().join(name) }
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| executable(&dir.join(name))))
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// A word the shell reads back as is.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_leads_to_the_row_it_is_about() {
        assert_eq!(row_target("https://github.com/acme/api/pull/4/checks"), "inbox:https://github.com/acme/api/pull/4");
        assert_eq!(row_target("act:review:https://github.com/acme/web/pull/9"), "inbox:https://github.com/acme/web/pull/9");
        assert_eq!(row_target("https://linear.app/acme/issue/ENG-2/sign-in"), "inbox:ENG-2");
        assert_eq!(row_target("act:plan:ENG-2"), "inbox:ENG-2");
        assert_eq!(row_target("runbook:eng-4:a,b"), "runbook:eng-4:a,b");
        assert_eq!(row_target("https://status.acme.dev"), "https://status.acme.dev");
        let (action, buttons) = for_app("https://github.com/acme/api/pull/4/checks", &["Why?=act:ci:https://github.com/acme/api/pull/4".into(), "Docs=https://x.dev".into()]);
        assert_eq!(action, "herdr-app:inbox:https://github.com/acme/api/pull/4");
        assert_eq!(buttons, ["Why?=herdr-app:act:ci:https://github.com/acme/api/pull/4", "Docs=https://x.dev"]);
    }

    #[test]
    fn quote_like_a_shell_word() {
        assert_eq!(quote("runbook:eng-4:a,b"), "'runbook:eng-4:a,b'");
        assert_eq!(quote("it's"), "'it'\"'\"'s'");
    }
}
