//! `cockpit mcp`: cockpit's agent tools (show, cockpit_search, cockpit_changes, report_step) as a stdio MCP server, in
//! newline-delimited JSON-RPC 2.0. The drawing guidance goes out as the server's instructions.

mod report;
pub mod show;
pub mod tools;

use std::io::{BufRead, Write};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{Value, json};

use crate::find;

const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// This server's session, the owner of the views it shows: answers to them come back here.
fn session() -> &'static str {
    static SESSION: OnceLock<String> = OnceLock::new();
    SESSION.get_or_init(|| format!("mcp-{}", std::process::id()))
}

fn here() -> Option<&'static show::Here> {
    static HERE: OnceLock<Option<show::Here>> = OnceLock::new();
    HERE.get_or_init(show::here).as_ref()
}

/// Once a view is shown, answers given in its pane are handed to the agent as its next prompt, every two seconds.
fn follow_answers() {
    static STARTED: OnceLock<()> = OnceLock::new();
    let Some(here) = here() else { return };
    STARTED.get_or_init(|| {
        std::thread::spawn(move || {
            loop {
                for text in show::take_answers(&show::dir(), session()) {
                    show::prompt(here, &text);
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    });
}

pub fn main(_: &[String]) {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let answers: Vec<Value> = batch.iter().filter_map(handle).collect();
                (!answers.is_empty()).then_some(Value::Array(answers))
            }
            Ok(message) => handle(&message),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(answer) = answer
            && writeln!(stdout, "{answer}").and_then(|_| stdout.flush()).is_err()
        {
            break;
        }
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The answer to one message; None for a notification.
pub fn handle(message: &Value) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let method = message["method"].as_str().unwrap_or_default();
    let params = &message["params"];
    let result = match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or_default();
            let version = VERSIONS.iter().find(|v| **v == asked).unwrap_or(&VERSIONS[1]);
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "cockpit", "title": "cockpit", "version": env!("CARGO_PKG_VERSION")},
                "instructions": tools::instructions(),
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools::list()}),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call(name, &args) {
                Some(Ok(text)) => json!({"content": [{"type": "text", "text": text}], "isError": false}),
                Some(Err(text)) => json!({"content": [{"type": "text", "text": text}], "isError": true}),
                None => return Some(error(id, -32602, &format!("unknown tool {name:?}"))),
            }
        }
        "resources/list" => json!({"resources": []}),
        "prompts/list" => json!({"prompts": []}),
        _ => return Some(error(id, -32601, &format!("method not found: {method}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// A tool's text, or what went wrong as the tool's error; None for a tool cockpit does not have.
fn call(name: &str, args: &Value) -> Option<Result<String, String>> {
    Some(match name {
        "show" => {
            let shown = show::show(args, here(), session());
            if shown.is_ok() {
                follow_answers();
            }
            shown
        }
        "cockpit_search" => {
            let query = args["query"].as_str().unwrap_or_default().trim();
            if query.is_empty() {
                Err("query: the words to search for, e.g. a ticket id or part of a PR title".into())
            } else {
                find::search_json(query).map(|v| v.to_string()).map_err(|e| format!("cockpit search: {e}"))
            }
        }
        "cockpit_changes" => {
            changes(args).and_then(|(since, app)| find::events_json(since, &app).map(|v| v.to_string()).map_err(|e| format!("cockpit events: {e}")))
        }
        "report_step" => report::report(&cockpit_core::files::cache().join("handoffs"), args),
        _ => return None,
    })
}

/// Since when and which app, from the tool's input, or what is wrong with it.
fn changes(args: &Value) -> Result<(u64, String), String> {
    let now = find::now_ms();
    let since = match &args["since"] {
        Value::Null => now.saturating_sub(86_400_000),
        given @ (Value::String(_) | Value::Number(_)) => {
            let text = given.as_str().map(str::to_string).unwrap_or_else(|| given.to_string());
            let text = text.trim();
            let shaped = text.len() > 1 && text[..text.len() - 1].chars().all(|c| c.is_ascii_digit()) && text.ends_with(['m', 'h', 'd'])
                || text.len() >= 13 && text.chars().all(|c| c.is_ascii_digit());
            shaped.then(|| find::since_ms(text, now)).flatten().ok_or_else(|| format!("since: 30m, 2h, 1d or epoch milliseconds, got {given}"))?
        }
        other => return Err(format!("since: 30m, 2h, 1d or epoch milliseconds, got {other}")),
    };
    let app = match &args["app"] {
        Value::Null => String::new(),
        Value::String(app) if tools::APPS.contains(&app.as_str()) => app.clone(),
        other => return Err(format!("app: one of {}, got {other}", tools::APPS.join(", "))),
    };
    Ok((since, app))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(method: &str, params: Value) -> Value {
        handle(&json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params})).unwrap()
    }

    #[test]
    fn it_introduces_itself_with_the_drawing_guidance() {
        let answer = ask("initialize", json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "codex", "version": "1"}}));
        assert_eq!(answer["id"], 7);
        assert_eq!(answer["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(answer["result"]["serverInfo"]["name"], "cockpit");
        assert!(answer["result"]["instructions"].as_str().unwrap().contains("cockpit's show tool"));
        assert_eq!(ask("initialize", json!({"protocolVersion": "1999-01-01"}))["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})), None);
    }

    #[test]
    fn it_lists_the_plugins_four_tools() {
        let tools = ask("tools/list", json!({}));
        let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["show", "cockpit_search", "cockpit_changes", "report_step"]);
    }

    #[test]
    fn a_wrong_call_says_what_is_wrong() {
        let shown = ask("tools/call", json!({"name": "show", "arguments": {"pane": "x", "title": "t", "blocks": [{"type": "pie"}]}}));
        assert_eq!(shown["result"]["isError"], true);
        assert!(shown["result"]["content"][0]["text"].as_str().unwrap().starts_with("at /blocks/0: unknown block type \"pie\""));
        let empty = ask("tools/call", json!({"name": "cockpit_search", "arguments": {"query": " "}}));
        assert_eq!(empty["result"]["content"][0]["text"], "query: the words to search for, e.g. a ticket id or part of a PR title");
        assert_eq!(ask("tools/call", json!({"name": "nope", "arguments": {}}))["error"]["code"], -32602);
        assert_eq!(ask("resources/read", json!({}))["error"]["code"], -32601);
    }

    #[test]
    fn changes_take_what_the_plugin_takes() {
        let (since, app) = changes(&json!({"since": "2h", "app": "monitor"})).unwrap();
        let now = find::now_ms();
        assert!(now - since >= 7_200_000 && now - since < 7_260_000);
        assert_eq!(app, "monitor");
        assert_eq!(changes(&json!({"since": 1_761_000_000_000u64})).unwrap().0, 1_761_000_000_000);
        assert_eq!(changes(&json!({"since": "soon"})).unwrap_err(), "since: 30m, 2h, 1d or epoch milliseconds, got \"soon\"");
        assert_eq!(changes(&json!({"app": "todo"})).unwrap_err(), "app: one of inbox, monitor, got \"todo\"");
    }
}
