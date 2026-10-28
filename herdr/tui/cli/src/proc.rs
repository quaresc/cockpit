//! Child processes run to their end with a timeout, the PATH the plugin's tools live on, shell quoting, and JSON printed
//! the way the scripts' readers have always parsed it.

use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::time::Duration;

use serde_json::Value;

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Homebrew's and the user's bin folders before the inherited PATH: herdr runs its plugins with a bare one.
pub fn path() -> String {
    let home = cockpit_core::files::home();
    let local = home.join(".local/bin");
    let inherited = std::env::var("PATH").unwrap_or_default();
    format!("/opt/homebrew/bin:/usr/local/bin:{}:{inherited}", local.display())
}

/// The first executable of that name on `path()`.
pub fn which(name: &str) -> Option<String> {
    path().split(':').filter(|d| !d.is_empty()).map(|d| std::path::Path::new(d).join(name)).find(|p| executable(p)).map(|p| p.to_string_lossy().into_owned())
}

pub fn executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Runs argv on `path()` with these extra variables; None when it cannot start or outlives the timeout.
pub fn run(argv: &[&str], env: &[(&str, &str)], timeout: Duration) -> Option<Out> {
    let (program, args) = argv.split_first()?;
    let child = Command::new(program)
        .args(args)
        .env("PATH", path())
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let pid = child.id();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(out)) => Some(Out {
            code: out.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }),
        Ok(Err(_)) => None,
        Err(_) => {
            // SAFETY: kill only sends a signal to the child this function started and still owns.
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            None
        }
    }
}

/// Opens a URL in the default browser, on macOS or Linux.
pub fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let Ok(child) = Command::new(opener).arg(url).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() else {
        return;
    };
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let _ = rx.recv_timeout(Duration::from_secs(10));
}

/// A word the shell reads back as is.
pub fn quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        return s.into();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// An object as `{"a": 1, "b": "é"}`: its keys in the given order, ", " and ": " between them, non-ASCII escaped.
pub fn dumps(entries: &[(&str, Value)]) -> String {
    let fields: Vec<String> = entries.iter().map(|(k, v)| format!("{}: {}", ascii(&Value::from(*k)), ascii(v))).collect();
    format!("{{{}}}", fields.join(", "))
}

fn ascii(value: &Value) -> String {
    let mut out = String::new();
    for c in value.to_string().chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

/// Prints the message on stderr and exits 1, as a command that cannot go on does.
pub fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dumps_like_a_python_dict() {
        let printed = dumps(&[("url", json!("https://x/é")), ("tab", json!(3)), ("title", json!("a \"b\" 😀"))]);
        assert_eq!(printed, r#"{"url": "https://x/\u00e9", "tab": 3, "title": "a \"b\" \ud83d\ude00"}"#);
        assert_eq!(dumps(&[]), "{}");
    }

    #[test]
    fn quote_like_a_shell_word() {
        assert_eq!(quote(""), "''");
        assert_eq!(quote("/a/b-c.d"), "/a/b-c.d");
        assert_eq!(quote("https://x/?a=1&b"), "'https://x/?a=1&b'");
        assert_eq!(quote("it's"), "'it'\"'\"'s'");
    }

    #[test]
    fn a_timeout_ends_the_run() {
        assert!(run(&["sleep", "5"], &[], Duration::from_millis(200)).is_none());
        let out = run(&["sh", "-c", "echo hi; echo err >&2; exit 3"], &[], Duration::from_secs(5)).unwrap();
        assert_eq!((out.code, out.stdout.as_str(), out.stderr.as_str()), (3, "hi\n", "err\n"));
    }
}
