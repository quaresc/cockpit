//! The `chat` view: one agent's conversation as a reader shows it, read from the transcript its terminal holds —
//! the messages in order, the tool calls between them, and the question it is waiting on.

use std::path::PathBuf;

use cockpit_core::transcripts::{self, Detail, Role};
use serde_json::{Value, json};

/// The most recent messages a chat holds unless it is asked for more; `more` says there are older ones.
const WINDOW: usize = 400;
/// However much is asked for, a chat never reads past this.
const CEILING: usize = 20_000;

/// The folders the view reads: none of its own, it follows the agents poller.
pub fn dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// One agent's conversation: `{agent, session, messages, question, more}`. `terminal` picks it, else the one the
/// agents view puts first. `limit` is how many of the latest messages to send, `WINDOW` by default. `agent` is the
/// agents view's row, so what draws a chat needs no second subscription.
pub fn view(args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let agents = crate::gui::view(&json!({"frontend": args["frontend"].clone()}), doc);
    let rows = agents["agents"].as_array().cloned().unwrap_or_default();
    // What is in front, when it is an agent at all; else the one the agents view puts first.
    let wanted = args["terminal"].as_str().filter(|t| !t.is_empty());
    let found = wanted.and_then(|t| rows.iter().find(|r| r["terminal"] == t || r["address"] == t));
    let Some(row) = found.or_else(|| rows.first()) else {
        return json!({"agent": Value::Null, "session": Value::Null, "messages": [], "question": Value::Null, "more": false});
    };
    let session = row["session"].as_str().unwrap_or_default();
    let cwd = row["cwd"].as_str().unwrap_or_default();
    let found = (!session.is_empty()).then(|| transcripts::locate(session, cwd, None)).flatten();
    let read = found.and_then(|(kind, path)| transcripts::read_detail(kind, &path, Detail::Tools));
    let window = args["limit"].as_u64().map(|n| (n as usize).clamp(1, CEILING)).unwrap_or(WINDOW);
    let (messages, more, usage, model) = match read {
        Some(t) => {
            let more = t.messages.len() > window;
            let from = t.messages.len().saturating_sub(window);
            (t.messages[from..].iter().map(message).collect::<Vec<_>>(), more, t.usage, t.model)
        }
        None => (Vec::new(), false, Default::default(), None),
    };
    json!({
        "agent": row,
        "session": {
            "id": session, "cwd": cwd, "branch": row["branch"].clone(),
            "model": model.map(Value::from).unwrap_or(Value::Null),
            "context": usage.context_percent(), "cost": usage.cost,
        },
        "messages": messages,
        "question": row["question"].clone(),
        "more": more,
    })
}

/// A message as the chat draws it: words, or the tool call that stood there.
fn message(m: &transcripts::Message) -> Value {
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
    }
}
