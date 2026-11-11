//! `cockpit session`: drives an agent over its streaming protocol from a shell, for working on the chat without the
//! window. A session lives as long as the command does.

use std::io::{BufRead, Write};
use std::time::Duration;

use crate::proc::fail;
use cockpit_core::hub;
use serde_json::{Value, json};

const USAGE: &str = "An agent cockpit drives itself, over its streaming protocol.

    cockpit session run <agent> <cwd> <prompt>...   starts it, sends the prompt, prints what comes back, ends it
    cockpit session host <key>                      the permission host of one session, as an MCP server on stdio
    cockpit session modes                           the permission modes an agent takes";

pub fn main(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("host") => host(args.get(1).map(String::as_str).unwrap_or_default()),
        Some("answer") => answer(&args[1..]),
        Some("modes") => println!("acceptEdits auto bypassPermissions manual dontAsk plan"),
        _ => fail(USAGE),
    }
}

/// A hub action, as the note it answers with; the sessions live in the hub, not here.
fn act(action: &str, arg: Value) -> Result<String, String> {
    let call = json!({"op": "act", "app": "session", "action": action, "args": [arg]});
    let answer = hub::request(&call, Duration::from_secs(20))?;
    match answer["error"].as_str().filter(|e| !e.is_empty()) {
        Some(why) => Err(why.to_string()),
        None => Ok(answer["note"].as_str().unwrap_or_default().to_string()),
    }
}

/// The session view, for watching one from a shell.
fn look(key: &str) -> Value {
    let call = json!({"op": "view", "name": "session", "args": {"key": key}});
    hub::request(&call, Duration::from_secs(10)).unwrap_or_default()["value"]["session"].clone()
}

fn run(args: &[String]) {
    let (Some(agent), Some(cwd)) = (args.first(), args.get(1)) else { fail(USAGE) };
    let prompt = args[2..].join(" ");
    if prompt.is_empty() {
        fail(USAGE);
    }
    let started = match act("start", json!({"agent": agent, "cwd": cwd, "mode": "default"})) {
        Ok(note) => note,
        Err(why) => fail(&why),
    };
    let key = started.trim_start_matches("started ").to_string();
    println!("{key}");
    if let Err(why) = act("send", json!({"key": key, "text": prompt})) {
        let _ = act("stop", json!({"key": key}));
        fail(&why);
    }
    for _ in 0..900 {
        std::thread::sleep(Duration::from_millis(200));
        let now = look(&key);
        for ask in now["pending"].as_array().into_iter().flatten() {
            println!("ASKS {}", serde_json::to_string(ask).unwrap_or_default());
        }
        if now["working"] == false && now["messages"].as_array().is_some_and(|m| m.len() > 1) {
            break;
        }
    }
    println!("{}", serde_json::to_string_pretty(&look(&key)).unwrap_or_default());
    let _ = act("stop", json!({"key": key}));
}

/// `cockpit session answer <id> allow|deny` — what the card does in the window.
fn answer(args: &[String]) {
    let (Some(id), verdict) = (args.first(), args.get(1).map(String::as_str).unwrap_or("allow")) else { fail(USAGE) };
    match act("answer", json!({"id": id, "allow": verdict == "allow"})) {
        Ok(note) => println!("{note}"),
        Err(why) => fail(&why),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The permission host

/// The one tool the host offers. The agent calls it before anything that would ask, and waits for the answer.
fn tool() -> Value {
    json!({
        "name": "permission",
        "description": "Ask the person whether a tool may run.",
        "inputSchema": {"type": "object", "properties": {
            "tool_name": {"type": "string"}, "input": {"type": "object"}, "tool_use_id": {"type": "string"}},
            "required": ["tool_name", "input"]},
    })
}

/// A line in the session's own log, so an answer nobody remembers giving can be traced to where it came from.
fn note(key: &str, line: &str) {
    let path = cockpit_core::files::cache().join("sessions").join(format!("{key}.log"));
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{} {line}", chrono::Utc::now().to_rfc3339());
    }
}

/// A hub action from the host, as the note it answers with.
fn ask_hub(action: &str, arg: Value) -> String {
    let call = json!({"op": "act", "app": "session", "action": action, "args": [arg]});
    let answer = hub::request(&call, Duration::from_secs(10)).unwrap_or_default();
    answer["note"].as_str().unwrap_or_default().to_string()
}

/// An MCP server on stdio holding one session's permission prompts, so the person answers them in the chat.
fn host(key: &str) {
    if key.is_empty() {
        fail("cockpit session host <key>: which session?");
    }
    let out = std::io::stdout();
    let send = |value: Value| {
        let mut held = out.lock();
        let _ = writeln!(held, "{value}");
        let _ = held.flush();
    };
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let Ok(request) = serde_json::from_str::<Value>(&line) else { continue };
        let id = request["id"].clone();
        match request["method"].as_str().unwrap_or_default() {
            "initialize" => send(json!({"jsonrpc": "2.0", "id": id, "result": {
                "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                "serverInfo": {"name": "cockpit-permission", "version": env!("CARGO_PKG_VERSION")}}})),
            "tools/list" => send(json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [tool()]}})),
            "tools/call" => {
                let args = request["params"]["arguments"].clone();
                let mut arg = args.clone();
                arg["key"] = json!(key);
                let asked = ask_hub("ask", arg);
                note(key, &format!("asked {asked} for {}", args["tool_name"].as_str().unwrap_or("a tool")));
                // The person may be away from the window: the agent waits rather than being told no.
                let mut answer = String::new();
                while answer.is_empty() {
                    std::thread::sleep(Duration::from_millis(300));
                    answer = ask_hub("decision", json!({"id": asked}));
                }
                note(key, &format!("{asked} answered {answer}"));
                send(json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": answer}]}}));
            }
            _ if !id.is_null() => send(json!({"jsonrpc": "2.0", "id": id, "result": {}})),
            _ => {}
        }
    }
}
