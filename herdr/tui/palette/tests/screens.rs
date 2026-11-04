//! The popup as drawn, its commands chosen by key, and what a found item becomes in the agent session below it.

use cockpit_core::hub::Found;
use cockpit_core::testing::render;
use cockpit_palette::{App, Command, Over, Then, agent_commands, commands, into_session};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

fn press(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn claude() -> Over {
    let pane = json!({"pane_id": "w2:p1", "agent": "claude", "agent_session": {"agent": "claude", "kind": "id", "value": "4f1c-77"},
                      "cwd": "/repos/api", "terminal_title_stripped": "Google sign-in"});
    Over::of(&pane).unwrap()
}

#[test]
fn the_pane_below_as_herdr_reports_it() {
    assert_eq!(
        claude(),
        Over { pane: "w2:p1".into(), agent: Some("claude".into()), session: Some("4f1c-77".into()), cwd: "/repos/api".into(), title: "Google sign-in".into() }
    );
    let shell = Over::of(&json!({"pane_id": "w1:p2", "cwd": "/repos"})).unwrap();
    assert_eq!((shell.agent, shell.session, shell.title.as_str()), (None, None, "w1:p2"));
    let agy = Over::of(&json!({"pane_id": "w1:p3", "agent": "agy", "agent_session": {"kind": "id", "value": "c-1"}})).unwrap();
    assert_eq!((agy.agent.as_deref(), agy.session.as_deref(), agy.forkable()), (Some("antigravity-cli"), Some("c-1"), None), "Antigravity forks only inside");
    assert_eq!(Over::of(&Value::Null), None);
}

#[test]
fn opens_on_its_commands_over_the_session() {
    let mut app = App::new(commands(&["inbox".into(), "actions".into(), "monitor".into()]), Some(claude()), None);
    insta::assert_snapshot!(render(&mut app, 80, 18));
}

#[test]
fn enter_chooses_a_command_and_esc_closes() {
    let mut app = App::new(commands(&["inbox".into()]), None, None);
    assert!(!app.event(press(KeyCode::Down)));
    assert!(app.event(press(KeyCode::Enter)));
    assert_eq!(app.then, Some(Then::RefreshInbox));
    let mut app = App::new(vec![Command::Space("to-do".into())], None, None);
    assert!(app.event(press(KeyCode::Enter)));
    assert_eq!(app.then, Some(Then::Space("to-do".into())));
    let mut app = App::new(commands(&[]), None, None);
    assert!(app.event(press(KeyCode::Esc)));
    assert_eq!(app.then, None);
}

#[test]
fn a_found_item_goes_into_the_agent_session() {
    let pr = Found {
        app: "inbox".into(),
        kind: "pr".into(),
        title: "api#4 Login page".into(),
        help: "me/eng-401-login · CI failure".into(),
        target: json!({"app": "inbox", "key": "https://github.com/acme/api/pull/4"}),
    };
    let none = Value::Null;
    let insert = into_session('r', &claude(), &pr, &none, &none);
    assert_eq!(insert, Some(Then::Insert { pane: "w2:p1".into(), text: "https://github.com/acme/api/pull/4 ".into() }));
    let Some(Then::Ask { pane, text }) = into_session('a', &claude(), &pr, &none, &none) else { panic!("an ask") };
    assert_eq!((pane.as_str(), text.starts_with("Look at https://github.com/acme/api/pull/4: ")), ("w2:p1", true));
    let Some(Then::Fork { agent, session, cwd, title, prompt }) = into_session('f', &claude(), &pr, &none, &none) else { panic!("a fork") };
    assert_eq!((agent.as_str(), session.as_str(), cwd.as_str(), title.as_str()), ("claude", "4f1c-77", "/repos/api", "api#4 Login page"));
    assert!(prompt.contains("look into https://github.com/acme/api/pull/4."));
    let shell = Over { agent: None, ..claude() };
    assert_eq!(into_session('r', &shell, &pr, &none, &none), None);
    let codex = Over { agent: Some("codex".into()), ..claude() };
    assert!(matches!(into_session('f', &codex, &pr, &none, &none), Some(Then::Fork { agent, .. }) if agent == "codex"));
    let unnamed = Over { session: None, ..claude() };
    assert_eq!(into_session('f', &unnamed, &pr, &none, &none), None);
}

#[test]
fn over_an_agent_session_it_forks_it_and_opens_its_folder() {
    let mut app = App::new(commands(&[]), Some(claude()), None);
    for _ in 0..3 {
        app.event(press(KeyCode::Down));
    }
    assert!(app.event(press(KeyCode::Enter)));
    assert_eq!(
        app.then,
        Some(Then::Fork { agent: "claude".into(), session: "4f1c-77".into(), cwd: "/repos/api".into(), title: "Google sign-in".into(), prompt: String::new() })
    );
    let mut app = App::new(commands(&[]), Some(claude()), None);
    for _ in 0..4 {
        app.event(press(KeyCode::Down));
    }
    assert!(app.event(press(KeyCode::Enter)));
    assert_eq!(app.then, Some(Then::Editor("/repos/api".into())));
    let shell = Over { agent: None, session: None, ..claude() };
    assert_eq!(cockpit_palette::session_commands(&shell), [Command::Editor("/repos/api".into())]);
}

#[test]
fn the_agent_below_brings_its_links_then_its_documents() {
    let agents = json!({"agents": [{"terminal": "w2:p1",
        "links": [{"name": "GitHub", "label": "GitHub", "detail": "#12 · Quotas", "url": "https://github.com/acme/api/pull/12"}],
        "documents": [{"name": "plan", "title": "The plan"}]}]});
    let found = agent_commands("w2:p1", &agents);
    assert_eq!(
        found,
        [
            Command::Link { label: "GitHub".into(), detail: "#12 · Quotas".into(), url: "https://github.com/acme/api/pull/12".into() },
            Command::Document { pane: "w2:p1".into(), name: "plan".into(), title: "The plan".into() },
        ]
    );
    assert!(agent_commands("w9:p9", &agents).is_empty());
    let mut app = App::new(found, Some(claude()), None);
    app.event(press(KeyCode::Enter));
    assert_eq!(app.then, Some(Then::Open("https://github.com/acme/api/pull/12".into())));
}
