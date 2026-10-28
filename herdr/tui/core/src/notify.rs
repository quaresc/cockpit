//! System notifications with the cockpit icon: a click does the notification's action, a URL or a `cockpit focus`
//! target, and each `<label>=<action>` button does its own. Notifications switched off in the config silence every
//! caller, the mod's and the apps' alike.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::files;

/// Names a program that gets each notification's arguments instead of showing it, for tests.
pub const TEST_PROGRAM: &str = "COCKPIT_NOTIFY_PROGRAM";

/// The notifier app's own bundle id, which is never the terminal to bring forward.
const NOTIFIER: &str = "dev.cockpit.notifier";

/// Shows one notification: the macOS notifier app, else notify-send, else a herdr toast.
pub fn post(title: &str, body: &str, action: &str, buttons: &[String]) {
    if files::config()["notifications"] == false {
        return;
    }
    remember_terminal();
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
    fn quote_like_a_shell_word() {
        assert_eq!(quote("runbook:eng-4:a,b"), "'runbook:eng-4:a,b'");
        assert_eq!(quote("it's"), "'it'\"'\"'s'");
    }
}
