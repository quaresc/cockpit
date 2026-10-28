//! `cockpit browse`: opens a page in terminal-browser, split right of a pane, else in the default browser, for an
//! agent's E2E tests and browser views. terminal-browser is used when it is installed and the pane is in herdr; a page
//! already open beside the pane comes to the front instead of opening twice, and the keyboard stays where it was.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::proc::{self, Out, dumps, fail, open_url, quote};

const USAGE: &str = "Opens a page in terminal-browser, split right of a pane, else in the default browser.

    cockpit browse [--size <fraction>] [--reload] <url> [<Claude session id>]   beside the session's pane, else the calling one
    cockpit browse --beside [<Claude session id>]                               the browser beside it, as JSON ({} when none)
    cockpit browse --external [<Claude session id>]                             its page, opened in the default browser";

/// herdr sends a picture of at most 16 MiB through a file, a larger one inline through the terminal, which is far slower.
const MAX_FRAME: f64 = 16.0 * 1024.0 * 1024.0;
pub const DEFAULT_SIZE: f64 = 0.5;
const TIMEOUT: Duration = Duration::from_secs(10);

pub fn main(args: &[String]) {
    let mut args: Vec<String> = args.to_vec();
    match args.first().map(String::as_str) {
        Some("--beside") => {
            println!("{}", beside_json(&beside(args.get(1).map(String::as_str))));
            return;
        }
        Some("--external") => {
            let page = beside(args.get(1).map(String::as_str));
            let Some(page) = page else { fail("no terminal-browser beside this pane") };
            open_url(page["url"].as_str().unwrap_or_default());
            return;
        }
        _ => {}
    }
    let (mut size, mut reload) = (DEFAULT_SIZE, false);
    while args.first().is_some_and(|a| a.starts_with("--")) {
        let flag = args.remove(0);
        if flag == "--size" && !args.is_empty() {
            let value = args.remove(0);
            size = value.parse().unwrap_or_else(|_| fail(&format!("--size wants a fraction, not {value}")));
        } else if flag == "--reload" {
            reload = true;
        }
    }
    let Some(url) = args.first() else { fail(USAGE) };
    let session = args.get(1).map(String::as_str);
    let opened = terminal_browser().is_some_and(|tb| pane_of(session).is_some_and(|pane| open_beside(&tb, &pane, url, size, reload)));
    if !opened {
        open_url(&if url.starts_with('/') { format!("file://{url}") } else { url.clone() });
    }
}

pub fn run(argv: &[&str], timeout: Duration) -> Option<Out> {
    proc::run(argv, &[], timeout)
}

fn herdr_bin() -> String {
    std::env::var("HERDR_BIN_PATH").ok().filter(|p| !p.is_empty()).or_else(|| proc::which("herdr")).unwrap_or_else(|| "herdr".into())
}

fn herdr(args: &[&str]) -> Option<Value> {
    let bin = herdr_bin();
    let argv: Vec<&str> = std::iter::once(bin.as_str()).chain(args.iter().copied()).collect();
    let out = run(&argv, TIMEOUT).filter(Out::ok)?;
    serde_json::from_str::<Value>(&out.stdout).ok()?.get("result").cloned()
}

/// The terminal-browser command, or None when it is missing; COCKPIT_TERMINAL_BROWSER names the one to use instead,
/// missing when it is not an executable.
pub fn terminal_browser() -> Option<String> {
    match std::env::var("COCKPIT_TERMINAL_BROWSER") {
        Ok(path) if !path.is_empty() => proc::executable(std::path::Path::new(&path)).then_some(path),
        _ => proc::which("terminal-browser"),
    }
}

/// The pane of a Claude session, else the calling one, with its tab and space; None outside herdr.
pub fn pane_of(session: Option<&str>) -> Option<Value> {
    let listed = herdr(&["pane", "list"]).unwrap_or(Value::Null);
    let panes = listed["panes"].as_array().cloned().unwrap_or_default();
    let by_session = session.and_then(|s| panes.iter().find(|p| p["agent_session"]["value"] == s));
    let calling = std::env::var("HERDR_PANE_ID").ok().and_then(|id| panes.iter().find(|p| p["pane_id"] == id.as_str()));
    by_session.or(calling).cloned()
}

