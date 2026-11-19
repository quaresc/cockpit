//! Each agent's prompt queue: follow-ups written while it works, the next one sent on its own once the agent rests
//! (unless the queue is paused: after a failed or stopped turn, or written for another agent), or sent by hand. The
//! hub's `queue` document holds them by handle (`app:t3`, a herdr pane id, a driven session's key), with why a queue is
//! paused, who it was written for and what went last on its own.

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use cockpit_agents::gui;
use cockpit_core::deliver::is_waiting;
use cockpit_hub::store;
use serde_json::{Map, Value, json};

/// Why a queue holds after a turn that failed, after one the person stopped, and when it was written for another agent.
pub const FAILED: &str = "the last turn failed";
pub const STOPPED: &str = "you stopped it";
pub const PARKED: &str = "written for another agent in this terminal: resume sends them to this one";
/// A stop that ends a turn this soon after a prompt went on its own puts that prompt back first.
const UNDO_MS: u64 = 10_000;
/// The last prompt that went on its own holds the next until the agent is seen resting after it, or this long.
const SENT_HOLDS_MS: u64 = 60_000;

pub fn dirs() -> Vec<std::path::PathBuf> {
    Vec::new()
}

/// The queues as the mod draws them: {queues: {handle: [{id, text, at}]}, paused: {handle: {why, at}}, drain, count}.
pub fn view(_args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let doc = doc("queue");
    let queues = doc["queues"].as_object().cloned().unwrap_or_default();
    let count: usize = queues.values().map(|q| q.as_array().map_or(0, Vec::len)).sum();
    // A driven session that is paused or whose agent ended takes nothing on its own: a prompt by hand picks it up.
    let asleep: Map<String, Value> = queues
        .keys()
        .filter(|h| cockpit_agents::session::summary(h).is_some_and(|s| s["paused"] == true || !s["ended"].is_null()))
        .map(|h| (h.clone(), json!(true)))
        .collect();
    json!({"queues": queues, "paused": doc["paused"].as_object().cloned().unwrap_or_default(), "asleep": asleep, "drain": drains(), "count": count})
}

/// Whether a prompt goes on its own once the agent rests: on unless the config's `queue.drain` is false.
fn drains() -> bool {
    cockpit_core::files::config()["queue"]["drain"] != false
}

fn copy(doc: &Value) -> Value {
    if doc.is_object() { doc.clone() } else { json!({"version": 1, "queues": {}}) }
}

/// The document with the next prompt for `handle` taken out, and that prompt, noted as the last that went at `now`;
/// nothing is taken from a paused queue.
pub fn take_next(doc: &Value, handle: &str, now: u64) -> (Value, Option<Value>) {
    let mut doc = copy(doc);
    if !doc["paused"][handle].is_null() {
        return (doc, None);
    }
    // Looked at before it is taken from: indexing a missing queue mutably would add it, empty.
    if doc["queues"][handle].as_array().is_none_or(Vec::is_empty) {
        return (doc, None);
    }
    let item = doc["queues"][handle].as_array_mut().map(|list| list.remove(0));
    if let Some(item) = &item {
        doc["sent"][handle] = json!({"at": now, "id": item["id"], "text": item["text"], "queued": item["at"]});
    }
    tidy(&mut doc);
    (doc, item)
}

/// The document with `items` back in `handle`'s queue from position `at` (any already there left as they are), paused
/// with `why` when one is given.
pub fn put_back(doc: &Value, handle: &str, items: &[Value], at: usize, why: Option<&str>, now: u64) -> Value {
    let mut doc = copy(doc);
    if !doc["queues"][handle].is_array() {
        doc["queues"][handle] = json!([]);
    }
    if let Some(list) = doc["queues"][handle].as_array_mut() {
        let back: Vec<Value> = items.iter().filter(|i| !list.iter().any(|l| l["id"] == i["id"])).cloned().collect();
        let at = at.min(list.len());
        list.splice(at..at, back);
    }
    if let Some(why) = why {
        doc["paused"][handle] = json!({"why": why, "at": now});
    }
    tidy(&mut doc);
    doc
}

