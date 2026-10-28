//! The hub: one background process (`cockpit-hub`) every app talks to over a unix socket. It runs the actions, searches
//! across the apps, brings a found item into view, fetches details on demand and keeps history for sparklines; the first
//! app that needs it starts it.
//!
//! One JSON request per line, one JSON answer per line. Requests, by `op`:
//! - `act` {app, action, args, armed}: an action, answered like `bridge::Reply` {note, pending, refreshing, error}.
//! - `search` {query}: {entries: [{app, kind, title, help, target}]} across inbox, runbook, show, to-do and meetings.
//! - `go` {target}: focuses the target's space and asks its app to bring it into view (see `focus`).
//! - `fetch` {kind, args}: details on demand, cached: `ci_log`, `failed_tests`, `review_threads`, `pr_files`.
//! - `history` {kind, key}: {values: [...]}, oldest first: `ci_duration` (repo/branch), `ticket_cost` (ticket id).
//! - `ping`: {ok: true}.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::files;

pub fn socket() -> PathBuf {
    files::cache().join("hub.sock")
}

/// The hub binary, beside the app's own.
fn binary() -> PathBuf {
    std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("cockpit-hub"))).unwrap_or_else(|| "cockpit-hub".into())
}

fn connect(timeout: Duration) -> Option<UnixStream> {
    if let Ok(stream) = UnixStream::connect(socket()) {
        return Some(stream);
    }
    // No hub yet: start one, detached, at most every few seconds, and wait for its socket.
    static STARTED: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    {
        let mut started = STARTED.lock().ok()?;
        if started.is_some_and(|at| at.elapsed() < Duration::from_secs(5)) {
            return None;
        }
        *started = Some(Instant::now());
    }
    let _ = std::process::Command::new(binary())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let start = Instant::now();
    while start.elapsed() < timeout.min(Duration::from_secs(2)) {
        std::thread::sleep(Duration::from_millis(25));
        if let Ok(stream) = UnixStream::connect(socket()) {
            return Some(stream);
        }
    }
    None
}

/// One request and its answer; an error when the hub cannot be reached or answers with `error`.
pub fn request(req: &Value, timeout: Duration) -> Result<Value, String> {
    let mut stream = connect(timeout).ok_or("the cockpit hub did not start")?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    writeln!(stream, "{req}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let answer: Value = serde_json::from_str(&line).map_err(|_| "the cockpit hub sent no answer".to_string())?;
    match answer.get("error").and_then(Value::as_str) {
        Some(error) if answer.get("note").is_none() => Err(error.to_string()),
        _ => Ok(answer),
    }
}

#[derive(Clone, Debug)]
pub struct Found {
    pub app: String,
    pub kind: String,
    pub title: String,
    pub help: String,
    pub target: Value,
}

/// Everything across the apps matching the query, best first; empty when the hub is not there.
pub fn search(query: &str) -> Vec<Found> {
    let Ok(answer) = request(&json!({"op": "search", "query": query}), Duration::from_millis(400)) else { return Vec::new() };
    answer["entries"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|e| Found {
                    app: e["app"].as_str().unwrap_or_default().into(),
                    kind: e["kind"].as_str().unwrap_or_default().into(),
                    title: e["title"].as_str().unwrap_or_default().into(),
                    help: e["help"].as_str().unwrap_or_default().into(),
                    target: e["target"].clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Brings a found item into view, in its own space.
pub fn go(target: &Value) {
    let target = target.clone();
    std::thread::spawn(move || {
        let _ = request(&json!({"op": "go", "target": target}), Duration::from_secs(5));
    });
}

/// Details on demand: blocking, so call it from a background thread.
pub fn fetch(kind: &str, args: Value) -> Result<Value, String> {
    request(&json!({"op": "fetch", "kind": kind, "args": args}), Duration::from_secs(30))
}

/// A series for a sparkline, oldest first.
pub fn history(kind: &str, key: &str) -> Vec<f64> {
    request(&json!({"op": "history", "kind": kind, "key": key}), Duration::from_millis(300))
        .ok()
        .and_then(|a| a["values"].as_array().map(|v| v.iter().filter_map(Value::as_f64).collect()))
        .unwrap_or_default()
}