fn field(value: &Value, name: &str) -> String {
    match &value[name] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The terminal-browsers open in the pane's tab.
pub fn browsers_beside(tb: &str, pane: &Value) -> Vec<Value> {
    let (pane_id, tab_id) = (field(pane, "pane_id"), field(pane, "tab_id"));
    let out = proc::run(&[tb, "ls", "--json"], &[("HERDR_PANE_ID", &pane_id), ("HERDR_TAB_ID", &tab_id)], TIMEOUT).filter(Out::ok);
    out.and_then(|o| serde_json::from_str::<Value>(&o.stdout).ok()).and_then(|v| v["browsers"].as_array().cloned()).unwrap_or_default()
}

/// The path with its existing part's links resolved, the rest kept as written.
fn real_path(path: &Path) -> PathBuf {
    for base in path.ancestors() {
        if let Ok(real) = std::fs::canonicalize(base) {
            return real.join(path.strip_prefix(base).unwrap_or(Path::new("")));
        }
    }
    path.to_path_buf()
}

fn same_page(a: &str, b: &str) -> bool {
    let norm = |url: &str| {
        let url = if url.starts_with('/') { format!("file://{}", real_path(Path::new(url)).display()) } else { url.to_string() };
        url.trim_end_matches('/').to_string()
    };
    norm(a) == norm(b)
}

/// The tty's size in pixels, from the shell running in the pane.
fn pixels(pane: &Value) -> Option<(f64, f64)> {
    let info = herdr(&["pane", "process-info", "--pane", &field(pane, "pane_id")]).unwrap_or(Value::Null);
    let pid = &info["process_info"]["shell_pid"];
    if pid.is_null() || pid == false || pid == 0 || pid == "" {
        return None;
    }
    let pid = field(&info["process_info"], "shell_pid");
    let tty = run(&["ps", "-o", "tty=", "-p", &pid], TIMEOUT)?.stdout.trim().to_string();
    if tty.is_empty() || tty == "??" {
        return None;
    }
    let file = std::fs::File::open(format!("/dev/{tty}")).ok()?;
    let mut size = libc::winsize { ws_row: 0, ws_col: 0, ws_xpixel: 0, ws_ypixel: 0 };
    use std::os::fd::AsRawFd;
    // SAFETY: TIOCGWINSZ only writes a winsize into the struct it is given, which lives for the call.
    if unsafe { libc::ioctl(file.as_raw_fd(), libc::TIOCGWINSZ, &mut size) } != 0 || size.ws_xpixel == 0 || size.ws_ypixel == 0 {
        return None;
    }
    Some((f64::from(size.ws_xpixel), f64::from(size.ws_ypixel)))
}

/// The fraction of the pane a new split takes: as wanted, but small enough for herdr to send its frames as files.
pub fn split_size(pane: &Value, wanted: f64) -> f64 {
    let Some((width, height)) = pixels(pane) else { return wanted };
    fraction(wanted, width, height)
}

fn fraction(wanted: f64, width: f64, height: f64) -> f64 {
    let fits = MAX_FRAME / (4.0 * height) / width;
    round2(wanted.min(fits).clamp(0.2, 0.95))
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

pub fn open_beside(tb: &str, pane: &Value, url: &str, size: f64, reload: bool) -> bool {
    let browsers = browsers_beside(tb, pane);
    for browser in &browsers {
        let key = field(browser, "key");
        for tab in browser["tabs"].as_array().into_iter().flatten() {
            if same_page(&field(tab, "url"), url) {
                let id = field(tab, "id");
                let mut argv = vec![tb, "action", "--browser", &key, "--tab", &id, "--follow", "--"];
                argv.extend(if reload { &["reload"][..] } else { &["get", "url"][..] });
                return run(&argv, TIMEOUT).is_some_and(|o| o.ok());
            }
        }
    }
    if let Some(first) = browsers.first() {
        return run(&[tb, "new-tab", "--browser", &field(first, "key"), url], TIMEOUT).is_some_and(|o| o.ok());
    }
    // The split is herdr's own, so the keyboard stays where it is; its ratio is what the first pane keeps.
    let ratio = round2(1.0 - split_size(pane, size)).to_string();
    let split = herdr(&["pane", "split", "--pane", &field(pane, "pane_id"), "--direction", "right", "--ratio", &ratio, "--no-focus"]);
    let Some(browser) = split.and_then(|s| s["pane"]["pane_id"].as_str().filter(|p| !p.is_empty()).map(str::to_string)) else {
        return false;
    };
    // exec, so the pane closes with the browser.
    let command = format!("exec {} open {}", quote(tb), quote(url));
    run(&[&herdr_bin(), "pane", "run", &browser, &command], TIMEOUT).is_some_and(|o| o.ok())
}

/// The active tab of the browser beside the pane: its browser key, tab id, URL and title.
pub fn beside(session: Option<&str>) -> Option<Value> {
    let tb = terminal_browser()?;
    let pane = pane_of(session)?;
    browsers_beside(&tb, &pane).iter().find_map(|browser| {
        let tab = browser["tabs"].as_array()?.iter().find(|t| t["active"].as_bool() == Some(true))?;
        Some(json!({"key": browser["key"], "tab": tab["id"], "url": tab.get("url").cloned().unwrap_or(json!("")), "title": tab.get("title").cloned().unwrap_or(json!(""))}))
    })
}

fn beside_json(page: &Option<Value>) -> String {
    match page {
        Some(p) => dumps(&[("key", p["key"].clone()), ("tab", p["tab"].clone()), ("url", p["url"].clone()), ("title", p["title"].clone())]),
        None => "{}".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_small_enough_for_frames_sent_as_files() {
        assert_eq!(fraction(0.5, 1000.0, 1000.0), 0.5);
        assert_eq!(fraction(0.5, 4000.0, 2000.0), 0.5);
        assert_eq!(fraction(0.9, 5000.0, 2400.0), 0.35);
        assert_eq!(fraction(0.5, 20000.0, 4000.0), 0.2);
        assert_eq!(fraction(1.0, 100.0, 100.0), 0.95);
    }

    #[test]
    fn the_same_page_whatever_its_trailing_slash_or_link() {
        assert!(same_page("https://x.test/a/", "https://x.test/a"));
        assert!(!same_page("https://x.test/a", "https://x.test/b"));
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        assert!(same_page(&format!("{}/page.html", dir.path().display()), &format!("file://{}/page.html", real.display())));
    }

    #[test]
    fn beside_prints_like_before() {
        let page = json!({"key": "b1", "tab": 2, "url": "https://x.test/", "title": "Café"});
        assert_eq!(beside_json(&Some(page)), r#"{"key": "b1", "tab": 2, "url": "https://x.test/", "title": "Caf\u00e9"}"#);
        assert_eq!(beside_json(&None), "{}");
    }
}
