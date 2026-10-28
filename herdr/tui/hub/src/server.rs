//! The socket: one JSON request per line, one JSON answer per line, each connection served in its own thread. A
//! `subscribe` request turns its connection into a stream of the store's documents.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use cockpit_core::files;
use serde_json::{Value, json};

use crate::{act, feed, fetch, go, history, search, store};

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Answers the connection's requests until it closes, noting each one's time for the idle exit.
pub fn serve(stream: UnixStream, last: Arc<AtomicU64>) {
    // The feed runs while the hub does, from its first request on.
    feed::start();
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        last.store(now_secs(), Ordering::SeqCst);
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(request) if request["op"] == "subscribe" => return subscribe(writer, &request),
            Ok(request) => handle(&request),
            Err(err) => json!({"error": format!("not JSON: {err}")}),
        };
        if writeln!(writer, "{answer}").and_then(|_| writer.flush()).is_err() {
            return;
        }
        last.store(now_secs(), Ordering::SeqCst);
    }
}

/// Writes the documents asked for, then each new version, until the other end goes; an empty line every half minute
/// finds out when it did.
fn subscribe(mut writer: UnixStream, request: &Value) {
    let names: Vec<String> =
        request["docs"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|n| store::is_doc(n)).map(str::to_string).collect();
    let (tx, rx) = channel();
    store::global().subscribe(names, tx);
    loop {
        let line = match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => String::new(),
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if writeln!(writer, "{line}").and_then(|_| writer.flush()).is_err() {
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
            act::act(field("app"), field("action"), args, armed, env)
        }
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
