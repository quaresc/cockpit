//! The socket: one JSON request per line, one JSON answer per line, each connection served in its own thread. A
//! `subscribe` request turns its connection into a stream of the store's documents.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use cockpit_core::files;
use cockpit_core::frontend::{self, Frontend};
use serde_json::{Value, json};

use crate::{act, feed, fetch, fetcher, go, history, names, outbox, search, store, views};

/// A connection the hub answers on: its unix socket, or TCP on 127.0.0.1 for the Herdr app's mod.
pub trait Conn: Read + Write + Send + Sized + 'static {
    fn dup(&self) -> std::io::Result<Self>;
    /// How long a write may block before it fails; None waits forever.
    fn write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
    /// How long a read may wait before it fails; None waits forever.
    fn read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
    /// Ends the connection at once for every handle on it, waking a read or write blocked there.
    fn abort(&self);
}

impl Conn for UnixStream {
    fn dup(&self) -> std::io::Result<Self> {
        self.try_clone()
    }

    fn write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_write_timeout(timeout)
    }

    fn read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(timeout)
    }

    fn abort(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
}

impl Conn for TcpStream {
    fn dup(&self) -> std::io::Result<Self> {
        self.try_clone()
    }

    fn write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_write_timeout(timeout)
    }

    fn read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(timeout)
    }

    /// With a zero linger the close sends a reset: a FIN would wait behind the unsent lines of a peer that stopped
    /// reading, and that peer would keep the connection open forever.
    fn abort(&self) {
        let linger = libc::linger { l_onoff: 1, l_linger: 0 };
        // SAFETY: setsockopt only reads `linger`, whose size it is given, on this stream's own descriptor.
        unsafe {
            libc::setsockopt(
                self.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_LINGER,
                (&linger as *const libc::linger).cast(),
                std::mem::size_of::<libc::linger>() as libc::socklen_t,
            )
        };
        let _ = self.shutdown(Shutdown::Both);
    }
}

/// What a connection off this machine may ask: the ops that only read, and the subscription stream. The mod keeps the
/// same list, so a remote desk says so without a round trip.
const READS: [&str; 8] = ["ping", "view", "search", "fetch", "events", "history", "shown", "subscribe"];

/// Who may ask: anyone on the unix socket (its folder is the user's), on TCP only a request carrying hub.json's token,
/// whose frontend is the Herdr app unless it says otherwise. A hub listening off loopback reads only.
#[derive(Clone)]
pub struct Gate {
    pub token: Option<String>,
    pub frontend: Option<Frontend>,
    pub read_only: bool,
}

/// Whether this gate turns the op away: off loopback, everything that is not a read.
fn refused(gate: &Gate, op: &str) -> bool {
    gate.read_only && !READS.contains(&op)
}

/// How long a subscriber's connection may refuse a line before the hub lets it go; COCKPIT_HUB_WRITE_SECS shortens it
/// for tests.
fn write_limit() -> Duration {
    Duration::from_secs(std::env::var("COCKPIT_HUB_WRITE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(60))
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Answers the connection's requests until it closes, noting each one's time for the idle exit.
pub fn serve<C: Conn>(stream: C, last: Arc<AtomicU64>, gate: Gate) {
    // The feed runs while the hub does, from its first request on.
    feed::start();
    fetcher::start();
    names::start();
    let Ok(mut writer) = stream.dup() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if !matches!(reader.read_line(&mut line), Ok(n) if n > 0) {
            return;
        }
        if line.trim().is_empty() {
            continue;
        }
        last.store(now_secs(), Ordering::SeqCst);
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(request) if gate.token.as_ref().is_some_and(|t| request["token"].as_str() != Some(t)) => {
                let _ = writeln!(writer, "{}", json!({"error": "a request on TCP needs hub.json's token"}));
                return;
            }
            Ok(request) if refused(&gate, request["op"].as_str().unwrap_or_default()) => {
                json!({"error": "this cockpit hub serves reads only off loopback: start it without COCKPIT_HUB_LISTEN to write from this machine"})
            }
            Ok(request) if request["op"] == "subscribe" => return subscribe(writer, reader, &request),
            Ok(request) => {
                let asked = request["frontend"].as_str().and_then(Frontend::parse).or(gate.frontend);
                frontend::with(asked, || handle(&request))
            }
            Err(err) => json!({"error": format!("not JSON: {err}")}),
        };
        if writeln!(writer, "{answer}").and_then(|_| writer.flush()).is_err() {
            return;
        }
        last.store(now_secs(), Ordering::SeqCst);
    }
}

/// Writes the documents and views asked for, then each new version, until the other end goes; an empty line every half
/// minute finds out when it did. A connection whose write blocks past write_limit, or that falls a budget behind (see
/// outbox), is ended, and so is one that promised a line every `keepalive` seconds and sent none for three of them (a
/// Herdr mod whose copy was reloaded away keeps its socket open without reading or writing). The app reconnects and
/// starts again from the current versions.
fn subscribe<C: Conn>(mut writer: C, mut reader: BufReader<C>, request: &Value) {
    let quiet = request["keepalive"].as_u64().map(|secs| Duration::from_secs(secs.clamp(1, 300) * 3));
    if writer.write_timeout(Some(write_limit())).and_then(|_| reader.get_ref().read_timeout(quiet)).is_err() {
        return;
    }
    let names: Vec<String> =
        request["docs"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|n| store::is_doc(n)).map(str::to_string).collect();
    let (tx, rx) = outbox::channel();
    if !names.is_empty() {
        store::global().subscribe(names, tx.clone());
    }
    views::subscribe(request, tx.clone());
    // What the other end sends is only a sign of life; the read ends when it closes, falls quiet or the writer is done.
    let gone = Arc::new(AtomicBool::new(false));
    std::thread::spawn({
        let gone = gone.clone();
        move || {
            let mut buf = [0; 512];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Err(e) if e.kind() != std::io::ErrorKind::Interrupted => break,
                    _ => {}
                }
            }
            gone.store(true, Ordering::Release);
            reader.get_ref().abort();
            let _ = tx.send(String::new());
        }
    });
    loop {
        let line = match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => String::new(),
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if gone.load(Ordering::Acquire) || writeln!(writer, "{line}").and_then(|_| writer.flush()).is_err() {
            break;
        }
    }
    writer.abort();
    views::prune();
}

