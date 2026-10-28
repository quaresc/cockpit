//! `cockpit shots`: screenshots of a page at phone, tablet and desktop sizes, taken in the terminal-browser beside a
//! pane. Each size gets a tab of its own, closed afterwards, so the person's tabs keep their size.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::browse::{self, DEFAULT_SIZE, browsers_beside, open_beside, pane_of, terminal_browser};
use crate::proc::{dumps, fail};

const USAGE: &str = "Screenshots of a page at phone, tablet and desktop sizes, taken in the terminal-browser beside a pane.

    cockpit shots <url> <out dir> [--sizes mobile,tablet,desktop] [--full] [--session <Claude session id>]

Prints the screenshots as JSON, {size: path}. Without terminal-browser beside a pane in herdr, it exits 2 and says so.";

const SIZES: [(&str, &[&str]); 3] =
    [("mobile", &["set", "device", "iPhone 16"]), ("tablet", &["set", "device", "iPad"]), ("desktop", &["set", "viewport", "1440", "900"])];

/// How long to wait between polls of a browser starting or a tab getting its page, and how many polls at most.
const POLL: Duration = Duration::from_millis(500);
const POLLS: usize = 40;

fn run_action(tb: &str, key: &str, tab: &str, command: &[&str], timeout: Duration) -> Option<crate::proc::Out> {
    let mut argv = vec![tb, "action", "--browser", key, "--tab", tab, "--"];
    argv.extend_from_slice(command);
    browse::run(&argv, timeout)
}

/// One browser action that must succeed; its error names the command and what the browser said.
fn action(tb: &str, key: &str, tab: &str, command: &[&str], timeout: Duration) -> Result<(), String> {
    match run_action(tb, key, tab, command, timeout) {
        Some(out) if out.ok() => Ok(()),
        done => {
            let said = match done {
                Some(out) => {
                    let text = if out.stderr.is_empty() { out.stdout } else { out.stderr };
                    text.trim().chars().take(200).collect()
                }
                None => "timed out".to_string(),
            };
            Err(format!("{}: {said}", command.iter().take(2).copied().collect::<Vec<_>>().join(" ")))
        }
    }
}

pub fn main(args: &[String]) {
    let (mut sizes, mut full, mut session): (Vec<&str>, bool, Option<String>) = (SIZES.iter().map(|(s, _)| *s).collect(), false, None);
    let mut rest = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sizes" if args.len() > 0 => {
                let wanted = args.next().map(String::as_str).unwrap_or_default();
                sizes = wanted.split(',').filter_map(|s| SIZES.iter().find(|(name, _)| *name == s).map(|(name, _)| *name)).collect();
            }
            "--full" => full = true,
            "--session" if args.len() > 0 => session = args.next().cloned(),
            _ => rest.push(arg.clone()),
        }
    }
    let [url, out] = rest.as_slice() else { fail(USAGE) };
    let out = std::path::absolute(out).unwrap_or_else(|_| PathBuf::from(out));
    let tb = terminal_browser();
    let pane = tb.as_ref().and_then(|_| pane_of(session.as_deref()));
    let (Some(tb), Some(pane)) = (tb, pane) else {
        eprintln!("terminal-browser is not beside a pane in herdr: take these screenshots in Chrome instead");
        std::process::exit(2);
    };
    if browsers_beside(&tb, &pane).is_empty() && !open_beside(&tb, &pane, url, DEFAULT_SIZE, false) {
        fail("could not open terminal-browser beside the pane");
    }
    // A browser just split in takes a moment to start.
    let browsers = wait_for(|| Some(browsers_beside(&tb, &pane)).filter(|b| !b.is_empty())).unwrap_or_else(|| fail("terminal-browser did not start beside the pane"));
    let key = match &browsers[0]["key"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if let Err(err) = std::fs::create_dir_all(&out) {
        fail(&format!("{}: {err}", out.display()));
    }
    let mut shots = Vec::new();
    for size in sizes {
        let opened = browse::run(&[&tb, "new-tab", "--browser", &key, url], Duration::from_secs(10)).filter(|o| o.ok());
        let tab = opened.and_then(|o| serde_json::from_str::<Value>(&o.stdout).ok()).map(|v| v["openedTab"].clone()).filter(|t| !t.is_null() && t != "" && t != 0);
        let Some(tab) = tab else { fail(&format!("could not open a tab for {size}")) };
        let tab = match tab {
            Value::String(s) => s,
            other => other.to_string(),
        };
        let path = out.join(format!("{size}.png"));
        let taken = shoot(&tb, &key, &tab, size, &path, full);
        run_action(&tb, &key, &tab, &["tab", "close"], Duration::from_secs(10));
        if let Err(err) = taken {
            fail(&err);
        }
        shots.push((size, Value::from(path.to_string_lossy().into_owned())));
    }
    browse::run(&[&tb, "action", "--browser", &key, "done"], Duration::from_secs(10));
    println!("{}", dumps(&shots));
}

/// Sizes the tab, reloads it at that size and takes its screenshot.
fn shoot(tb: &str, key: &str, tab: &str, size: &str, path: &std::path::Path, full: bool) -> Result<(), String> {
    // A new tab only answers once its page has a target.
    wait_for(|| run_action(tb, key, tab, &["get", "url"], Duration::from_secs(10)).filter(|o| o.ok()));
    let device = SIZES.iter().find(|(name, _)| *name == size).map(|(_, command)| *command).unwrap_or_default();
    action(tb, key, tab, device, Duration::from_secs(30))?;
    action(tb, key, tab, &["reload"], Duration::from_secs(30))?;
    action(tb, key, tab, &["wait", "--load", "networkidle"], Duration::from_secs(60))?;
    let path = path.to_string_lossy();
    let mut command = vec!["screenshot", &path];
    if full {
        command.push("--full");
    }
    action(tb, key, tab, &command, Duration::from_secs(60))
}

/// The first answer of up to POLLS polls, POLL apart.
fn wait_for<T>(mut poll: impl FnMut() -> Option<T>) -> Option<T> {
    for i in 0..POLLS {
        if let Some(found) = poll() {
            return Some(found);
        }
        if i + 1 < POLLS {
            std::thread::sleep(POLL);
        }
    }
    None
}
