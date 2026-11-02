//! The socket: one JSON request per line, one JSON answer per line, each connection served in its own thread. A
//! `subscribe` request turns its connection into a stream of the store's documents.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use cockpit_core::files;
use cockpit_core::frontend::{self, Frontend};
use serde_json::{Value, json};

use crate::{act, feed, fetch, go, history, search, store, views};

/// A connection the hub answers on: its unix socket, or TCP on 127.0.0.1 for the Herdr app's mod.
pub trait Conn: Read + Write + Send + Sized + 'static {
    fn dup(&self) -> std::io::Result<Self>;
}

impl Conn for UnixStream {
    fn dup(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
}

impl Conn for TcpStream {
    fn dup(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
}

/// Who may ask: anyone on the unix socket (its folder is the user's), on TCP only a request carrying hub.json's token,
/// whose frontend is the Herdr app unless it says otherwise.
#[derive(Clone)]
pub struct Gate {
    pub token: Option<String>,
    pub frontend: Option<Frontend>,
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Answers the connection's requests until it closes, noting each one's time for the idle exit.
pub fn serve<C: Conn>(stream: C, last: Arc<AtomicU64>, gate: Gate) {
    // The feed runs while the hub does, from its first request on.
    feed::start();
    let Ok(mut writer) = stream.dup() else {
        return;
    };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        last.store(now_secs(), Ordering::SeqCst);
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(request) if gate.token.as_ref().is_some_and(|t| request["token"].as_str() != Some(t)) => {
                let _ = writeln!(writer, "{}", json!({"error": "a request on TCP needs hub.json's token"}));
                return;
            }
            Ok(request) if request["op"] == "subscribe" => return subscribe(writer, &request),
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
/// minute finds out when it did.
fn subscribe<C: Conn>(mut writer: C, request: &Value) {
    let names: Vec<String> =
        request["docs"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|n| store::is_doc(n)).map(str::to_string).collect();
    let (tx, rx) = channel();
    if !names.is_empty() {
        store::global().subscribe(names, tx.clone());
    }
    views::subscribe(request, tx);
    loop {
        let line = match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => String::new(),
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if writeln!(writer, "{line}").and_then(|_| writer.flush()).is_err() {
            views::prune();
            return;
        }
    }
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
            json!({"value": store::global().update(name, |_| request["value"].clone()).1})
        }
        "store.patch" => {
            let Some(fields) = request["fields"].as_object() else {
                return json!({"error": "a patch needs its fields as an object"});
            };
            let merge = |doc: &Value| {
                if name == "inbox" { feed::merge::mod_inbox(doc, fields) } else { store::patch(doc, fields) }
            };
            json!({"value": store::global().update(name, merge).1})
        }
        other => json!({"error": format!("unknown op {other:?}")}),
    }
}