/// The document with `handle`'s queue paused with `why`; an empty queue has nothing to hold.
pub fn paused(doc: &Value, handle: &str, why: &str, now: u64) -> Value {
    let mut doc = copy(doc);
    if doc["queues"][handle].as_array().is_some_and(|l| !l.is_empty()) {
        doc["paused"][handle] = json!({"why": why, "at": now});
    }
    doc
}

/// The document with `handle`'s pause lifted, its queue now written for `owner`.
pub fn unpaused(doc: &Value, handle: &str, owner: Option<&str>) -> Value {
    let mut doc = copy(doc);
    if let Some(paused) = doc["paused"].as_object_mut() {
        paused.remove(handle);
    }
    if let Some(owner) = owner {
        doc["owners"][handle] = json!(owner);
    }
    tidy(&mut doc);
    doc
}

/// Whether `handle`'s queue was written for `owner`: one that names nobody, or an agent nobody can name now, passes.
pub fn owned(doc: &Value, handle: &str, owner: Option<&str>) -> bool {
    match (doc["owners"][handle].as_str(), owner) {
        (Some(was), Some(now)) => was == now,
        _ => true,
    }
}

/// A stopped turn: a prompt that went on its own just before, and whose turn this was (no turn ended since it went), comes
/// back first; then the queue holds.
pub fn stopped(doc: &Value, handle: &str, now: u64) -> Value {
    let sent = doc["sent"][handle].clone();
    let mut doc = copy(doc);
    if sent["ended"].is_null() && sent["at"].as_u64().is_some_and(|at| now.saturating_sub(at) < UNDO_MS) {
        let item = json!({"id": sent["id"], "text": sent["text"], "at": sent["queued"]});
        doc = put_back(&doc, handle, &[item], 0, None, now);
        if let Some(all) = doc["sent"].as_object_mut() {
            all.remove(handle);
        }
    }
    paused(&doc, handle, STOPPED, now)
}

/// The document after a turn of the agent at `handle` ended done or failed: the last prompt that went had its turn.
pub fn retired(doc: &Value, handle: &str, now: u64) -> Value {
    let mut doc = copy(doc);
    if doc["sent"][handle].is_object() {
        doc["sent"][handle]["ended"] = json!(now);
    }
    doc
}

/// The document without anything for `handle`: its queue, pause, owner and last prompt, as when its session ended.
pub fn forgotten(doc: &Value, handle: &str) -> Value {
    let mut doc = copy(doc);
    for part in ["queues", "paused", "owners", "sent"] {
        if let Some(map) = doc[part].as_object_mut() {
            map.remove(handle);
        }
    }
    doc
}

/// No empty queue kept, and no pause or owner for a queue that is gone; the last prompt that went stays, for the stop
/// that may follow it.
fn tidy(doc: &mut Value) {
    if let Some(queues) = doc["queues"].as_object_mut() {
        queues.retain(|_, q| q.as_array().is_some_and(|a| !a.is_empty()));
    }
    let queues = doc["queues"].clone();
    for part in ["paused", "owners"] {
        if let Some(map) = doc[part].as_object_mut() {
            map.retain(|h, _| queues[h.as_str()].is_array());
        }
    }
}

/// The handles a drain runs for: a second look at the same rest waits for the first to finish.
fn draining() -> MutexGuard<'static, HashSet<String>> {
    static DRAINING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    DRAINING.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// Sends the next prompt queued for `handle` when the agent is ready for it (resting since after the last one went,
/// unless `now` is asked), taken out of the document inside the store's update so that nothing else takes it too, and
/// nothing at all on a store handed over to another hub; put back first when it does not go.
fn drain(handle: &str, now: bool) -> Option<Result<(), String>> {
    let last = store::global().get("queue")["sent"][handle]["at"].as_u64().unwrap_or(0);
    gui::ready(handle, rest_after(last, now_ms(), now), now).ok()?;
    let mut taken = None;
    store::global().update("queue", |doc| {
        let (next, item) = take_next(doc, handle, now_ms());
        taken = item;
        next
    });
    let item = taken?;
    // On its own, it looks at the agent again under the lock its answers take before typing.
    let text = item["text"].as_str().unwrap_or_default();
    let went = if now { send(handle, text) } else { gui::automatically(|| send(handle, text)) };
    match went {
        Ok(()) => Some(Ok(())),
        // A session ended meanwhile takes its queue with it: nothing comes back for it.
        Err(why) if cockpit_agents::session::is_gone(&why) => Some(Err(why)),
        Err(why) => {
            // Refused for a question, a screen it cannot read or a draft, it goes on a later look; anything else holds the
            // queue.
            let hold = (!is_waiting(&why) && !gui::passes(&why)).then(|| format!("it did not take the last prompt: {why}"));
            store::global().update("queue", |doc| put_back(doc, handle, std::slice::from_ref(&item), 0, hold.as_deref(), now_ms()));
            Some(Err(why))
        }
    }
}

