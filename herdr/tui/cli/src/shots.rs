//! `cockpit shots`: screenshots of a page at phone, tablet and desktop sizes, taken in the side browser beside a pane
//! (terminal-browser, or cockpit's Chromium window). Each size gets a tab of its own, closed afterwards, so the
//! person's tabs keep their size.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::browse::{DEFAULT_SIZE, Opened, Tb, browsers_at, open_at, place_of, tb_at, terminal_browser};
use crate::proc::{dumps, fail};

const USAGE: &str = "Screenshots of a page at phone, tablet and desktop sizes, taken in the terminal-browser beside a pane.

    cockpit shots <url> <out dir> [--sizes mobile,tablet,desktop] [--full] [--session <session id>]

Prints the screenshots as JSON, {size: path}. Without terminal-browser beside a pane in herdr or the Herdr app, it exits 2 and says so.";

const SIZES: [(&str, &[&str]); 3] =
    [("mobile", &["set", "device", "iPhone 16"]), ("tablet", &["set", "device", "iPad"]), ("desktop", &["set", "viewport", "1440", "900"])];

/// How long to wait between polls of a browser starting or a tab getting its page, and how many polls at most.
const POLL: Duration = Duration::from_millis(500);
const POLLS: usize = 40;

fn run_action(tb: &Tb, key: &str, tab: &str, command: &[&str], timeout: Duration) -> Result<crate::proc::Out, String> {
    let mut args = vec!["action", "--browser", key, "--tab", tab, "--"];
    args.extend_from_slice(command);
    tb.run(&args, timeout)
}

/// One browser action that must succeed; its error names the command and what the browser said.
fn action(tb: &Tb, key: &str, tab: &str, command: &[&str], timeout: Duration) -> Result<(), String> {
    match run_action(tb, key, tab, command, timeout) {
        Ok(out) if out.ok() => Ok(()),
        done => {
            let said = match done {
                Ok(out) => {
                    let text = if out.stderr.is_empty() { out.stdout } else { out.stderr };
                    text.trim().chars().take(200).collect()
                }
                Err(why) => why,
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
    let session = session.as_deref();
    if cockpit_core::browser::engine() == cockpit_core::browser::Engine::Chromium {
        return chromium_shots(url, &out, &sizes, full, session);
    }
    let place = place_of(session).unwrap_or_else(|why| fail(&why));
    let Some(bin) = terminal_browser().filter(|_| place.is_placed()) else {
        eprintln!("terminal-browser is not beside a pane in herdr or the Herdr app: take these screenshots in Chrome instead");
        std::process::exit(2);
    };
    let tb = tb_at(bin, &place);
    let beside = browsers_at(&tb, &place).unwrap_or_else(|why| fail(&why));
    if beside.is_empty()
        && let Opened::No(why) = open_at(&tb, &place, url, DEFAULT_SIZE, false).unwrap_or_else(|why| fail(&why))
    {
        fail(&format!("could not open terminal-browser beside the pane: {why}"));
    }
    // A browser just split in takes a moment to start, and lists nothing meanwhile.
    let browsers =
        wait_for(|| browsers_at(&tb, &place).ok().filter(|b| !b.is_empty())).unwrap_or_else(|| fail("terminal-browser did not start beside the pane"));
    let key = match &browsers[0]["key"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if let Err(err) = std::fs::create_dir_all(&out) {
        fail(&format!("{}: {err}", out.display()));
    }
    let mut shots = Vec::new();
    for size in sizes {
        let opened = tb.run(&["new-tab", "--browser", &key, url], Duration::from_secs(10)).ok().filter(|o| o.ok());
        let tab =
            opened.and_then(|o| serde_json::from_str::<Value>(&o.stdout).ok()).map(|v| v["openedTab"].clone()).filter(|t| !t.is_null() && t != "" && t != 0);
        let Some(tab) = tab else { fail(&format!("could not open a tab for {size}")) };
        let tab = match tab {
            Value::String(s) => s,
            other => other.to_string(),
        };
        let path = out.join(format!("{size}.png"));
        let taken = shoot(&tb, &key, &tab, size, &path, full);
        let _ = run_action(&tb, &key, &tab, &["tab", "close"], Duration::from_secs(10));
        if let Err(err) = taken {
            fail(&err);
        }
        shots.push((size, Value::from(path.to_string_lossy().into_owned())));
    }
    let _ = tb.run(&["action", "--browser", &key, "done"], Duration::from_secs(10));
    println!("{}", dumps(&shots));
}

/// The screenshots taken in cockpit's Chromium window: a background tab per size in the session's browser, closed after.
fn chromium_shots(url: &str, out: &std::path::Path, sizes: &[&str], full: bool, session: Option<&str>) {
    use cockpit_core::browser;
    use serde_json::json;

    use crate::browser::{agent, side};
    let owner = side::owner(session).unwrap_or_else(|why| fail(&why));
    let ask = |request: Value| browser::request(&request, Duration::from_secs(20));
    if browser::connect().is_none()
        && let Err(why) = side::open(url, session, false)
    {
        fail(&format!("could not open the side browser: {why}"));
    }
    if let Err(err) = std::fs::create_dir_all(out) {
        fail(&format!("{}: {err}", out.display()));
    }
    let mut shots = Vec::new();
    for size in sizes {
        let opened = ask(json!({"op": "open", "owner": owner, "url": url, "background": true, "fresh": true}))
            .unwrap_or_else(|why| fail(&format!("could not open a tab for {size}: {why}")));
        let tab = opened["tab"].as_str().unwrap_or_default().to_string();
        let path = out.join(format!("{size}.png"));
        let taken = agent::pinned(&owner, Some(&tab), "-shots").and_then(|pinned| {
            let step = |command: &[&str], secs: u64| match pinned.run(command, Duration::from_secs(secs)) {
                Ok(done) if done.ok() => Ok(()),
                Ok(done) => Err(format!(
                    "{}: {}",
                    command[0],
                    if done.stderr.is_empty() { done.stdout } else { done.stderr }.trim().chars().take(200).collect::<String>()
                )),
                Err(why) => Err(format!("{}: {why}", command[0])),
            };
            let device = SIZES.iter().find(|(name, _)| name == size).map(|(_, command)| *command).unwrap_or_default();
            step(device, 30)?;
            step(&["reload"], 30)?;
            step(&["wait", "--load", "networkidle"], 60)?;
            let file = path.to_string_lossy();
            let mut command = vec!["screenshot", &file];
            if full {
                command.push("--full");
            }
            step(&command, 60)
        });
        let _ = ask(json!({"op": "tab.close", "owner": owner, "tab": tab}));
        if let Err(err) = taken {
            fail(&err);
        }
        shots.push((*size, Value::from(path.to_string_lossy().into_owned())));
    }
    println!("{}", dumps(&shots));
}

/// Sizes the tab, reloads it at that size and takes its screenshot.
fn shoot(tb: &Tb, key: &str, tab: &str, size: &str, path: &std::path::Path, full: bool) -> Result<(), String> {
    // A new tab only answers once its page has a target.
    wait_for(|| run_action(tb, key, tab, &["get", "url"], Duration::from_secs(10)).ok().filter(|o| o.ok()));
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
