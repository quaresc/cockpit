//! The popup as drawn, its commands chosen by key, and what a found item becomes in the Claude session below it.

use cockpit_core::hub::Found;
use cockpit_core::testing::render;
use cockpit_palette::{App, Command, Over, Then, commands, into_session};
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
    assert_eq!(claude(), Over { pane: "w2:p1".into(), is_claude: true, session: Some("4f1c-77".into()), cwd: "/repos/api".into(), title: "Google sign-in".into() });
    let shell = Over::of(&json!({"pane_id": "w1:p2", "cwd": "/repos"})).unwrap();
    assert_eq!((shell.is_claude, shell.session, shell.title.as_str()), (false, None, "w1:p2"));
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
fn a_found_item_goes_into_the_claude_session() {
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
    let Some(Then::Fork { session, cwd, title, prompt }) = into_session('f', &claude(), &pr, &none, &none) else { panic!("a fork") };
    assert_eq!((session.as_str(), cwd.as_str(), title.as_str()), ("4f1c-77", "/repos/api", "api#4 Login page"));
    assert!(prompt.contains("look into https://github.com/acme/api/pull/4."));
    let shell = Over { is_claude: false, ..claude() };
    assert_eq!(into_session('r', &shell, &pr, &none, &none), None);
    let unnamed = Over { session: None, ..claude() };
    assert_eq!(into_session('f', &unnamed, &pr, &none, &none), None);
}