/// Since when the agent must have rested for a prompt to go on its own: since the last one went, unless that was long
/// ago or the person asks for it now.
pub fn rest_after(last: u64, now: u64, asked: bool) -> u64 {
    if asked || now.saturating_sub(last) > SENT_HOLDS_MS { 0 } else { last }
}

/// Whether `tell`'s words go now: the agent is ready and no queue of its own runs ahead of them.
pub fn tell_now(ready: bool, waiting: usize, runs: bool) -> bool {
    ready && (waiting == 0 || !runs)
}

/// The poller saw the agent at `handle` at rest: the next prompt goes when its queue runs on its own, was written for
/// this agent (else it is parked) and the agent is ready, on a thread of its own since it types into the terminal.
pub fn rested(handle: &str) {
    let doc = store::global().get("queue");
    if doc["queues"][handle].as_array().is_none_or(Vec::is_empty) || !doc["paused"][handle].is_null() || !drains() {
        return;
    }
    if !owned(&doc, handle, gui::owner(handle).as_deref()) {
        store::global().update("queue", |doc| paused(doc, handle, PARKED, now_ms()));
        return;
    }
    if !draining().insert(handle.to_string()) {
        return;
    }
    let handle = handle.to_string();
    std::thread::spawn(move || {
        let _ = drain(&handle, false);
        draining().remove(&handle);
    });
}

/// A turn of the agent at `handle` ended: a failed or stopped one holds its queue, a turn that ends well lifts the hold
/// a failure put on it, and either retires the last prompt that went; a driven session that `ended` takes its queue
/// with it.
pub fn turn_ended(handle: &str, outcome: &str) {
    let now = now_ms();
    store::global().update("queue", |doc| match outcome {
        "failed" => paused(&retired(doc, handle, now), handle, FAILED, now),
        "interrupted" => stopped(doc, handle, now),
        "done" if doc["paused"][handle]["why"] == FAILED => unpaused(&retired(doc, handle, now), handle, None),
        "done" => retired(doc, handle, now),
        "ended" => forgotten(doc, handle),
        _ => doc.clone(),
    });
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
pub fn edit(queues: &mut Map<String, Value>, action: &str, args: &[Value], now: u64) -> Result<String, String> {
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
                return Err("nothing to queue".into());
            }
            list.push(json!({"id": format!("q{now}-{}", list.len()), "text": text, "at": now}));
            "queued".to_string()
        }
        "edit" => {
            let at = index(&arg(1), list)?;
            if arg(2).is_empty() {
                return Err("nothing to put there: remove it instead".into());
            }
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
        other => return Err(format!("no queue action {other}")),
    };
    queues.retain(|_, q| q.as_array().is_some_and(|a| !a.is_empty()));
    Ok(note)
}

/// The document with what `send [handle, which]` sends taken out (all, or the item `which` names), the items, and where
/// they stood.
pub fn take_for_send(doc: &Value, handle: &str, which: &str) -> Result<(Value, Vec<Value>, usize), String> {
    let mut doc = copy(doc);
    let list = doc["queues"][handle].as_array().cloned().unwrap_or_default();
    let (taken, at) = match which {
        "" | "all" => (list, 0),
        id => {
            let at = list.iter().position(|i| i["id"] == id).ok_or("not in the queue any more")?;
            (vec![list[at].clone()], at)
        }
    };
    if taken.is_empty() {
        return Err("nothing queued".into());
    }
    if let Some(list) = doc["queues"][handle].as_array_mut() {
        list.retain(|i| !taken.iter().any(|t| t["id"] == i["id"]));
    }
    tidy(&mut doc);
    Ok((doc, taken, at))
}

