//! Each agent's prompt queue: follow-ups written while it works, sent one at a time or all at once as one numbered
//! prompt. The hub's `queue` document holds them by terminal handle (`app:t3`, or a herdr pane id).

use std::time::{SystemTime, UNIX_EPOCH};

use cockpit_core::frontend::{self, APP};
use cockpit_hub::store;
use serde_json::{Map, Value, json};

pub fn dirs() -> Vec<std::path::PathBuf> {
    Vec::new()
}

/// The queues as the mod draws them: {queues: {handle: [{id, text, at}]}, count}.
pub fn view(_args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let queues = doc("queue")["queues"].as_object().cloned().unwrap_or_default();
    let count: usize = queues.values().map(|q| q.as_array().map_or(0, Vec::len)).sum();
    json!({"queues": queues, "count": count})
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The prompt that sends these items: the one alone, several numbered.
pub fn prompt(texts: &[String]) -> String {
    match texts {
        [one] => one.clone(),
        many => many.iter().enumerate().map(|(i, t)| format!("{}. {t}", i + 1)).collect::<Vec<_>>().join("\n"),
    }
}

/// One edit of the queues; Err when the edit says why it did nothing.
pub fn edit(queues: &mut Map<String, Value>, action: &str, args: &[Value], now: u64) -> Result<(String, Vec<String>), String> {
    let arg = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let handle = arg(0);
    if handle.is_empty() {
        return Err("which agent?".into());
    }
    let list = queues.entry(handle.clone()).or_insert_with(|| json!([])).as_array_mut().ok_or("not a queue")?;
    let index = |id: &str, list: &[Value]| list.iter().position(|i| i["id"] == id).ok_or_else(|| "not in the queue any more".to_string());
    let note = match action {
        "add" => {
            let text = arg(1);
            if text.is_empty() {
                return Ok(("nothing to add".into(), Vec::new()));
            }
            list.push(json!({"id": format!("q{now}-{}", list.len()), "text": text, "at": now}));
            "queued".to_string()
        }
        "edit" => {
            let at = index(&arg(1), list)?;
            list[at]["text"] = json!(arg(2));
            String::new()
        }
        "remove" => {
            let at = index(&arg(1), list)?;
            list.remove(at);
            String::new()
        }
        "move" => {
            let at = index(&arg(1), list)?;
            let to = args.get(2).and_then(Value::as_u64).unwrap_or(0) as usize;
            let item = list.remove(at);
            list.insert(to.min(list.len()), item);
            String::new()
        }
        "clear" => {
            list.clear();
            String::new()
        }
        "send" => {
            let which = arg(1);
            let taken: Vec<Value> = if which.is_empty() || which == "all" {
                std::mem::take(list)
            } else {
                let at = index(&which, list)?;
                vec![list.remove(at)]
            };
            let texts: Vec<String> = taken.iter().filter_map(|i| i["text"].as_str()).map(str::to_string).collect();
            if texts.is_empty() {
                return Ok(("nothing queued".into(), Vec::new()));
            }
            let note = if texts.len() == 1 { "sent".to_string() } else { format!("sent {} prompts", texts.len()) };
            queues.retain(|_, q| q.as_array().is_some_and(|a| !a.is_empty()));
            return Ok((note, texts));
        }
        other => return Err(format!("no queue action {other}")),
    };
    queues.retain(|_, q| q.as_array().is_some_and(|a| !a.is_empty()));
    Ok((note, Vec::new()))
}

/// Types the prompt into the agent: `hc send` in the app (it refuses an agent waiting on a dialog), `herdr pane run` in
/// the TUI.
fn send(handle: &str, text: &str) -> Result<(), String> {
    let ok = match handle.strip_prefix(APP) {
        Some(terminal) => frontend::hc(&["send", terminal, text]).is_some(),
        None => frontend::herdr(&["pane", "run", handle, text]).is_some(),
    };
    if ok { Ok(()) } else { Err("the agent did not take it (waiting on a question? answer it first)".into()) }
}

/// The queue's actions, args [handle, ...]: `add [h, text]`, `edit [h, id, text]`, `remove [h, id]`, `move [h, id, to]`,
/// `clear [h]`, `send [h, id | "all"]`.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    if !["add", "edit", "remove", "move", "clear", "send"].contains(&action) {
        return None;
    }
    let mut outcome: Result<(String, Vec<String>), String> = Err(String::new());
    let before = store::global().get("queue");
    store::global().update("queue", |doc| {
        let mut queues = doc["queues"].as_object().cloned().unwrap_or_default();
        outcome = edit(&mut queues, action, args, now_ms());
        if outcome.is_ok() { json!({"version": 1, "queues": queues}) } else { doc.clone() }
    });
    Some(match outcome {
        Ok((note, texts)) if !texts.is_empty() => {
            let handle = args.first().and_then(Value::as_str).unwrap_or_default();
            match send(handle, &prompt(&texts)) {
                Ok(()) => Ok(note),
                Err(err) => {
                    // Not sent: the items go back where they were.
                    store::global().update("queue", |_| before.clone());
                    Err(err)
                }
            }
        }
        Ok((note, _)) => Ok(note),
        Err(err) => Err(err),
    })
}

