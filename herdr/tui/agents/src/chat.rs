//! The `chat` view: one agent's conversation as a reader shows it, read from the transcript its terminal holds —
//! the messages in order, the tool calls between them, and the question it is waiting on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use cockpit_core::transcripts::{self, Detail, Follow, Role, Task, TaskKind, TaskState, Transcript, Transcripts, Usage};
use serde_json::{Value, json};

use crate::dialog::{self, Dialog, Kind};
use crate::prompt::{self, Activity, Shapes};

/// The most recent messages a chat holds unless it is asked for more; `more` says there are older ones.
const WINDOW: usize = 400;
/// However much is asked for, a chat never reads past this.
const CEILING: usize = 20_000;
/// How many transcripts are followed at once: the chats open in a window, and those just left.
const KEPT: usize = 24;
/// About how much they may hold together, in bytes of words and call summaries; past it the ones left longest ago go.
const BUDGET: usize = 96 << 20;

/// The transcripts chats follow, the last read first: each is read on from where it stopped, not again whole.
static FOLLOWED: Mutex<Vec<(PathBuf, Follow)>> = Mutex::new(Vec::new());

/// How the hub builds the chat view again, set once by the hub.
static GROWN: OnceLock<fn()> = OnceLock::new();

/// A terminal agent some chat shows: its agent (for its screen's shapes), whether it works, and when a chat last asked
/// for it.
#[derive(Clone, Debug)]
struct Subject {
    agent: String,
    working: bool,
    seen: Instant,
}

/// What a subject's screen last said: the step it is on, the dialog or completion menu it shows, and what is typed in
/// its box.
#[derive(Clone, Debug, Default, PartialEq)]
struct Read {
    activity: Activity,
    dialog: Option<Dialog>,
    input: Option<String>,
}

impl Read {
    /// Whether the screen holds something that moves on each key: a dialog, a menu.
    fn live(&self) -> bool {
        self.dialog.is_some()
    }
}

/// The chats' subjects by handle, and what was last read off their screens: the growth thread reads them between
/// builds, so a build never waits on a terminal.
static SUBJECTS: Mutex<Option<HashMap<String, Subject>>> = Mutex::new(None);
static SCREENS: Mutex<Option<HashMap<String, Read>>> = Mutex::new(None);

/// How long a subject no chat asks for keeps its screen read.
const SUBJECT_FOR: Duration = Duration::from_secs(10);
/// The growth thread's tick; a working agent's screen is read every eighth, a live one's every other.
const TICK: Duration = Duration::from_millis(250);

/// Builds the chat view again whenever a transcript it follows grows, and whenever a subject's screen says something
/// new: a long turn writes for minutes while its agent's status, which is what moves the agents view, stays the same.
pub fn on_growth(touch: fn()) {
    if GROWN.set(touch).is_err() {
        return;
    }
    std::thread::spawn(|| {
        let mut stamps: HashMap<PathBuf, (u64, SystemTime)> = HashMap::new();
        let mut tick: u64 = 0;
        loop {
            std::thread::sleep(TICK);
            tick += 1;
            let paths: Vec<PathBuf> = FOLLOWED.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(p, _)| p.clone()).collect();
            let grew = grown(&mut stamps, &paths);
            let said = refresh_screens(tick, &|handle| crate::term::screen(handle, 0));
            if (grew || said)
                && let Some(touch) = GROWN.get()
            {
                touch();
            }
        }
    });
}