/// Hands the prompt to the agent, a terminal or a session cockpit drives; nothing goes while it waits on a question.
fn send(handle: &str, text: &str) -> Result<(), String> {
    cockpit_core::deliver::deliver(handle, text)
}

/// How many prompts wait in `handle`'s queue.
pub fn waiting(handle: &str) -> usize {
    store::global().get("queue")["queues"][handle].as_array().map_or(0, Vec::len)
}

/// The queue's actions, args `[handle, ...]`: add, tell (sent now when the agent is ready and nothing waits ahead, else
/// queued), edit, remove, move, clear, send (now, paused or not) and resume; docs/app.md has their arguments. An agent's
/// own tools write only to its own queue.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    if !["add", "tell", "edit", "remove", "move", "clear", "send", "resume"].contains(&action) {
        return None;
    }
    let handle = args.first().and_then(Value::as_str).unwrap_or_default().to_string();
    // An agent's own tools queue words for their own agent only.
    if let Err(why) = gui::may_touch(&handle) {
        return Some(Err(why));
    }
    Some(match action {
        "resume" => resume(&handle),
        "tell" => tell(&handle, args.get(1).and_then(Value::as_str).unwrap_or_default().trim()),
        "send" => send_now(&handle, args.get(1).and_then(Value::as_str).unwrap_or_default()),
        _ => change(action, args),
    })
}

/// An edit of `handle`'s queue, its pause, owner and last prompt kept; an item added names who it was written for.
fn change(action: &str, args: &[Value]) -> Result<String, String> {
    let handle = args.first().and_then(Value::as_str).unwrap_or_default();
    let owner = (action == "add").then(|| gui::owner(handle)).flatten();
    let mut outcome = Err(String::new());
    store::global().update("queue", |doc| {
        let mut next = copy(doc);
        let mut queues = doc["queues"].as_object().cloned().unwrap_or_default();
        outcome = edit(&mut queues, action, args, now_ms());
        if outcome.is_err() {
            return doc.clone();
        }
        next["queues"] = json!(queues);
        if let Some(owner) = &owner
            && next["owners"][handle].is_null()
        {
            next["owners"][handle] = json!(owner);
        }
        tidy(&mut next);
        next
    });
    outcome
}

/// `send`: what it names goes now, paused or not; when it does not go, those items go back where they stood.
fn send_now(handle: &str, which: &str) -> Result<String, String> {
    let mut taken: Result<(Vec<Value>, usize), String> = Err("nothing queued".into());
    store::global().update("queue", |doc| match take_for_send(doc, handle, which) {
        Ok((next, items, at)) => {
            taken = Ok((items, at));
            next
        }
        Err(why) => {
            taken = Err(why);
            doc.clone()
        }
    });
    let (items, at) = taken?;
    let texts: Vec<String> = items.iter().filter_map(|i| i["text"].as_str()).map(str::to_string).collect();
    match send(handle, &prompt(&texts)) {
        Ok(()) => Ok(if texts.len() == 1 { "sent".into() } else { format!("sent {} prompts", texts.len()) }),
        Err(why) => {
            store::global().update("queue", |doc| put_back(doc, handle, &items, at, None, now_ms()));
            Err(why)
        }
    }
}

/// `tell`: words for an agent that is ready go now, older items left queued when its queue is held; otherwise they
/// queue, and the note says when they will go.
fn tell(handle: &str, text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Err("nothing to send".into());
    }
    let doc = store::global().get("queue");
    let waiting = doc["queues"][handle].as_array().map_or(0, Vec::len);
    let hold = doc["paused"][handle]["why"].as_str().map(str::to_string);
    let runs = drains() && hold.is_none();
    let ready = gui::ready(handle, 0, true).is_ok();
    let mut asks = false;
    if tell_now(ready, waiting, runs) {
        match send(handle, text) {
            Ok(()) => {
                let older = match (&hold, waiting) {
                    (_, 0) => String::new(),
                    (Some(why), n) => format!(": {n} older still wait, paused ({why})"),
                    (None, n) => format!(": {n} older still wait, sent by hand only (queue.drain is off)"),
                };
                return Ok(format!("sent{older}"));
            }
            Err(why) if !is_waiting(&why) => return Err(why),
            Err(_) => asks = true,
        }
    }
    change("add", &[json!(handle), json!(text)])?;
    if !asks && ready && runs && draining().insert(handle.to_string()) {
        // Ready with a queue that runs: the oldest goes first, now.
        let went = drain(handle, true);
        draining().remove(handle);
        return behind(waiting, went);
    }
    Ok(match (hold, drains()) {
        _ if asks => "queued: it waits on a question, this goes once it is answered and the agent rests".into(),
        (Some(why), _) => format!("queued: its queue is paused ({why}), resume to send"),
        (None, false) => "queued: queue.drain is off, send it by hand".into(),
        (None, true) => "queued: it goes once the agent rests".into(),
    })
}

