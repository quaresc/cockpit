//! Words for any agent, the one way cockpit sends them: as its next prompt in its terminal, or handed to a session
//! cockpit drives through the handler the cli registers. Nothing goes while the agent waits on a question or a
//! permission, since Enter in a menu picks its highlighted choice.

use std::sync::OnceLock;

use crate::frontend::{self, APP};

/// A session cockpit drives takes the words itself: None when the handle is not one of its sessions.
pub type Driven = fn(&str, &str) -> Option<Result<(), String>>;

/// Types into a terminal's agent through `send` unless it waits on a question, looking and typing under the lock its
/// answers take; what it says names the agent.
pub type Waiting = fn(&str, &mut dyn FnMut() -> bool) -> Result<(), String>;

static DRIVEN: OnceLock<Driven> = OnceLock::new();
static WAITING: OnceLock<Waiting> = OnceLock::new();

/// The cli hands over the sessions it drives.
pub fn on_driven(handler: Driven) {
    let _ = DRIVEN.set(handler);
}

/// The cli says how to tell an agent that waits on a question.
pub fn on_waiting(probe: Waiting) {
    let _ = WAITING.set(probe);
}

const WAITING_TAIL: &str = " is waiting on a question: answer it first";

/// What a send to an agent that waits on a question is told.
pub fn waiting_note(name: &str) -> String {
    format!("{name}{WAITING_TAIL}")
}

/// Whether a refusal is `waiting_note`'s: the agent takes the words once it is answered.
pub fn is_waiting(refusal: &str) -> bool {
    refusal.ends_with(WAITING_TAIL)
}

/// Sends `text` to the agent at `handle` as its next prompt; see the module comment. Outside the hub, which holds the
/// handlers, the hub is asked to, so every caller meets the same refusals.
pub fn deliver(handle: &str, text: &str) -> Result<(), String> {
    match DRIVEN.get() {
        Some(driven) => route(handle, text, Some(*driven), WAITING.get().copied(), frontend::alive, typed),
        _ if text.trim().is_empty() => Err("nothing to send".into()),
        _ => {
            let request = serde_json::json!({"op": "act", "app": "agents", "action": "deliver", "args": [{"terminal": handle, "text": text}]});
            let answer = crate::hub::request(&request, std::time::Duration::from_secs(30)).map_err(|e| format!("the cockpit hub: {e}"))?;
            match answer["error"].as_str() {
                Some(why) => Err(why.to_string()),
                None => Ok(()),
            }
        }
    }
}

/// Typed into the terminal and sent: `hc send`, which waits out a person's unfinished draft, or `herdr agent prompt`.
fn typed(handle: &str, text: &str) -> bool {
    match handle.strip_prefix(APP) {
        Some(terminal) => frontend::app_prompt(terminal, text),
        // herdr's own prompt for its agents: it takes lines and long text the way the agent's box does.
        None => frontend::herdr(&["agent", "prompt", handle, text]).is_some(),
    }
}

fn route(
    handle: &str,
    text: &str,
    driven: Option<Driven>,
    waiting: Option<Waiting>,
    alive: impl Fn(&str) -> bool,
    send: impl Fn(&str, &str) -> bool,
) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("nothing to send".into());
    }
    if handle.is_empty() {
        return Err("which agent? no handle given".into());
    }
    if let Some(done) = driven.and_then(|d| d(handle, text)) {
        return done;
    }
    // herdr's CLI takes no `--`: text it could read as one of its switches is not handed to it.
    if !handle.starts_with(APP) && frontend::switch_like(text) {
        return Err("herdr would read words that start with a dash as one of its options: start with another word".into());
    }
    // A pane whose agent has left would take the words as a shell command.
    if !alive(handle) {
        return Err(format!("no agent runs in {handle} any more"));
    }
    match waiting {
        Some(probe) => probe(handle, &mut || send(handle, text)),
        None if send(handle, text) => Ok(()),
        None => Err(format!("{handle} did not take it")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn driven(handle: &str, text: &str) -> Option<Result<(), String>> {
        handle.starts_with("claude:").then(|| if text == "busy" { Err(waiting_note("Claude Code")) } else { Ok(()) })
    }

    fn waiting(handle: &str, send: &mut dyn FnMut() -> bool) -> Result<(), String> {
        match handle {
            "app:t41" => Err(waiting_note("quotas")),
            _ if send() => Ok(()),
            _ => Err("billing did not take it".into()),
        }
    }

    #[test]
    fn words_go_to_the_agent_that_takes_them_and_never_into_a_question() {
        let typed = RefCell::new(Vec::new());
        let send = |h: &str, t: &str| {
            typed.borrow_mut().push(format!("{h} {t}"));
            h != "app:t404"
        };
        let alive = |h: &str| h != "app:t9";
        assert_eq!(route("app:t42", "  run the tests ", Some(driven), Some(waiting), alive, send), Ok(()));
        assert_eq!(route("claude:1794", "run the tests", Some(driven), Some(waiting), alive, send), Ok(()), "a driven session takes it");
        assert_eq!(route("claude:1794", "busy", Some(driven), Some(waiting), alive, send), Err("Claude Code is waiting on a question: answer it first".into()));
        assert_eq!(route("app:t41", "run the tests", Some(driven), Some(waiting), alive, send), Err("quotas is waiting on a question: answer it first".into()));
        assert_eq!(route("app:t404", "run the tests", Some(driven), Some(waiting), alive, send), Err("billing did not take it".into()));
        assert_eq!(
            route("app:t9", "run the tests", Some(driven), Some(waiting), alive, send),
            Err("no agent runs in app:t9 any more".into()),
            "its shell never gets it"
        );
        assert_eq!(route("app:t42", "   ", Some(driven), Some(waiting), alive, send), Err("nothing to send".into()));
        assert_eq!(route("", "run", None, None, alive, send), Err("which agent? no handle given".into()));
        assert_eq!(*typed.borrow(), ["app:t42 run the tests", "app:t404 run the tests"], "only what may go is typed, trimmed");
        assert!(route("w1:p2", "--wait for it", None, None, |_| true, send).unwrap_err().contains("start with a dash"), "herdr takes no --");
        assert_eq!(route("app:t42", "--wait for it", None, None, |_| true, send), Ok(()), "hc gets it after --");
        assert_eq!(route("w1:p2", "- fix the list", None, None, |_| true, send), Ok(()));
    }
}
