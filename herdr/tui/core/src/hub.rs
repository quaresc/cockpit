//! The hub: one background process (`cockpit hub`) every app talks to over a unix socket. It runs the actions, searches
//! across the apps, brings a found item into view, fetches details on demand and keeps history for sparklines; the first
//! app that needs it starts it.
//!
//! One JSON request per line, one JSON answer per line. Requests, by `op`:
//! - `act` {app, action, args, armed}: an action, answered like `bridge::Reply` {note, pending, refreshing, error}.
//! - `search` {query}: {entries: [{app, kind, title, help, target}]} across inbox, runbook, show, to-do and meetings.
//! - `go` {target}: focuses the target's space and asks its app to bring it into view (see `focus`).
//! - `fetch` {kind, args}: details on demand, cached: `ci_log`, `failed_tests`, `review_threads`, `pr_files`.
//! - `history` {kind, key}: {values: [...]}, oldest first: `ci_duration` (repo/branch), `ticket_cost` (ticket id).
//! - `store.get`, `store.put` {value}, `store.patch` {fields}, `store.past` {since} on a shared document by `name`
//!   (inbox, ci, meetings, brief, monitor), each answered {value} or {versions}; `store.lease` {name, holder, ttl,
//!   renew} answered {granted}, and `store.release` {name, holder}.
//! - `subscribe` {docs}: the connection then carries {doc, value} lines, one per document at once and per new version
//!   after, and an empty line every half minute.
//! - `events` {since, app, limit}: {events: [{at, doc, app, key, title}]}, newest first: what changed in the shared
//!   documents over the last week, each with the app and row key it touched.
//! - `ping`: {ok: true}.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::files;

/// ~/.cache/cockpit/hub.sock, or a name in the temp folder when that path is longer than a unix socket allows
/// (104 bytes on macOS), keyed by the cache folder so two homes never share a hub.
pub fn socket() -> PathBuf {
    let path = files::cache().join("hub.sock");
    if path.as_os_str().len() < 100 {
        return path;
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    files::cache().hash(&mut hasher);
    std::env::temp_dir().join(format!("cockpit-hub-{:016x}.sock", hasher.finish()))
}

/// The `cockpit` binary beside the app's own (the app itself, outside tests), which serves as the hub with `hub`.
fn binary() -> PathBuf {
    std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("cockpit"))).unwrap_or_else(|| "cockpit".into())
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
        .arg("hub")
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

/// What changed for this app since `since_ms` (epoch milliseconds), newest first; empty when the hub is not there.
pub fn events(app: &str, since_ms: u64) -> Vec<Value> {
    request(&json!({"op": "events", "app": app, "since": since_ms, "limit": 500}), Duration::from_millis(500))
        .ok()
        .and_then(|a| a["events"].as_array().cloned())
        .unwrap_or_default()
}

/// A series for a sparkline, oldest first.
pub fn history(kind: &str, key: &str) -> Vec<f64> {
    request(&json!({"op": "history", "kind": kind, "key": key}), Duration::from_millis(300))
        .ok()
        .and_then(|a| a["values"].as_array().map(|v| v.iter().filter_map(Value::as_f64).collect()))
        .unwrap_or_default()
}

/// The shared documents an app follows, pushed by the hub: each read from its file at first, then kept as the hub sends
/// it, over a connection made again whenever the hub restarts.
pub struct Docs {
    latest: HashMap<String, Value>,
    rx: Receiver<(String, Value)>,
    /// Counts the versions taken in, so a rebuild can tell it saw them all.
    pub revision: u64,
}

impl Docs {
    /// Takes in what the hub sent since; true when anything came.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        while let Ok((name, value)) = self.rx.try_recv() {
            if self.latest.get(&name) != Some(&value) {
                self.latest.insert(name, value);
                self.revision += 1;
                changed = true;
            }
        }
        changed
    }

    /// The document, None before anyone wrote it.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.latest.get(name).filter(|v| !v.is_null())
    }
}

/// Follows these documents (inbox, ci, meetings, brief, monitor) as long as the returned Docs lives.
pub fn subscribe(names: &[&str]) -> Docs {
    let latest = names.iter().filter_map(|n| Some((n.to_string(), files::load(&files::cache().join(format!("{n}.json")))?))).collect();
    let (tx, rx) = channel();
    let request = json!({"op": "subscribe", "docs": names});
    std::thread::spawn(move || {
        loop {
            if let Some(mut stream) = connect(Duration::from_secs(3))
                && writeln!(stream, "{request}").is_ok()
            {
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    let Ok(message) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    let Some(name) = message["doc"].as_str() else {
                        continue;
                    };
                    if tx.send((name.to_string(), message["value"].clone())).is_err() {
                        return;
                    }
                    crate::term::wake();
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
    Docs { latest, rx, revision: 0 }
}
