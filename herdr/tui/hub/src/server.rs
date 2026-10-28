//! The socket: one JSON request per line, one JSON answer per line, each connection served in its own thread.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cockpit_core::files;
use serde_json::{Value, json};

use crate::{act, fetch, go, history, search};

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Answers the connection's requests until it closes, noting each one's time for the idle exit.
pub fn serve(stream: UnixStream, last: Arc<AtomicU64>) {
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
            Ok(request) => handle(&request),
            Err(err) => json!({"error": format!("not JSON: {err}")}),
        };
        if writeln!(writer, "{answer}").and_then(|_| writer.flush()).is_err() {
            return;
        }
        last.store(now_secs(), Ordering::SeqCst);
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
        other => json!({"error": format!("unknown op {other:?}")}),
    }
}
