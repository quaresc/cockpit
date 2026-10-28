//! `cockpit store`: how the mod reads and writes the documents the hub keeps for everyone, and takes the leases that
//! let one Claude session at a time do a job for all of them. Starts the hub when none runs.

use std::time::Duration;

use serde_json::{Value, json};

use crate::proc::fail;

const USAGE: &str = "The documents the hub keeps (inbox, ci, meetings, brief, monitor) and its leases.

    cockpit store get <doc>                       the document as JSON, null before anyone wrote it
    cockpit store put <doc> <file>                the JSON in the file becomes the document; the file goes, the document is printed
    cockpit store patch <doc> <file>              the file's fields are set on the document; the file goes, the document is printed
    cockpit store lease <name> <holder> [ttl]     prints granted or held; the lease lasts ttl seconds (120 by default)
    cockpit store renew <name> <holder> [ttl]     extends the holder's lease, never taking one it lost; granted or held
    cockpit store release <name> <holder>         the holder's lease goes; prints released";

fn ask(request: Value) -> Value {
    cockpit_core::hub::request(&request, Duration::from_secs(15)).unwrap_or_else(|error| fail(&format!("cockpit store: {error}")))
}

/// The JSON in the file, which goes once read: the mod writes one per call.
fn take(path: &str) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|error| fail(&format!("cockpit store: {path}: {error}")));
    let _ = std::fs::remove_file(path);
    serde_json::from_str(&text).unwrap_or_else(|error| fail(&format!("cockpit store: {path}: {error}")))
}

fn lease(name: &str, holder: &str, ttl: Option<&String>, renew: bool) {
    let ttl = ttl.map(|t| t.parse::<u64>().unwrap_or_else(|_| fail(USAGE))).unwrap_or(120);
    let answer = ask(json!({"op": "store.lease", "name": name, "holder": holder, "ttl": ttl, "renew": renew}));
    println!("{}", if answer["granted"] == true { "granted" } else { "held" });
}

pub fn main(args: &[String]) {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let answer = match args.as_slice() {
        ["get", doc] => ask(json!({"op": "store.get", "name": doc})),
        ["put", doc, file] => ask(json!({"op": "store.put", "name": doc, "value": take(file)})),
        ["patch", doc, file] => ask(json!({"op": "store.patch", "name": doc, "fields": take(file)})),
        [verb @ ("lease" | "renew"), name, holder, rest @ ..] if rest.len() <= 1 => {
            return lease(name, holder, rest.first().map(|t| t.to_string()).as_ref(), *verb == "renew");
        }
        ["release", name, holder] => {
            ask(json!({"op": "store.release", "name": name, "holder": holder}));
            return println!("released");
        }
        _ => fail(USAGE),
    };
    println!("{}", answer["value"]);
}
