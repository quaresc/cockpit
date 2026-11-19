//! The `chat` view: one agent's conversation as a reader shows it, read from the transcript its terminal holds —
//! the messages in order, the tool calls between them, and the question it is waiting on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use cockpit_core::transcripts::{self, Detail, Follow, Role, Transcripts, Usage};
use serde_json::{Value, json};

/// The most recent messages a chat holds unless it is asked for more; `more` says there are older ones.
const WINDOW: usize = 400;
/// However much is asked for, a chat never reads past this.
const CEILING: usize = 20_000;
/// How many transcripts are followed at once: the chats open in a window, and a few just left.
const KEPT: usize = 8;

/// The transcripts chats follow, the last read first: each is read on from where it stopped, not again whole.
static FOLLOWED: Mutex<Vec<(PathBuf, Follow)>> = Mutex::new(Vec::new());

/// How the hub builds the chat view again, set once by the hub.
static GROWN: OnceLock<fn()> = OnceLock::new();

/// Builds the chat view again whenever a transcript it follows grows: a long turn writes for minutes while its agent's
/// status, which is what moves the agents view, stays the same.
pub fn on_growth(touch: fn()) {
    if GROWN.set(touch).is_err() {
        return;
    }
    std::thread::spawn(|| {
        let mut stamps: HashMap<PathBuf, (u64, SystemTime)> = HashMap::new();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let paths: Vec<PathBuf> = FOLLOWED.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(p, _)| p.clone()).collect();
            let grew = grown(&mut stamps, &paths);
            if grew && let Some(touch) = GROWN.get() {
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

/// The folders the view reads: none of its own, it follows the agents poller.
pub fn dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// One agent's conversation: `{agent, session: {id, cwd, branch, model, effort, fast, context, cost}, messages, question,
/// more, next}`, for the agent `terminal` names (a handle, an address or a driven session's key); empty when it names none the
/// agents view lists. `limit` is how many of the latest messages to send, `WINDOW` by default; `agent` is the agents
/// view's row, so a chat needs no second subscription.
pub fn view(args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let agents = crate::gui::view(&json!({"frontend": args["frontend"].clone()}), doc);
    let rows = agents["agents"].as_array().cloned().unwrap_or_default();
    // The chat's subject, and only that: another agent's conversation is never shown in its place.
    let wanted = args["terminal"].as_str().filter(|t| !t.is_empty());
    let Some(row) = wanted.and_then(|t| rows.iter().find(|r| r["terminal"] == t || r["address"] == t)) else {
        return json!({"agent": Value::Null, "session": Value::Null, "messages": [], "question": Value::Null, "more": false, "next": Value::Null});
    };
    let session = row["session"].as_str().unwrap_or_default();
    let cwd = row["cwd"].as_str().unwrap_or_default();
    let found = (!session.is_empty()).then(|| transcripts::locate(session, cwd, None)).flatten();
    let window = args["limit"].as_u64().map(|n| (n as usize).clamp(1, CEILING)).unwrap_or(WINDOW);
    let (messages, more, usage, ran) = found.and_then(|(kind, path)| latest(kind, &path, window)).unwrap_or_default();
    let next = crate::next::of(row, &messages, &cockpit_core::files::config(), ran.model.as_deref(), GROWN.get().copied());
    json!({
        "agent": row,
        "session": {
            "id": session, "cwd": cwd, "branch": row["branch"].clone(),
            "model": ran.model.map(Value::from).unwrap_or(Value::Null),
            "effort": ran.effort.map(Value::from).unwrap_or(Value::Null),
            "fast": ran.speed.map(|s| Value::from(s == "fast")).unwrap_or(Value::Null),
            "context": usage.context_percent(), "cost": usage.cost,
        },
        "messages": messages,
        "question": row["question"].clone(),
        "more": more,
        "next": next,
    })
}

/// What a transcript says its last turn ran with.
#[derive(Default)]
struct Ran {
    model: Option<String>,
    effort: Option<String>,
    speed: Option<String>,
}

/// The latest `window` messages of a transcript, whether it holds older ones, its token use and what it ran with.
fn latest(kind: Transcripts, path: &Path, window: usize) -> Option<(Vec<Value>, bool, Usage, Ran)> {
    let mut followed = FOLLOWED.lock().unwrap_or_else(|e| e.into_inner());
    let mut entry = match followed.iter().position(|(p, _)| p == path) {
        Some(at) => followed.remove(at),
        None => (path.to_path_buf(), Follow::new(kind, path, Detail::Tools)),
    };
    let out = entry.1.read().map(|t| {
        let from = t.messages.len().saturating_sub(window);
        let ran = Ran { model: t.model.clone(), effort: t.effort.clone(), speed: t.speed.clone() };
        (t.messages[from..].iter().map(message).collect(), from > 0, t.usage.clone(), ran)
    });
    followed.insert(0, entry);
    followed.truncate(KEPT);
    out
}

/// A message as the chat draws it: words, or the tool call that stood there.
pub(crate) fn message(m: &transcripts::Message) -> Value {
    json!({
        "role": if m.role == Role::User { "user" } else { "assistant" },
        "at": m.at,
        "text": m.text,
        "tool": m.tool.as_ref().map(|t| json!({"name": t.name, "summary": t.summary, "ok": t.ok})).unwrap_or(Value::Null),
    })
}

/// It has no actions of its own: a chat answers, replies and snoozes through the `agents` view.
pub fn act(_action: &str, _args: &[Value]) -> Option<Result<String, String>> {
    None
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
            tool: Some(Tool { name: "Bash".into(), summary: "mix test".into(), ok: Some(false) }),
        };
        assert_eq!(
            message(&ran),
            json!({"role": "assistant", "at": "2026-11-03T10:01:00Z", "text": "", "tool": {"name": "Bash", "summary": "mix test", "ok": false}})
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
        time("followed, first read", 1, &mut || latest(Transcripts::Claude, &path, WINDOW).unwrap().0.len());
        time("followed, nothing new", 20, &mut || latest(Transcripts::Claude, &path, WINDOW).unwrap().0.len());
        let mut n = 1250;
        time("followed, one turn more each time", 20, &mut || {
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(turn(n).as_bytes()).unwrap();
            n += 1;
            latest(Transcripts::Claude, &path, WINDOW).unwrap().0.len()
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
        assert!(latest(Transcripts::Claude, &path, 10).is_some(), "followed now");
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