/// What `tell` says once its words are queued behind `waiting` older ones and the oldest was asked to go now (`went`):
/// always queued, whatever became of the oldest, since sending them again would say them twice.
pub fn behind(waiting: usize, went: Option<Result<(), String>>) -> Result<String, String> {
    match went {
        Some(Ok(())) => Ok(format!("queued behind {waiting}: the oldest went now, the rest follow as it rests")),
        Some(Err(why)) if is_waiting(&why) => Ok(format!("queued behind {waiting}: it waits on a question, they go once it is answered")),
        Some(Err(why)) => Ok(format!("queued behind {waiting}: the oldest did not go ({why}), the queue keeps them")),
        None => Ok(format!("queued behind {waiting}: they go once the agent rests")),
    }
}

/// `resume`: the hold lifts and the queue is now this agent's; the next goes now when the agent is ready and the queue
/// runs on its own.
fn resume(handle: &str) -> Result<String, String> {
    let owner = gui::owner(handle);
    store::global().update("queue", |doc| unpaused(doc, handle, owner.as_deref()));
    if !drains() {
        return Ok("resumed: queue.drain is off, so they go by hand".into());
    }
    // One drain at a time for the agent: a look at its rest that is sending one already sends the next.
    if !draining().insert(handle.to_string()) {
        return Ok("resumed: the next is on its way".into());
    }
    let went = drain(handle, true);
    draining().remove(handle);
    match went {
        Some(Ok(())) => Ok("resumed: sent the next prompt".into()),
        Some(Err(why)) => Err(why),
        None => Ok("resumed: the next goes once the agent rests".into()),
    }
}

/// herdr's queue popup (prefix+shift+q) on the pane it was opened over: a line is told to the agent (sent now when it is
/// ready, else queued); commands start with a colon so no prompt is taken for one: `:s` sends them all as one prompt,
/// `:s <n>` one, `:e <n> <text>` rewrites one, `:m <n> <to>` moves one, `:d <n>` drops one, `:c` clears, `:r` resumes a
/// paused queue, `:q` quits.
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
        println!("\x1b[2J\x1b[HQueued for {handle}");
        println!("{}\n", state_line(&value, handle));
        if items.is_empty() {
            println!("  nothing yet: type a prompt to send once the agent is free");
        }
        for (i, item) in items.iter().enumerate() {
            println!("  [{}] {}", i + 1, item["text"].as_str().unwrap_or_default());
        }
        if !note.is_empty() {
            println!("\n{note}");
        }
        print!(
            "\n<text> tells it · :s sends all · :s <n> one · :e <n> <text> rewrites · :m <n> <to> moves · :d <n> drops · :c clears · :r resumes · :q quits\n> "
        );
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let id = |n: &str| n.parse::<usize>().ok().and_then(|n| items.get(n.wrapping_sub(1))).map(|i| i["id"].clone());
        let words: Vec<&str> = line.split_whitespace().collect();
        note = match words.as_slice() {
            [] => String::new(),
            [":q" | ":quit"] => return,
            [":s"] => act("send", vec![json!(handle), json!("all")]),
            [":s", n] if id(n).is_some() => act("send", vec![json!(handle), id(n).unwrap_or_default()]),
            [":d", n] if id(n).is_some() => act("remove", vec![json!(handle), id(n).unwrap_or_default()]),
            [":e", n, ..] if id(n).is_some() && words.len() > 2 => act("edit", vec![json!(handle), id(n).unwrap_or_default(), json!(words[2..].join(" "))]),
            [":m", n, to] if id(n).is_some() && to.parse::<usize>().is_ok_and(|t| t >= 1) => {
                act("move", vec![json!(handle), id(n).unwrap_or_default(), json!(to.parse::<usize>().unwrap_or(1) - 1)])
            }
            [":c"] => act("clear", vec![json!(handle)]),
            [":r"] => act("resume", vec![json!(handle)]),
            [first, ..] if first.starts_with(':') => format!("no command {first}: commands are :s :e :m :d :c :r :q"),
            _ => act("tell", vec![json!(handle), json!(line.trim())]),
        };
    }
}