/// Whether any of these files changed since the last look; a file seen for the first time has not.
fn grown(stamps: &mut HashMap<PathBuf, (u64, SystemTime)>, paths: &[PathBuf]) -> bool {
    let mut grew = false;
    for path in paths {
        let Ok(meta) = std::fs::metadata(path) else { continue };
        let now = (meta.len(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        grew |= stamps.insert(path.clone(), now).is_some_and(|before| before != now);
    }
    stamps.retain(|p, _| paths.contains(p));
    grew
}

/// Reads the screens due at this tick — a subject's first, a working one's every 2 s, and every 500 ms one showing a
/// dialog or a menu, or that cockpit is typing into — and says whether any of them changed. Subjects nobody asked for
/// lately are let go.
fn refresh_screens(tick: u64, read: &dyn Fn(&str) -> Option<String>) -> bool {
    let now = Instant::now();
    let due: Vec<(String, String)> = {
        let mut subjects = SUBJECTS.lock().unwrap_or_else(|e| e.into_inner());
        let subjects = subjects.get_or_insert_with(HashMap::new);
        subjects.retain(|_, s| now.duration_since(s.seen) < SUBJECT_FOR);
        let mut screens = SCREENS.lock().unwrap_or_else(|e| e.into_inner());
        let screens = screens.get_or_insert_with(HashMap::new);
        screens.retain(|h, _| subjects.contains_key(h));
        subjects
            .iter()
            .filter(|(h, s)| {
                let live = screens.get(*h).is_some_and(Read::live) || crate::gui::mirroring(h);
                !screens.contains_key(*h) || (live && tick.is_multiple_of(2)) || (s.working && tick.is_multiple_of(8))
            })
            .map(|(h, s)| (h.clone(), s.agent.clone()))
            .collect()
    };
    let mut changed = false;
    for (handle, agent) in due {
        let Some(screen) = read(&handle) else { continue };
        let shapes = Shapes::of(&agent);
        let shown =
            Read { activity: prompt::activity(&screen, &shapes), dialog: dialog::read(&screen, &shapes), input: prompt::typed(&screen).map(|d| typed_of(&d)) };
        let mut screens = SCREENS.lock().unwrap_or_else(|e| e.into_inner());
        let screens = screens.get_or_insert_with(HashMap::new);
        changed |= screens.get(&handle) != Some(&shown);
        screens.insert(handle, shown);
    }
    changed
}

/// What was typed in the box, without the argument hint an agent greys in after a completed command (`/model  [model]`).
fn typed_of(draft: &str) -> String {
    match draft.find("  [") {
        Some(at) if draft.ends_with(']') => draft[..at].to_string(),
        _ => draft.to_string(),
    }
}

/// Notes that a chat shows this terminal, and gives what its screen last said; the growth thread reads it from now on.
fn subject(handle: &str, agent: &str, working: bool) -> Read {
    let mut subjects = SUBJECTS.lock().unwrap_or_else(|e| e.into_inner());
    subjects.get_or_insert_with(HashMap::new).insert(handle.to_string(), Subject { agent: agent.to_string(), working, seen: Instant::now() });
    SCREENS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|s| s.get(handle).cloned()).unwrap_or_default()
}

/// A dialog as the chat view sends it, with the fingerprint an answer to a list must bring back; a completion menu goes
/// out as `menu` instead.
fn dialog_json(handle: &str, dialog: Option<&Dialog>) -> (Value, Value) {
    match dialog {
        Some(d) if d.kind == Kind::Menu => {
            let items: Vec<Value> = d.options.iter().map(|o| json!({"label": o.label, "hint": o.hint})).collect();
            let selected = d.options.iter().position(|o| o.selected).unwrap_or(0);
            (Value::Null, json!({"items": items, "selected": selected}))
        }
        Some(d) => {
            let mut out = d.json();
            out["asked"] = d.question.as_ref().map_or(Value::Null, |q| json!(crate::gui::asked_fingerprint(handle, q)));
            (out, Value::Null)
        }
        None => (Value::Null, Value::Null),
    }
}

/// The folders the view reads: none of its own, it follows the agents poller.
pub fn dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// One agent's conversation: `{agent, session: {id, cwd, branch, model, effort, fast, mode, focus, switch, repos, task, detail,
/// dialog, menu, input, context, cost}, messages, cursor, question, more, next, tasks}`, for the agent `terminal` names (a
/// handle, an address or a driven session's key); empty when it names none the agents view lists. `limit` is how many
/// messages to send, `WINDOW` by default, the latest or those before message `before`; `agent` is the agents view's row,
/// so a chat needs no second subscription. `dialog` is what a command opened in its terminal, `menu` the completion of
/// the slash command typed there, `input` what its box holds. `tasks` lists the subagents, background jobs and
/// workflows its turns started; with `subagent: <id>`, the view is that subagent's own conversation instead.
pub fn view(args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    // The chat's subject, and only that: another agent's conversation is never shown in its place.
    let wanted = args["terminal"].as_str().filter(|t| !t.is_empty());
    let Some(row) = wanted.and_then(|t| crate::gui::row_of(t, args["frontend"].as_str(), doc)) else {
        return json!({"agent": Value::Null, "session": Value::Null, "messages": [], "cursor": Value::Null, "question": Value::Null, "more": false, "next": Value::Null, "tasks": []});
    };
    let session = row["session"].as_str().unwrap_or_default();
    let cwd = row["cwd"].as_str().unwrap_or_default();
    let found = transcript_of(&row);
    let window = args["limit"].as_u64().map(|n| (n as usize).clamp(1, CEILING)).unwrap_or(WINDOW);
    let before = args["before"].as_u64().map(|n| n as usize);
    let terminal = row["driven"] != true;
    let handle = row["terminal"].as_str().unwrap_or_default();
    // A driven session's tasks come from its stream; a terminal's from its transcript. Either way a subagent's own
    // file is looked for beside the transcript, so it can be opened.
    let (page, mut tasks) = match (terminal, &found) {
        (true, Some((kind, path))) => {
            let page = latest(*kind, path, window, before, false);
            let tasks = page.as_ref().map(|p| p.tasks.clone()).unwrap_or_default();
            (page, tasks)
        }
        (false, _) => {
            let mut tasks: Vec<Task> =
                crate::session::summary(handle).and_then(|s| s["tasks"].as_array().cloned()).unwrap_or_default().iter().filter_map(task_of_json).collect();
            if let Some((kind, path)) = &found {
                let parent = Transcript { cwd: cwd.to_string(), ..Transcript::default() };
                transcripts::match_subagents(&mut tasks, &transcripts::subagents_of(*kind, path, &parent));
            }
            (None, tasks)
        }
        _ => (None, Vec::new()),
    };
    if let Some(id) = args["subagent"].as_str().filter(|id| !id.is_empty()) {
        let Some((kind, _)) = found else { return subagent_view(row, None, id, window, before) };
        let child = tasks.iter_mut().find(|t| t.id == id || t.tool_use_id == id);
        return subagent_view(row, child.map(|t| (kind, t.clone())), id, window, before);
    }
    let tasks: Vec<Value> = tasks.iter().map(|t| task_json(t, handle)).collect();
    let Page { messages, cursor, usage, ran, .. } = page.unwrap_or_default();
    let next = crate::next::of(&row, &messages, &cockpit_core::files::config(), ran.model.as_deref(), GROWN.get().copied());
    let working = row["status"] == "working";
    // What its terminal shows now: the step it is on while it works, and its mode, which the transcript says a turn late.
    let read = if terminal { subject(handle, row["agent"].as_str().unwrap_or_default(), working) } else { Read::default() };
    let shown = read.activity;
    // A dialog shows only while no question card does: a permission ask the poller saw is the card's.
    let (dialog, menu) = if row["question"].is_null() { dialog_json(handle, read.dialog.as_ref()) } else { (Value::Null, Value::Null) };
    // What a click in the chat may change in its terminal: the agent's mode cycle and focus view, where measured.
    let program = row["program"].as_str().unwrap_or_default();
    let known = cockpit_core::agent::of_program(program).or_else(|| cockpit_core::agent::get(row["agent"].as_str().unwrap_or_default()));
    let switch = json!({
        "mode": terminal && known.as_ref().is_some_and(|a| a.cycle.is_some()),
        "focus": terminal && known.as_ref().is_some_and(|a| a.focus.is_some()),
    });
    json!({
        "agent": row,
        "session": {
            "id": session, "cwd": cwd, "branch": row["branch"].clone(),
            "model": ran.model.map(Value::from).unwrap_or(Value::Null),
            "effort": ran.effort.map(Value::from).unwrap_or(Value::Null),
            "fast": ran.speed.map(|s| Value::from(s == "fast")).unwrap_or(Value::Null),
            "mode": shown.mode.or(ran.mode).map(Value::from).unwrap_or(Value::Null),
            "focus": ran.focus.map(Value::from).unwrap_or(Value::Null),
            "repos": repos(cwd, &messages),
            "switch": switch,
            "task": shown.task.filter(|_| working).map(Value::from).unwrap_or(Value::Null),
            "detail": shown.detail.filter(|_| working).map(Value::from).unwrap_or(Value::Null),
            "dialog": dialog, "menu": menu, "input": read.input.map(Value::from).unwrap_or(Value::Null),
            "context": usage.context_percent(), "cost": usage.cost,
        },
        "messages": messages,
        "question": row["question"].clone(),
        "more": cursor["more"] == true,
        "cursor": cursor,
        "next": next,
        "tasks": tasks,
    })
}

/// A subagent's conversation in the chat's shape: the parent's row and facts, `session.subagent` naming the task
/// (`{id, name, state, started, ended, parent, title}`), its own messages paged like a parent's, and nothing to
/// answer or send.
fn subagent_view(row: Value, child: Option<(Transcripts, Task)>, id: &str, window: usize, before: Option<usize>) -> Value {
    let handle = row["terminal"].as_str().unwrap_or_default().to_string();
    let page = child.as_ref().and_then(|(kind, t)| t.transcript.as_ref().and_then(|p| latest(*kind, p, window, before, true)));
    let task = child.as_ref().map(|(_, t)| t);
    let subagent = json!({
        "id": id,
        "name": task.map(|t| t.name.clone()).unwrap_or_default(),
        "state": task.map(|t| t.state.word()).unwrap_or("done"),
        "started": task.map(|t| t.started.clone()),
        "ended": task.and_then(|t| t.ended.clone()),
        "summary": task.and_then(|t| t.summary.clone()),
        "parent": handle,
        "title": row["title"].clone(),
    });
    let Page { messages, cursor, usage, ran, .. } = page.unwrap_or_default();
    json!({
        "agent": row.clone(),
        "session": {
            "id": row["session"].clone(), "cwd": row["cwd"].clone(), "branch": row["branch"].clone(),
            "model": ran.model.map(Value::from).unwrap_or(Value::Null),
            "effort": ran.effort.map(Value::from).unwrap_or(Value::Null),
            "fast": ran.speed.map(|s| Value::from(s == "fast")).unwrap_or(Value::Null),
            "mode": Value::Null, "focus": Value::Null, "repos": [], "switch": {"mode": false, "focus": false},
            "task": Value::Null, "detail": Value::Null, "dialog": Value::Null, "menu": Value::Null, "input": Value::Null,
            "context": usage.context_percent(), "cost": usage.cost,
            "subagent": subagent,
        },
        "messages": messages,
        "question": Value::Null,
        "more": cursor["more"] == true,
        "cursor": cursor,
        "next": Value::Null,
        "tasks": [],
    })
}

/// A task as the views send it: `{id, kind, name, state, started, ended, summary, tool_use_id}`, RFC 3339 times.
pub(crate) fn task_json_plain(t: &Task) -> Value {
    json!({"id": t.id, "kind": t.kind.word(), "name": t.name, "state": t.state.word(), "started": t.started, "ended": t.ended, "summary": t.summary, "tool_use_id": t.tool_use_id})
}

/// The same with how to open it: `open: {terminal, subagent}` for a subagent whose transcript was found, else null.
fn task_json(t: &Task, terminal: &str) -> Value {
    let mut out = task_json_plain(t);
    out["open"] = t.transcript.as_ref().map_or(Value::Null, |_| json!({"terminal": terminal, "subagent": t.id}));
    out
}

/// A task back from the shape the views send, as a session keeps them.
fn task_of_json(v: &Value) -> Option<Task> {
    let kind = match v["kind"].as_str()? {
        "subagent" => TaskKind::Subagent,
        "background" => TaskKind::Background,
        "workflow" => TaskKind::Workflow,
        _ => return None,
    };
    let state = match v["state"].as_str().unwrap_or("running") {
        "done" => TaskState::Done,
        "failed" => TaskState::Failed,
        _ => TaskState::Running,
    };
    Some(Task {
        id: v["id"].as_str()?.to_string(),
        kind,
        name: v["name"].as_str().unwrap_or_default().to_string(),
        state,
        started: v["started"].as_str().unwrap_or_default().to_string(),
        ended: v["ended"].as_str().map(str::to_string),
        tool_use_id: v["tool_use_id"].as_str().unwrap_or_default().to_string(),
        summary: v["summary"].as_str().map(str::to_string),
        transcript: None,
    })
}

/// The row's transcript and its format, as the agents view found it: its agent's own format, else what the file says.
fn transcript_of(row: &Value) -> Option<(Transcripts, PathBuf)> {
    let path = PathBuf::from(row["transcript"].as_str().filter(|p| !p.is_empty())?);
    let own = cockpit_core::agent::get(row["agent"].as_str().unwrap_or_default()).map(|a| a.transcripts).filter(|k| *k != Transcripts::None);
    let kind = own.or_else(|| transcripts::kind_of(&path))?;
    Some((kind, path))
}

/// The most repositories a chat names as worked in.
const REPOS: usize = 8;

/// The git repositories the agent works in, the latest touched first, as `[{path, name}]`: its folder's own, and those its
/// calls changed or went into (a file written, a `cd`, a path under its folder in a command), never one it only read.
fn repos(cwd: &str, messages: &[Value]) -> Value {
    let home = std::env::var("HOME").unwrap_or_default();
    let full = |p: &str| -> Option<PathBuf> {
        let p = p.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | ',' | ';' | ')' | '('));
        let p = match p.strip_prefix("~/") {
            Some(rest) if !home.is_empty() => format!("{home}/{rest}"),
            _ => p.to_string(),
        };
        p.starts_with('/').then(|| PathBuf::from(p))
    };
    let mut found: Vec<PathBuf> = Vec::new();
    let mut note = |top: Option<PathBuf>| {
        if let Some(top) = top {
            found.retain(|t| *t != top);
            found.push(top);
        }
    };
    note(toplevel(Path::new(cwd)));
    for m in messages {
        let Some(summary) = m["tool"]["summary"].as_str() else { continue };
        let name = m["tool"]["name"].as_str().unwrap_or_default();
        if matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
            note(full(summary).and_then(|p| toplevel(&p)));
            continue;
        }
        // Reading a project is no work in it.
        if matches!(name, "Read" | "Grep" | "Glob" | "LS" | "NotebookRead" | "WebFetch" | "WebSearch") {
            continue;
        }
        let words: Vec<&str> = summary.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            let Some(p) = full(w) else { continue };
            let went = i > 0 && words[i - 1] == "cd";
            if went || (!cwd.is_empty() && p.starts_with(cwd)) {
                note(toplevel(&p));
            }
        }
    }
    let list: Vec<Value> = found
        .iter()
        .rev()
        .take(REPOS)
        .map(|t| json!({"path": t.to_string_lossy(), "name": t.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()}))
        .collect();
    Value::from(list)
}