/// Claude Code's answer to an MCP call from a session that has none bound yet (still starting, or a background job):
/// not news about the source, so it never reaches a document.
fn not_ready(error: &Value) -> bool {
    error.as_str().is_some_and(|e| e.contains("is not available in this mode"))
}

pub fn handle(request: &Value) -> Value {
    let field = |name: &str| request[name].as_str().unwrap_or_default();
    match field("op") {
        "ping" => json!({"ok": true}),
        "act" => {
            let args = request["args"].as_array().cloned().unwrap_or_default();
            let armed = request["armed"].as_str().map(str::to_string);
            let env: HashMap<String, String> =
                request["env"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
            let answer = act::act(field("app"), field("action"), args.clone(), armed, env);
            match answer["unknown"] == true {
                true => views::act(field("app"), field("action"), &args).unwrap_or(answer),
                false => answer,
            }
        }
        "view" => json!({"value": views::build(field("name"), &request["args"])}),
        "notify" => json!({"delivered": views::notify(&request["notification"])}),
        "shown" => json!({"args": views::shown(field("view"))}),
        "search" => json!({"entries": search::search(field("query"))}),
        "go" => go::go(&request["target"]),
        "fetch" => fetch::fetch(field("kind"), &request["args"]),
        "history" => {
            json!({"values": history::series(&files::cache(), field("kind"), field("key"))})
        }
        "events" => {
            let (since, limit) = (request["since"].as_u64().unwrap_or(0), request["limit"].as_u64().unwrap_or(500) as usize);
            let events = store::global().events(since, limit);
            let app = field("app");
            json!({"events": events.into_iter().filter(|e| app.is_empty() || e["app"] == app).collect::<Vec<_>>()})
        }
        op if op.starts_with("store.") => store_op(op, request),
        other => json!({"error": format!("unknown op {other:?}")}),
    }
}

/// The store's requests: `get`, `put` {value}, `patch` {fields} and `past` {since} on a document; `lease` {holder,
/// ttl, renew} and `release` {holder} on a lease by name.
fn store_op(op: &str, request: &Value) -> Value {
    let name = request["name"].as_str().unwrap_or_default();
    let holder = request["holder"].as_str().unwrap_or_default();
    match op {
        "store.lease" if !name.is_empty() && !holder.is_empty() => {
            let ttl = Duration::from_secs(request["ttl"].as_u64().unwrap_or(120).min(600));
            json!({"granted": store::global().lease(name, holder, ttl, request["renew"] == true)})
        }
        "store.release" => {
            store::global().release(name, holder);
            json!({"ok": true})
        }
        "store.lease" => json!({"error": "a lease needs a name and a holder"}),
        _ if !store::is_doc(name) => {
            json!({"error": format!("no document {name:?}: one of {}", store::DOCS.join(", "))})
        }
        "store.get" => json!({"value": store::global().get(name)}),
        "store.past" => {
            json!({"versions": store::global().past(name, request["since"].as_u64().unwrap_or(0))})
        }
        "store.put" => {
            // A session that cannot call MCP yet must not write that over the last good document.
            if not_ready(&request["value"]["error"]) {
                return json!({"value": store::global().get(name)});
            }
            json!({"value": store::global().update(name, |_| request["value"].clone()).1})
        }
        "store.patch" => {
            let Some(fields) = request["fields"].as_object() else {
                return json!({"error": "a patch needs its fields as an object"});
            };
            let mut fields = fields.clone();
            if not_ready(&fields.get("error").cloned().unwrap_or(Value::Null)) {
                fields.remove("error");
            }
            if let Some(Value::Array(errors)) = fields.get_mut("errors") {
                errors.retain(|e| !not_ready(e));
            }
            let fields = &fields;
            let merge = |doc: &Value| {
                if name == "inbox" { feed::merge::mod_inbox(doc, fields) } else { store::patch(doc, fields) }
            };
            json!({"value": store::global().update(name, merge).1})
        }
        other => json!({"error": format!("unknown op {other:?}")}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(read_only: bool) -> Gate {
        Gate { token: None, frontend: None, read_only }
    }

    #[test]
    fn a_read_only_gate_turns_away_everything_that_writes() {
        for op in ["ping", "view", "search", "fetch", "events", "history", "shown", "subscribe"] {
            assert!(!refused(&gate(true), op), "{op} only reads");
        }
        for op in ["act", "go", "notify", "store.put", "store.patch", "store.lease", "store.release", "dance", ""] {
            assert!(refused(&gate(true), op), "{op} must not reach a hub read off loopback");
            assert!(!refused(&gate(false), op), "{op} belongs to a hub on this machine");
        }
    }
}
