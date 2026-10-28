//! An action run in the background by the hub. A call names the app, the action and its JSON arguments; its reply (the
//! note the action left, the requests it filed) comes back on a channel, so a slow herdr or git call never holds up the keys.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde_json::{Value, json};

#[derive(Debug, Default)]
pub struct Reply {
    pub tag: String,
    pub note: String,
    pub pending: Value,
    pub refreshing: Value,
    pub error: Option<String>,
}

pub struct Bridge {
    tx: Sender<Reply>,
    pub replies: Receiver<Reply>,
}

impl Default for Bridge {
    fn default() -> Self {
        let (tx, replies) = channel();
        Self { tx, replies }
    }
}

impl Bridge {
    /// Runs the app's action in the background; `armed` is the key a twice-pressed action checks, given when a dialog
    /// already asked. `tag` comes back with the reply, to tell calls apart.
    pub fn call(&self, tag: &str, app: &str, action: &str, args: Vec<Value>, armed: Option<String>) {
        let tx = self.tx.clone();
        // The asking app's pane, for an action that opens something over it, like reviewr.
        let env = json!({"HERDR_PANE_ID": std::env::var("HERDR_PANE_ID").unwrap_or_default()});
        let request = json!({"op": "act", "app": app, "action": action, "args": args, "armed": armed, "env": env});
        let (tag, unknown) = (tag.to_string(), format!("no action {app} {action}"));
        std::thread::spawn(move || {
            let _ = tx.send(reply(tag, crate::hub::request(&request, Duration::from_secs(120)), unknown));
            crate::term::wake();
        });
    }
}

/// The hub's answer as a reply; an unreachable hub or an action it does not know comes back as the reply's error.
fn reply(tag: String, answer: Result<Value, String>, unknown: String) -> Reply {
    let answer = match answer {
        Ok(answer) if answer["unknown"].as_bool() == Some(true) => Err(unknown),
        other => other,
    };
    match answer {
        Ok(answer) => Reply {
            tag,
            note: answer["note"].as_str().unwrap_or_default().to_string(),
            pending: answer["pending"].clone(),
            refreshing: answer["refreshing"].clone(),
            error: answer["error"].as_str().map(str::to_string),
        },
        Err(error) => Reply { tag, error: Some(error), ..Default::default() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unknown() -> String {
        "no action inbox dance".into()
    }

    #[test]
    fn an_answer_becomes_the_reply() {
        let answer = json!({"note": "muted docs", "pending": {}, "refreshing": {}, "error": null});
        let reply = reply("t".into(), Ok(answer), unknown());
        assert_eq!((reply.tag.as_str(), reply.note.as_str(), reply.error), ("t", "muted docs", None));
    }

    #[test]
    fn an_unknown_action_is_an_error() {
        let reply = reply("t".into(), Ok(json!({"unknown": true})), unknown());
        assert_eq!(reply.error.as_deref(), Some("no action inbox dance"));
    }

    #[test]
    fn an_unreachable_hub_is_an_error() {
        let reply = reply("t".into(), Err("the cockpit hub did not start".into()), unknown());
        assert_eq!((reply.note.as_str(), reply.error.as_deref()), ("", Some("the cockpit hub did not start")));
    }
}