/// The top of the git repository `path` is in, found by its `.git` (a folder, or a worktree's file); None outside one.
fn toplevel(path: &Path) -> Option<PathBuf> {
    static SEEN: Mutex<Option<HashMap<PathBuf, Option<PathBuf>>>> = Mutex::new(None);
    if path.as_os_str().is_empty() {
        return None;
    }
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let seen = seen.get_or_insert_with(HashMap::new);
    if let Some(top) = seen.get(path) {
        return top.clone();
    }
    let top = path.ancestors().find(|dir| dir.join(".git").exists()).filter(|dir| dir.parent().is_some()).map(Path::to_path_buf);
    if seen.len() > 4096 {
        seen.clear();
    }
    seen.insert(path.to_path_buf(), top.clone());
    top
}

/// What a transcript says its last turn ran with.
#[derive(Default)]
struct Ran {
    model: Option<String>,
    effort: Option<String>,
    speed: Option<String>,
    mode: Option<String>,
    focus: Option<bool>,
}

/// A page of a transcript, as `latest` cuts it.
#[derive(Default)]
struct Page {
    messages: Vec<Value>,
    cursor: Value,
    usage: Usage,
    ran: Ran,
    /// Its tasks, each subagent's file laid on it when found beside the transcript.
    tasks: Vec<Task>,
}

