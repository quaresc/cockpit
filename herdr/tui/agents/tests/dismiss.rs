//! Dismissing what an agent waits on, against the demo world: it leaves what needs the person and its counts until the
//! agent waits on something else, the state file keeps it for the next hub, and Undo brings it back.

use cockpit_agents::{gui, state, term};
use cockpit_core::peer::{Sender, Who};
use serde_json::{Value, json};

const LIMIT: &str = include_str!("fixtures/screens/claude-limit-no-box.txt");
const MENU: &str = "⏺ Old accounts have no quota row yet.\n\nWhich limit should existing accounts start with?\n\n❯ 1. The plan's default\n  2. Their last 30 days' peak\n\nEnter to select · Esc to cancel";

fn agent(terminal: &str, title: &str, status: &str, since_min: u64, screen: &str) -> Value {
    json!({"terminal": terminal, "space": {"id": "sp-api", "name": "api"}, "tab": {"id": terminal, "name": title}, "title": title,
           "program": "claude", "status": status, "since_min": since_min, "cwd": "/Users/me/code/api", "screen": screen})
}

/// Changes one of the demo world's agents, as its terminal would.
fn change(terminal: &str, with: impl FnOnce(&mut Value)) {
    let mut world = term::demo().expect("the demo world");
    let found = world["agents"].as_array_mut().unwrap().iter_mut().find(|a| a["terminal"] == terminal).expect("a demo agent");
    with(found);
}

fn view() -> Value {
    gui::view(&json!({}), &|_| Value::Null)
}

fn row(view: &Value, terminal: &str) -> Value {
    view["agents"].as_array().unwrap().iter().find(|r| r["terminal"] == terminal).cloned().expect("its row")
}

fn dismiss(terminal: &str, undo: bool) -> Result<String, String> {
    gui::act("dismiss", &[json!({"terminal": terminal, "undo": undo})]).unwrap()
}