/// herdr's queue popup (prefix+shift+q) on the pane it was opened over: a line queues it, `s` sends them all as one
/// prompt, `s <n>` one, `d <n>` drops one, `c` clears, `q` quits.
fn pick(handle: &str) {
    use std::io::{BufRead, Write};
    let request = |req: Value| cockpit_core::hub::request(&req, std::time::Duration::from_secs(20));
    let act = |action: &str, args: Vec<Value>| match request(json!({"op": "act", "app": "queue", "action": action, "args": args})) {
        Ok(answer) => answer["error"].as_str().or(answer["note"].as_str()).unwrap_or_default().to_string(),
        Err(err) => err,
    };
    let mut note = String::new();
    loop {
        let value = request(json!({"op": "view", "name": "queue"})).map(|a| a["value"].clone()).unwrap_or_default();
        let items: Vec<Value> = value["queues"][handle].as_array().cloned().unwrap_or_default();
        print!("\x1b[2J\x1b[HQueued for {handle}\n\n");
        if items.is_empty() {
            println!("  nothing yet: type a prompt to send once the agent is free");
        }
        for (i, item) in items.iter().enumerate() {
            println!("  [{}] {}", i + 1, item["text"].as_str().unwrap_or_default());
        }
        if !note.is_empty() {
            println!("\n{note}");
        }
        print!("\n<text> queues · s sends all · s <n> one · d <n> drops · c clears · q quits\n> ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let id = |n: &str| n.parse::<usize>().ok().and_then(|n| items.get(n.wrapping_sub(1))).map(|i| i["id"].clone());
        let words: Vec<&str> = line.split_whitespace().collect();
        note = match words.as_slice() {
            [] => String::new(),
            ["q" | "quit"] => return,
            ["s"] => act("send", vec![json!(handle), json!("all")]),
            ["s", n] if id(n).is_some() => act("send", vec![json!(handle), id(n).unwrap_or_default()]),
            ["d", n] if id(n).is_some() => act("remove", vec![json!(handle), id(n).unwrap_or_default()]),
            ["c"] => act("clear", vec![json!(handle)]),
            _ => act("add", vec![json!(handle), json!(line.trim())]),
        };
    }
}

/// `cockpit queue`: the queues as JSON, one of their actions through the hub, or `pick` (herdr's popup).
pub fn main(args: &[String]) {
    let (action, rest) = args.split_first().map(|(a, r)| (a.as_str(), r)).unwrap_or(("list", &[]));
    let request = match action {
        "pick" => {
            let handle = rest.first().cloned().or_else(|| std::env::var("QUEUE_PANE").ok()).filter(|h| !h.is_empty());
            return pick(&handle.unwrap_or_else(|| crate::proc::fail("cockpit queue pick <terminal>: which agent?")));
        }
        "list" => json!({"op": "view", "name": "queue"}),
        "add" | "send" | "clear" | "remove" => {
            let mut parts: Vec<Value> = rest.iter().take(1).map(|h| json!(h)).collect();
            if !rest.is_empty() {
                parts.push(json!(rest[1..].join(" ")));
            }
            if action == "clear" {
                parts.truncate(1);
            }
            json!({"op": "act", "app": "queue", "action": action, "args": parts})
        }
        _ => crate::proc::fail("cockpit queue [list|add <terminal> <text>|send <terminal> [all|<id>]|clear <terminal>|pick [terminal]]"),
    };
    match cockpit_core::hub::request(&request, std::time::Duration::from_secs(20)) {
        Ok(answer) if action == "list" => println!("{}", answer["value"]),
        Ok(answer) if answer["error"].is_string() => crate::proc::fail(answer["error"].as_str().unwrap_or_default()),
        Ok(answer) => println!("{}", answer["note"].as_str().unwrap_or_default()),
        Err(err) => crate::proc::fail(&err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_queue_move_and_send_numbered() {
        let mut q = Map::new();
        let h = json!("app:t3");
        edit(&mut q, "add", &[h.clone(), json!("run the tests")], 1).unwrap();
        edit(&mut q, "add", &[h.clone(), json!("then push")], 2).unwrap();
        edit(&mut q, "add", &[h.clone(), json!("  ")], 3).unwrap();
        assert_eq!(q["app:t3"].as_array().unwrap().len(), 2);
        let second = q["app:t3"][1]["id"].as_str().unwrap().to_string();
        edit(&mut q, "move", &[h.clone(), json!(second), json!(0)], 4).unwrap();
        assert_eq!(q["app:t3"][0]["text"], "then push");
        let (note, texts) = edit(&mut q, "send", &[h.clone(), json!("all")], 5).unwrap();
        assert_eq!((note.as_str(), prompt(&texts)), ("sent 2 prompts", "1. then push\n2. run the tests".to_string()));
        assert!(q.get("app:t3").is_none());
        assert!(edit(&mut q, "remove", &[h, json!("q9")], 6).is_err());
    }
}