/// How the queue stands, in a line: paused and why, its driven session paused, sent by hand only, or sent as turns end.
pub fn state_line(view: &Value, handle: &str) -> String {
    match view["paused"][handle]["why"].as_str() {
        Some(why) => format!("Paused — {why} · :r resumes"),
        None if view["asleep"][handle] == true => "Its session is paused: nothing goes on its own · :s sends, which picks it up".into(),
        None if view["drain"] == false => "Sent by hand only: queue.drain is off".into(),
        None => "Each goes once the agent rests".into(),
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
        "add" | "tell" | "send" | "clear" | "remove" | "resume" => {
            let mut parts: Vec<Value> = rest.iter().take(1).map(|h| json!(h)).collect();
            if !rest.is_empty() {
                parts.push(json!(rest[1..].join(" ")));
            }
            if ["clear", "resume"].contains(&action) {
                parts.truncate(1);
            }
            json!({"op": "act", "app": "queue", "action": action, "args": parts})
        }
        _ => crate::proc::fail(
            "cockpit queue [list|add <terminal> <text>|tell <terminal> <text>|send <terminal> [all|<id>]|clear <terminal>|resume <terminal>|pick [terminal]]",
        ),
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
    fn items_queue_move_and_take_for_a_numbered_send() {
        let mut q = Map::new();
        let h = json!("app:t3");
        edit(&mut q, "add", &[h.clone(), json!("run the tests")], 1).unwrap();
        edit(&mut q, "add", &[h.clone(), json!("then push")], 2).unwrap();
        assert!(edit(&mut q, "add", &[h.clone(), json!("  ")], 3).is_err(), "nothing to queue");
        assert_eq!(q["app:t3"].as_array().unwrap().len(), 2);
        let second = q["app:t3"][1]["id"].as_str().unwrap().to_string();
        assert!(edit(&mut q, "edit", &[h.clone(), json!(second), json!(" ")], 4).is_err(), "an edit to nothing is refused");
        edit(&mut q, "move", &[h.clone(), json!(second), json!(0)], 4).unwrap();
        assert_eq!(q["app:t3"][0]["text"], "then push");
        let (left, items, at) = take_for_send(&json!({"queues": q}), "app:t3", "all").unwrap();
        let texts: Vec<String> = items.iter().map(|i| i["text"].as_str().unwrap().to_string()).collect();
        assert_eq!((prompt(&texts), at), ("1. then push\n2. run the tests".to_string(), 0));
        assert!(left["queues"]["app:t3"].is_null());
        assert!(edit(&mut q, "remove", &[h, json!("q9")], 6).is_err());
    }

    fn doc(texts: &[&str]) -> Value {
        let items: Vec<Value> = texts.iter().enumerate().map(|(i, t)| json!({"id": format!("q{i}"), "text": t, "at": i})).collect();
        json!({"version": 1, "queues": {"claude:1": items}})
    }

    fn texts(d: &Value, handle: &str) -> Vec<String> {
        d["queues"][handle].as_array().into_iter().flatten().map(|i| i["text"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn rests_take_the_items_one_at_a_time_in_order_and_note_the_last() {
        let mut d = doc(&["run the tests", "then fix them", "then push"]);
        let mut sent = Vec::new();
        while let (next, Some(item)) = take_next(&d, "claude:1", 100) {
            sent.push(item["text"].as_str().unwrap().to_string());
            d = next;
        }
        assert_eq!(sent, ["run the tests", "then fix them", "then push"]);
        assert_eq!((d["sent"]["claude:1"]["text"].clone(), d["sent"]["claude:1"]["at"].clone()), (json!("then push"), json!(100)), "what went last, and when");
        assert!(d["queues"]["claude:1"].is_null(), "an empty queue is not kept");
        assert_eq!(take_next(&d, "claude:1", 101).0, d, "nothing to take changes nothing");
        assert_eq!(take_next(&Value::Null, "claude:1", 1).1, None);
    }

    #[test]
    fn two_takers_on_one_store_never_take_the_same_item_and_a_handed_over_store_gives_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Mutex::new(store::Store::open(dir.path(), std::time::Duration::ZERO));
        store.lock().unwrap().update("queue", |_| doc(&["one", "two", "three", "four", "five", "six"]));
        let took = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    loop {
                        let mut taken = None;
                        store.lock().unwrap().update("queue", |d| {
                            let (next, item) = take_next(d, "claude:1", 1);
                            taken = item;
                            next
                        });
                        let Some(item) = taken else { break };
                        took.lock().unwrap().push(item["text"].as_str().unwrap().to_string());
                    }
                });
            }
        });
        let mut took = took.into_inner().unwrap();
        took.sort();
        assert_eq!(took, ["five", "four", "one", "six", "three", "two"], "each once");
        let mut handed = store.into_inner().unwrap();
        handed.update("queue", |_| doc(&["one"]));
        handed.close();
        let mut taken = None;
        handed.update("queue", |d| {
            let (next, item) = take_next(d, "claude:1", 1);
            taken = item;
            next
        });
        assert_eq!(taken, None, "a store handed over takes nothing");
    }

    #[test]
    fn a_failed_send_puts_back_only_its_own_items_where_they_stood() {
        // Another agent's item went meanwhile: putting A's back must not bring B's back too.
        let mut d = doc(&["a1", "a2", "a3"]);
        d["queues"]["claude:2"] = json!([{"id": "b1", "text": "b1", "at": 0}]);
        let (after_a, items, at) = take_for_send(&d, "claude:1", "q1").unwrap();
        assert_eq!((texts(&after_a, "claude:1"), at), (vec!["a1".to_string(), "a3".to_string()], 1));
        let (after_b, _) = take_next(&after_a, "claude:2", 5);
        let back = put_back(&after_b, "claude:1", &items, at, None, 6);
        assert_eq!(texts(&back, "claude:1"), ["a1", "a2", "a3"], "back where it stood");
        assert!(back["queues"]["claude:2"].is_null(), "B's item stays gone");
        assert!(back["paused"]["claude:1"].is_null(), "a send by hand that fails pauses nothing");
        assert_eq!(texts(&put_back(&back, "claude:1", &items, 0, None, 7), "claude:1"), ["a1", "a2", "a3"], "never twice");
    }

    #[test]
    fn a_failed_turn_holds_the_queue_until_resumed_or_a_turn_ends_well() {
        let d = paused(&doc(&["run the tests"]), "claude:1", FAILED, 5);
        assert_eq!(d["paused"]["claude:1"], json!({"why": FAILED, "at": 5}));
        assert_eq!(take_next(&d, "claude:1", 6).1, None, "paused: nothing goes");
        assert_eq!(take_next(&unpaused(&d, "claude:1", None), "claude:1", 6).1.unwrap()["text"], "run the tests", "resumed");
        assert!(paused(&json!({"queues": {}}), "claude:1", FAILED, 5)["paused"].is_null(), "an empty queue has nothing to hold");
        let mut edited = Map::new();
        edited.insert("claude:1".into(), d["queues"]["claude:1"].clone());
        edit(&mut edited, "add", &[json!("claude:1"), json!("then push")], 7).unwrap();
        assert!(!d["paused"]["claude:1"].is_null() && edited["claude:1"].as_array().unwrap().len() == 2, "an add keeps the pause");
    }

    #[test]
    fn a_stop_right_after_a_prompt_went_on_its_own_puts_it_back_first() {
        let (d, item) = take_next(&doc(&["run the tests", "then push"]), "claude:1", 1_000);
        assert_eq!(item.unwrap()["text"], "run the tests");
        let soon = stopped(&d, "claude:1", 3_000);
        assert_eq!(texts(&soon, "claude:1"), ["run the tests", "then push"], "the stop came right after it went");
        assert_eq!(soon["paused"]["claude:1"]["why"], STOPPED);
        assert!(soon["sent"]["claude:1"].is_null(), "and it is not put back twice");
        let late = stopped(&d, "claude:1", 1_000 + UNDO_MS + 1);
        assert_eq!(texts(&late, "claude:1"), ["then push"], "a stop long after: the prompt had its turn");
        let had_its_turn = stopped(&retired(&d, "claude:1", 2_000), "claude:1", 3_000);
        assert_eq!(texts(&had_its_turn, "claude:1"), ["then push"], "a turn ended since it went: the stop is a later turn's");
    }

    #[test]
    fn an_ended_session_takes_its_queue_with_it() {
        let mut d = paused(&doc(&["run the tests"]), "claude:1", STOPPED, 1);
        d["owners"]["claude:1"] = json!("00000000-acme-4000-8000-000000000421");
        let (d, _) = take_next(&unpaused(&d, "claude:1", None), "claude:1", 2);
        let gone = forgotten(&d, "claude:1");
        for part in ["queues", "paused", "owners", "sent"] {
            assert!(gone[part]["claude:1"].is_null(), "{part}");
        }
    }

    #[test]
    fn a_queue_written_for_another_agent_is_not_its_successors() {
        let mut d = doc(&["run the tests"]);
        d["owners"]["claude:1"] = json!("claude@/r/api");
        assert!(owned(&d, "claude:1", Some("claude@/r/api")));
        assert!(!owned(&d, "claude:1", Some("codex@/r/web")), "the terminal runs another agent now");
        assert!(owned(&d, "claude:1", None), "an agent nobody can name now is not refused");
        let resumed = unpaused(&paused(&d, "claude:1", PARKED, 1), "claude:1", Some("codex@/r/web"));
        assert!(owned(&resumed, "claude:1", Some("codex@/r/web")), "resumed: it is this agent's now");
    }

    #[test]
    fn a_prompt_goes_on_its_own_only_after_a_rest_that_followed_the_last() {
        assert_eq!(rest_after(1_000, 5_000, false), 1_000, "it must rest after the last went");
        assert_eq!(rest_after(1_000, 1_000 + SENT_HOLDS_MS + 1, false), 0, "a last one long gone holds nothing");
        assert_eq!(rest_after(1_000, 5_000, true), 0, "the person asks now");
    }

    #[test]
    fn told_words_go_now_unless_a_running_queue_is_ahead_of_them() {
        assert!(tell_now(true, 0, true), "ready, nothing waits");
        assert!(tell_now(true, 2, false), "the queue is held: these go, the older ones wait");
        assert!(!tell_now(true, 2, true), "the queue runs: they go in order");
        assert!(!tell_now(false, 0, true), "not ready: queued");
    }

    #[test]
    fn tell_says_what_happened_to_the_oldest() {
        assert_eq!(behind(2, Some(Ok(()))).unwrap(), "queued behind 2: the oldest went now, the rest follow as it rests");
        let asking = cockpit_core::deliver::waiting_note("Claude Code");
        assert!(behind(2, Some(Err(asking))).unwrap().contains("waits on a question"));
        let kept = behind(2, Some(Err("app:t3 did not take it".into()))).unwrap();
        assert!(kept.starts_with("queued") && kept.contains("did not take it"), "added, the words are queued: never a refusal to send again");
        assert!(behind(2, None).unwrap().contains("once the agent rests"));
    }

    #[test]
    fn the_state_line_says_what_will_happen() {
        let view = json!({"paused": {"claude:1": {"why": STOPPED}}, "drain": true});
        assert_eq!(state_line(&view, "claude:1"), format!("Paused — {STOPPED} · :r resumes"));
        assert_eq!(state_line(&json!({"drain": false}), "claude:1"), "Sent by hand only: queue.drain is off");
        assert_eq!(state_line(&json!({"drain": true}), "claude:1"), "Each goes once the agent rests");
        let asleep = json!({"drain": true, "asleep": {"claude:1": true}});
        assert_eq!(state_line(&asleep, "claude:1"), "Its session is paused: nothing goes on its own · :s sends, which picks it up");
    }
}