#[test]
fn a_dismissed_agent_leaves_what_needs_you_until_it_waits_on_something_else() {
    let home = tempfile::tempdir().unwrap();
    let world = home.path().join("world.json");
    let agents = [
        agent("app:t52", "Exact match for quoted keywords", "blocked", 5, LIMIT),
        agent("app:t41", "Enforce daily quotas", "blocked", 6, MENU),
        json!({"terminal": "app:t48", "space": {"id": "sp-api", "name": "api"}, "tab": {"id": "tb-417", "name": "export retries"}, "title": "Retry failed CSV exports",
               "program": "codex", "status": "failed", "error": "stream disconnected before completion", "since_min": 4, "cwd": "/Users/me/code/api", "screen": "■ stream disconnected"}),
        agent("app:t44", "Invoice CSV export", "working", 3, "⏺ Reading the review comments on billing#375"),
    ];
    std::fs::write(&world, json!({"agents": agents}).to_string()).unwrap();
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("COCKPIT_AGENTS_DEMO", &world);
    }
    // The person's own acts: the test says so, as only cockpit's own code may.
    cockpit_core::peer::trusted();
    let before = view();
    assert_eq!(
        (before["counts"]["needs_you"].as_u64(), before["counts"]["failed"].as_u64(), before["counts"]["dismissed"].as_u64()),
        (Some(2), Some(1), Some(0))
    );
    assert_eq!(before["next"], "app:t41", "the one waiting longest");
    let limit = row(&before, "app:t52");
    assert_eq!((limit["question"].clone(), limit["waits"]["text"].as_str()), (Value::Null, Some("Reached its usage limit · resets 08:00")));
    assert_eq!(limit["dismissed"], Value::Null);

    // A dismissal names the wait its card showed: another one is refused, the one shown is taken.
    let wait = limit["wait"].as_str().expect("what a dismissal names").to_string();
    assert!(wait.ends_with(":limit"), "{wait}");
    let named = |terminal: &str, wait: &str| gui::act("dismiss", &[json!({"terminal": terminal, "wait": wait})]).unwrap();
    assert_eq!(named("app:t52", "1:0123456789abcdef"), Err(cockpit_agents::session::CHANGED.into()));
    assert_eq!(row(&view(), "app:t52")["stage"], "needs_you", "refused: nothing dismissed");
    // Dismissed: out of Needs you and its counts, with its real status, neither snoozed nor seen.
    assert_eq!(named("app:t52", &wait), Ok("dismissed".into()));
    let after = view();
    let limit = row(&after, "app:t52");
    assert_eq!((limit["stage"].as_str(), limit["status"].as_str(), limit["unread"].as_bool()), (Some("dismissed"), Some("blocked"), Some(false)));
    assert!(limit["dismissed"]["at"].is_u64(), "{limit}");
    assert_eq!((limit["snoozed"].clone(), limit["seen"].as_bool()), (Value::Null, Some(false)), "not a snooze, not a look");
    assert_eq!(limit["waits"]["text"], "Reached its usage limit · resets 08:00", "what it waits on still shows where it is listed");
    assert_eq!((after["counts"]["needs_you"].as_u64(), after["counts"]["dismissed"].as_u64(), after["counts"]["total"].as_u64()), (Some(1), Some(1), Some(4)));
    assert_eq!(after["next"], "app:t41");
    assert_eq!(after["stages"].as_array().unwrap().last().unwrap(), &json!({"id": "dismissed", "label": "Dismissed", "count": 1}));
    assert_eq!(after["agents"].as_array().unwrap().last().unwrap()["terminal"], "app:t52", "listed last, after the snoozed ones");
    // Looked at again and again while nothing changes: it stays dismissed.
    gui::poll();
    gui::poll();
    assert_eq!(row(&view(), "app:t52")["stage"], "dismissed");
    // The state file holds it, for the hub that starts next.
    let kept = state::State::load(&state::path());
    let entry = &kept.agents["app:t52"];
    assert!(entry.dismissed(entry.since, "limit"), "{entry:?}");

    // A screen that cannot be read for a while says nothing new: it stays dismissed and blocked, its wait unchanged.
    change("app:t52", |a| a["screen"] = Value::Null);
    gui::poll();
    gui::poll();
    let unread = row(&view(), "app:t52");
    assert_eq!((unread["stage"].as_str(), unread["status"].as_str(), unread["wait"].as_str()), (Some("dismissed"), Some("blocked"), Some(wait.as_str())));
    change("app:t52", |a| a["screen"] = json!(LIMIT));
    gui::poll();
    // Undo brings it back, once.
    assert_eq!(dismiss("app:t52", true), Ok("back in Needs you".into()));
    assert_eq!(dismiss("app:t52", true), Err("it was not dismissed".into()));
    let back = view();
    assert_eq!(
        (row(&back, "app:t52")["stage"].as_str(), back["counts"]["needs_you"].as_u64(), back["counts"]["dismissed"].as_u64()),
        (Some("needs_you"), Some(2), Some(0))
    );
    assert!(state::State::load(&state::path()).agents["app:t52"].dismissed.is_none());

    // It asks something new in the same wait: read once it stays dismissed (a screen mid-redraw), read twice it is back.
    assert_eq!(dismiss("app:t52", false), Ok("dismissed".into()));
    change("app:t52", |a| a["screen"] = json!(MENU));
    gui::poll();
    assert_eq!(row(&view(), "app:t52")["stage"], "dismissed");
    gui::poll();
    let asks = row(&view(), "app:t52");
    assert_eq!((asks["stage"].as_str(), asks["dismissed"].clone()), (Some("needs_you"), Value::Null));
    assert_eq!(asks["question"]["text"], "Which limit should existing accounts start with?");

    // It works again and comes back with the same question: a new wait, in need of the person again.
    assert_eq!(dismiss("app:t41", false), Ok("dismissed".into()));
    assert_eq!(row(&view(), "app:t41")["stage"], "dismissed");
    change("app:t41", |a| (a["status"], a["since"]) = (json!("working"), json!(cockpit_core::seen::now_ms())));
    gui::poll();
    assert_eq!(row(&view(), "app:t41")["stage"], "working");
    change("app:t41", |a| (a["status"], a["since"]) = (json!("blocked"), json!(cockpit_core::seen::now_ms())));
    gui::poll();
    assert_eq!(row(&view(), "app:t41")["stage"], "needs_you");

    // One stopped with an error is dismissed the same way; one that waits on nothing is not.
    assert_eq!(dismiss("app:t48", false), Ok("dismissed".into()));
    let failed = view();
    assert_eq!(
        (row(&failed, "app:t48")["stage"].as_str(), row(&failed, "app:t48")["status"].as_str(), failed["counts"]["failed"].as_u64()),
        (Some("dismissed"), Some("failed"), Some(0))
    );
    assert_eq!(dismiss("app:t44", false), Err("it does not wait on you: there is nothing to dismiss".into()));
    assert_eq!(dismiss("app:t99", false), Err("no such agent".into()));

    // A person's act alone: an agent's tools dismiss nothing, their own card included.
    for sender in [Who::Agent("app:t41".into()), Who::Agent("app:t52".into()), Who::Unplaced("cockpit cannot tell who sent it".into())] {
        let refused = cockpit_core::peer::with(Sender::Placed(sender.clone()), || dismiss("app:t41", false));
        assert!(refused.is_err(), "{sender:?}: {refused:?}");
    }
    let by_agent = cockpit_core::peer::with(Sender::Placed(Who::Agent("app:t41".into())), || dismiss("app:t41", false));
    assert_eq!(by_agent, Err("a card is dismissed by a person, not by an agent's tools".into()));
    assert_eq!(row(&view(), "app:t41")["stage"], "needs_you");
}