/// A page of a transcript's messages — the `window` latest, or those before message `before` — with where it sits,
/// `cursor: {from, to, total, more}`, the transcript's token use, what it ran with and its tasks. A subagent's own
/// file is followed with `child`.
fn latest(kind: Transcripts, path: &Path, window: usize, before: Option<usize>, child: bool) -> Option<Page> {
    let mut followed = FOLLOWED.lock().unwrap_or_else(|e| e.into_inner());
    let mut entry = match followed.iter().position(|(p, _)| p == path) {
        Some(at) => followed.remove(at),
        None => (path.to_path_buf(), if child { Follow::subagent(kind, path, Detail::Tools) } else { Follow::new(kind, path, Detail::Tools) }),
    };
    let out = entry.1.read().map(|t| {
        let total = t.messages.len();
        let to = before.unwrap_or(total).min(total);
        let from = to.saturating_sub(window);
        let ran = Ran { model: t.model.clone(), effort: t.effort.clone(), speed: t.speed.clone(), mode: t.mode.clone(), focus: t.focus };
        let cursor = json!({"from": from, "to": to, "total": total, "more": from > 0});
        let mut tasks = t.tasks.clone();
        if !child && tasks.iter().any(|t| t.kind == TaskKind::Subagent) {
            transcripts::match_subagents(&mut tasks, &transcripts::subagents_of(kind, path, t));
        }
        Page { messages: t.messages[from..to].iter().map(message).collect(), cursor, usage: t.usage.clone(), ran, tasks }
    });
    followed.insert(0, entry);
    trim(&mut followed);
    out
}

