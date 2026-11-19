//! `cockpit session`: drives an agent over its streaming protocol from a shell, for working on the chat without the
//! window. A session lives as long as the command does.

use std::time::Duration;

use crate::proc::fail;
use cockpit_core::hub;
use serde_json::{Value, json};

const USAGE: &str = "An agent cockpit drives itself, over its streaming protocol.

    cockpit session run <agent> <cwd> <prompt>...   starts it, sends the prompt, prints what comes back, ends it
    cockpit session answer <ask> allow|deny         answers a permission it asks, at a terminal
    cockpit session modes                           the permission modes an agent takes";

pub fn main(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
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
        let _ = act("stop", json!({"key": key, "confirm": true}));
        fail(&why);
    }
    let mut shown: Vec<String> = Vec::new();
    for _ in 0..900 {
        std::thread::sleep(Duration::from_millis(200));
        let now = look(&key);
        for ask in now["pending"].as_array().into_iter().flatten() {
            let id = ask["id"].as_str().unwrap_or_default().to_string();
            if !shown.contains(&id) {
                println!("ASKS {}  (cockpit session answer {id} allow|deny)", serde_json::to_string(ask).unwrap_or_default());
                shown.push(id);
            }
        }
        if now["working"] == false && now["messages"].as_array().is_some_and(|m| m.len() > 1) {
            break;
        }
    }
    println!("{}", serde_json::to_string_pretty(&look(&key)).unwrap_or_default());
    let _ = act("stop", json!({"key": key, "confirm": true}));
}

/// The ask and the verdict `cockpit session answer` was given: both are required, and only at a terminal.
fn verdict(args: &[String], at_a_terminal: bool) -> Result<(String, bool), String> {
    let (Some(id), Some(word)) = (args.first(), args.get(1)) else { return Err(USAGE.into()) };
    let allow = match word.as_str() {
        "allow" => true,
        "deny" => false,
        _ => return Err(USAGE.into()),
    };
    if !at_a_terminal {
        return Err("a permission is answered by a person: run this at a terminal, or answer it in the window".into());
    }
    Ok((id.clone(), allow))
}

/// `cockpit session answer <ask> allow|deny` — what the card does in the window.
fn answer(args: &[String]) {
    let (id, allow) = verdict(args, cockpit_agents::cli::at_a_terminal()).unwrap_or_else(|e| fail(&e));
    match act("answer", json!({"ask": id, "allow": allow, "source": "cli:command"})) {
        Ok(note) => println!("{note}"),
        Err(why) => fail(&why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_permission_answer_needs_its_verdict_and_a_terminal() {
        let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(verdict(&words(&["toolu_a", "allow"]), true), Ok(("toolu_a".into(), true)));
        assert_eq!(verdict(&words(&["toolu_a", "deny"]), true), Ok(("toolu_a".into(), false)));
        assert!(verdict(&words(&["toolu_a"]), true).is_err(), "no verdict is not an allow");
        assert!(verdict(&words(&["toolu_a", "yes"]), true).is_err());
        assert!(verdict(&words(&["toolu_a", "allow"]), false).unwrap_err().contains("at a terminal"), "a script is refused");
    }
}