/// Keeps the transcripts read last, as many as `KEPT` and about `BUDGET` bytes of them.
fn trim(followed: &mut Vec<(PathBuf, Follow)>) {
    followed.truncate(KEPT);
    let mut held = 0usize;
    followed.retain(|(_, f)| {
        held += f.bytes();
        held <= BUDGET
    });
}

/// A message as the chat draws it: words, or the tool call that stood there.
pub(crate) fn message(m: &transcripts::Message) -> Value {
    json!({
        "role": if m.role == Role::User { "user" } else { "assistant" },
        "at": m.at,
        "text": m.text,
        "tool": m.tool.as_ref().map(|t| json!({"id": t.id, "name": t.name, "summary": t.summary, "ok": t.ok, "about": t.about})).unwrap_or(Value::Null),
    })
}

/// `task-stop {terminal, task}` stops a task of a session cockpit drives; a terminal's is stopped in its terminal. A
/// chat answers, replies and snoozes through the `agents` view.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let arg = args.first().cloned().unwrap_or(Value::Null);
    match action {
        "task-stop" => {
            let terminal = arg["terminal"].as_str().unwrap_or_default();
            let task = arg["task"].as_str().unwrap_or_default();
            if terminal.is_empty() || task.is_empty() {
                return Some(Err("which task? pass {terminal, task}".into()));
            }
            // A terminal's task is its agent's own to stop; a driven session's goes over its stream, by its owner or a person.
            if crate::session::summary(terminal).is_none() {
                return Some(Err("stop it in its terminal".into()));
            }
            Some(crate::gui::may_touch(terminal).and_then(|()| crate::session::stop_task(terminal, task)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_core::transcripts::{Message, Tool};

    #[test]
    fn a_message_carries_its_words_or_the_call_that_stood_there() {
        let said = Message { role: Role::User, text: "Fix the rounding".into(), at: "2026-11-03T10:00:00Z".into(), tool: None };
        assert_eq!(message(&said), json!({"role": "user", "at": "2026-11-03T10:00:00Z", "text": "Fix the rounding", "tool": null}));

        let ran = Message {
            role: Role::Assistant,
            text: String::new(),
            at: "2026-11-03T10:01:00Z".into(),
            tool: Some(Tool { id: "t1".into(), name: "Bash".into(), summary: "mix test".into(), ok: Some(false), about: None }),
        };
        assert_eq!(
            message(&ran),
            json!({"role": "assistant", "at": "2026-11-03T10:01:00Z", "text": "", "tool": {"id": "t1", "name": "Bash", "summary": "mix test", "ok": false, "about": null}})
        );
    }

    /// The view's build on a 5,000-message transcript: a whole read as it was, then a followed one, untouched and
    /// after a turn more. `cargo test -p cockpit-agents --release chat::tests::measure -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn measure_a_5000_message_chat() {
        use std::io::Write;
        use std::time::Instant;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.jsonl");
        let turn = |n: usize| {
            let at = format!("2026-11-03T10:{:02}:{:02}Z", (n / 60) % 60, n % 60);
            let call = |id: String, command: String| json!({"type": "assistant", "timestamp": at, "message": {"content": [{"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command}}]}});
            let result = |id: String| json!({"type": "user", "timestamp": at, "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": "ok"}]}});
            [
                json!({"type": "user", "cwd": "/r/api", "sessionId": "long", "timestamp": at, "message": {"role": "user", "content": format!("Turn {n}: make the daily quota configurable per plan, and run the quota tests when it builds.")}}),
                call(format!("a{n}"), format!("mix test test/api/quota_test.exs:{n}")),
                result(format!("a{n}")),
                call(format!("b{n}"), "rg daily_limit lib".into()),
                result(format!("b{n}")),
                json!({"type": "assistant", "timestamp": at, "message": {"content": [{"type": "text", "text": format!("Turn {n} done: the limit now reads from each plan's settings, with the current value as the default, and the quota tests pass.")}]}}),
            ]
            .map(|l| format!("{l}\n"))
            .concat()
        };
        std::fs::write(&path, (0..1250).map(turn).collect::<String>()).unwrap();
        let time = |what: &str, runs: u32, f: &mut dyn FnMut() -> usize| {
            let start = Instant::now();
            let mut n = 0;
            for _ in 0..runs {
                n = f();
            }
            println!("{what}: {:.2} ms ({n} messages sent)", start.elapsed().as_secs_f64() * 1000.0 / f64::from(runs));
        };
        time("whole read, as every build was", 5, &mut || {
            let t = transcripts::read_detail(Transcripts::Claude, &path, Detail::Tools).unwrap();
            assert_eq!(t.messages.len(), 5000);
            t.messages[t.messages.len() - WINDOW..].iter().map(message).count()
        });
        time("followed, first read", 1, &mut || latest(Transcripts::Claude, &path, WINDOW, None, false).unwrap().messages.len());
        time("followed, nothing new", 20, &mut || latest(Transcripts::Claude, &path, WINDOW, None, false).unwrap().messages.len());
        let mut n = 1250;
        time("followed, one turn more each time", 20, &mut || {
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(turn(n).as_bytes()).unwrap();
            n += 1;
            latest(Transcripts::Claude, &path, WINDOW, None, false).unwrap().messages.len()
        });
    }

    #[test]
    fn a_transcript_that_grows_has_its_chat_built_again_within_about_a_second() {
        use std::io::Write;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static TOUCHED: AtomicUsize = AtomicUsize::new(0);
        on_growth(|| {
            TOUCHED.fetch_add(1, Ordering::SeqCst);
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("g.jsonl");
        let line = |n: usize| {
            format!(
                "{}\n",
                json!({"type": "user", "cwd": "/r", "sessionId": "g", "timestamp": "2026-11-03T10:00:00Z", "message": {"role": "user", "content": format!("Turn {n}")}})
            )
        };
        std::fs::write(&path, line(0)).unwrap();
        assert!(latest(Transcripts::Claude, &path, 10, None, false).is_some(), "followed now");
        // The watcher's first look sees the file; nothing in the agents view changes after that.
        std::thread::sleep(Duration::from_millis(1500));
        let before = TOUCHED.load(Ordering::SeqCst);
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(line(1).as_bytes()).unwrap();
        let started = std::time::Instant::now();
        while TOUCHED.load(Ordering::SeqCst) == before && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(TOUCHED.load(Ordering::SeqCst) > before, "built again as the transcript grew");
        assert!(started.elapsed() < Duration::from_millis(2100), "within about a second: {:?}", started.elapsed());
    }

    #[test]
    fn a_subject_s_screen_is_read_between_builds_at_its_own_pace() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let reads = AtomicUsize::new(0);
        let read = |_: &str| {
            reads.fetch_add(1, Ordering::SeqCst);
            Some("⏺ Reading quota.ex\n✻ Thinking…\n? for shortcuts".to_string())
        };
        assert_eq!(subject("app:t7", "claude", true), Read::default(), "nothing read yet: the first build waits on no terminal");
        assert!(refresh_screens(1, &read), "its first read says something");
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert!(!refresh_screens(2, &read), "an odd tick: not read again");
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert!(!refresh_screens(8, &read), "every eighth tick while it works, unchanged");
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        subject("app:t7", "claude", false);
        refresh_screens(16, &read);
        assert_eq!(reads.load(Ordering::SeqCst), 2, "at rest its screen stays as read");
        assert_eq!(subject("app:t7", "claude", true).activity.task.as_deref(), Some("Reading quota.ex"));
        // A dialog on its screen: read every other tick, until it is gone.
        let model = std::fs::read_to_string(format!("{}/tests/fixtures/screens/claude-model.txt", env!("CARGO_MANIFEST_DIR"))).unwrap();
        subject("app:t8", "claude", false);
        assert!(refresh_screens(17, &|_| Some(model.clone())));
        assert!(subject("app:t8", "claude", false).dialog.is_some());
        let reads8 = AtomicUsize::new(0);
        let again = |_: &str| {
            reads8.fetch_add(1, Ordering::SeqCst);
            Some(model.clone())
        };
        refresh_screens(18, &again);
        refresh_screens(19, &again);
        refresh_screens(20, &again);
        assert_eq!(reads8.load(Ordering::SeqCst), 2, "live, every other tick");
    }

    #[test]
    fn a_followed_transcript_that_grows_is_seen_to_grow_and_one_seen_first_is_not() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        let mut stamps = HashMap::new();
        assert!(!grown(&mut stamps, std::slice::from_ref(&path)), "seen for the first time");
        assert!(!grown(&mut stamps, std::slice::from_ref(&path)), "unchanged");
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{}\n").unwrap();
        assert!(grown(&mut stamps, std::slice::from_ref(&path)), "a line more");
        assert!(!grown(&mut stamps, &[]), "no longer followed, no longer looked at");
        assert!(stamps.is_empty());
    }

    #[test]
    fn a_chat_sends_the_latest_messages_and_says_when_there_are_older_ones() {
        let longer = |n: usize| (0..n).map(|i| Message { role: Role::User, text: format!("m{i}"), at: String::new(), tool: None }).collect::<Vec<_>>();
        let cut = |all: &[Message], window: usize| (all.len() > window, all.len().saturating_sub(window));
        assert_eq!(cut(&longer(10), 400), (false, 0), "a short session is sent whole");
        assert_eq!(cut(&longer(500), 400), (true, 100), "a long one keeps its latest 400 and says so");
        assert_eq!(cut(&longer(500), 500), (false, 0), "asking for more reaches the start");
    }

    #[test]
    fn the_repositories_an_agent_works_in_are_the_ones_it_changed_or_went_into_the_latest_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        for repo in ["api", "web", "docs"] {
            std::fs::create_dir_all(root.join(repo).join(".git")).unwrap();
        }
        let at = |p: &str| root.join(p).to_string_lossy().into_owned();
        let call = |name: &str, summary: String| json!({"role": "assistant", "text": "", "tool": {"name": name, "summary": summary, "ok": true}});
        let messages = [
            call("Edit", at("api/lib/quota.ex")),
            call("Bash", format!("cd {} && mix test", at("web"))),
            call("Read", at("docs/README.md")),
            call("Bash", format!("grep -rn quota {}", at("api/lib"))),
        ];
        let names = |v: Value| v.as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(
            names(repos(&root.to_string_lossy(), &messages)),
            ["api", "web"],
            "a file only read is no work; a command naming a path under the folder is"
        );
        assert_eq!(names(repos(&at("web"), &messages[..1])), ["api", "web"], "its own folder's repository, then the one it changed");
    }

    #[test]
    fn a_chat_lists_its_tasks_and_opens_a_subagent_s_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, lines: &[Value]| {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
            path
        };
        let parent = write(
            "-r-api/c-9.jsonl",
            &[
                json!({"type": "user", "cwd": "/r/api", "sessionId": "c-9", "timestamp": "2026-11-03T10:00:00Z", "message": {"role": "user", "content": "Find the slow test"}}),
                json!({"type": "assistant", "timestamp": "2026-11-03T10:00:05Z", "message": {"id": "m1", "content": [{"type": "tool_use", "id": "toolu_agent", "name": "Agent", "input": {"description": "Find the slow test", "subagent_type": "Explore", "prompt": "Find it"}}]}}),
                json!({"type": "user", "timestamp": "2026-11-03T10:00:06Z", "message": {"role": "user", "content": [{"tool_use_id": "toolu_agent", "type": "tool_result", "content": "Async agent launched successfully."}]}, "toolUseResult": {"isAsync": true, "agentId": "a1f2e3d4"}}),
            ],
        );
        let child = write(
            "-r-api/c-9/subagents/agent-a1f2e3d4.jsonl",
            &[
                json!({"type": "user", "isSidechain": true, "agentId": "a1f2e3d4", "sessionId": "c-9", "cwd": "/r/api", "timestamp": "2026-11-03T10:00:06Z", "message": {"role": "user", "content": "Find it"}}),
                json!({"type": "assistant", "isSidechain": true, "agentId": "a1f2e3d4", "timestamp": "2026-11-03T10:00:10Z", "message": {"id": "s1", "model": "claude-haiku-5-5", "content": [{"type": "text", "text": "It is daily_test.exs:41."}]}}),
            ],
        );
        let page = latest(Transcripts::Claude, &parent, WINDOW, None, false).unwrap();
        assert_eq!(page.messages.len(), 2);
        assert_eq!(page.messages[1]["tool"]["id"], "toolu_agent", "a call carries its id, which a task names");
        assert_eq!(page.tasks.len(), 1);
        assert_eq!(page.tasks[0].transcript.as_deref(), Some(child.as_path()), "the subagent's file is found beside the parent's");
        let listed = task_json(&page.tasks[0], "app:t7");
        assert_eq!(listed["open"], json!({"terminal": "app:t7", "subagent": "a1f2e3d4"}));
        assert_eq!(
            (listed["kind"].as_str(), listed["state"].as_str(), listed["tool_use_id"].as_str()),
            (Some("subagent"), Some("running"), Some("toolu_agent"))
        );
        assert_eq!(
            task_of_json(&listed).as_ref().map(|t| (t.kind, t.state, t.id.as_str())),
            Some((TaskKind::Subagent, TaskState::Running, "a1f2e3d4")),
            "and reads back"
        );

        let row = json!({"terminal": "app:t7", "title": "Daily quotas", "session": "c-9", "cwd": "/r/api", "branch": "main"});
        let opened = subagent_view(row.clone(), Some((Transcripts::Claude, page.tasks[0].clone())), "a1f2e3d4", WINDOW, None);
        assert_eq!(opened["agent"]["terminal"], "app:t7", "the parent's row stays");
        assert_eq!(
            opened["session"]["subagent"],
            json!({"id": "a1f2e3d4", "name": "Find the slow test", "state": "running", "started": "2026-11-03T10:00:05Z", "ended": null, "summary": null, "parent": "app:t7", "title": "Daily quotas"})
        );
        assert_eq!(opened["messages"].as_array().map(Vec::len), Some(2), "the subagent's own conversation");
        assert_eq!(opened["messages"][1]["text"], "It is daily_test.exs:41.");
        assert_eq!(opened["session"]["model"], "claude-haiku-5-5");
        assert_eq!(
            (opened["question"].clone(), opened["next"].clone(), opened["tasks"].clone()),
            (Value::Null, Value::Null, json!([])),
            "nothing to answer, send or open from it"
        );
        let unknown = subagent_view(row, None, "nope", WINDOW, None);
        assert_eq!((unknown["messages"].as_array().map(Vec::len), unknown["session"]["subagent"]["id"].as_str()), (Some(0), Some("nope")));
        assert_eq!(
            act("task-stop", &[json!({"terminal": "app:t7", "task": "a1f2e3d4"})]),
            Some(Err("stop it in its terminal".into())),
            "a terminal's task is not cockpit's to stop"
        );
        assert_eq!(act("task-stop", &[json!({"terminal": "app:t7"})]), Some(Err("which task? pass {terminal, task}".into())));
        assert_eq!(act("snooze", &[]), None);
    }

    #[test]
    fn a_chat_for_an_agent_nobody_knows_still_answers() {
        let chat = view(&json!({"terminal": "t404", "frontend": "nothing"}), &|_| Value::Null);
        for key in ["agent", "session", "messages", "question", "more"] {
            assert!(chat.get(key).is_some(), "the view always has {key}");
        }
        assert_eq!(chat["messages"].as_array().map(Vec::len), Some(0));
        let none = view(&json!({"frontend": "nothing"}), &|_| Value::Null);
        assert_eq!(none["agent"], Value::Null, "no subject: no one else's conversation in its place");
    }
}
