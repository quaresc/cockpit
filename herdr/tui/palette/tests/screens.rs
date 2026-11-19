//! The popup as drawn, its commands chosen by key, and what a found item becomes in the agent session below it.

use cockpit_core::hub::Found;
use cockpit_core::testing::render;
use cockpit_palette::{App, Arrived, Command, Over, Then, agent_commands, commands, into_session, keyed_commands};
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
    // The chords are given, not read from this machine's herdr config: the screen must be the same everywhere.
    let keys = [("inbox", "prefix+i"), ("actions", "prefix+a"), ("monitor", "prefix+o")];
    let keys = keys.iter().map(|(s, k)| (s.to_string(), k.to_string())).collect();
    let mut app = App::new(keyed_commands(&["inbox".into(), "actions".into(), "monitor".into()], &keys), Some(claude()), None);
    insta::assert_snapshot!(render(&mut app, 80, 18));
}

#[test]
fn enter_chooses_a_command_and_esc_closes() {
    let mut app = App::new(commands(&["inbox".into()]), None, None);
    assert!(!app.event(press(KeyCode::Down)));
    assert!(app.event(press(KeyCode::Enter)));
    assert_eq!(app.then, Some(Then::RefreshInbox));
    let mut app = App::new(vec![Command::Space("to-do".into(), None)], None, None);
    assert!(app.event(press(KeyCode::Enter)));
    assert_eq!(app.then, Some(Then::Space("to-do".into())));
    let mut app = App::new(commands(&[]), None, None);
    assert!(app.event(press(KeyCode::Esc)));
    assert_eq!(app.then, None);
}

/// Typed words nothing matches become a start: the Start work form on them. One character, so the hub is not asked.
#[test]
fn what_nothing_matches_is_offered_as_start_work() {
    let mut app = App::new(commands(&["inbox".into()]), None, None);
    assert!(!app.event(press(KeyCode::Char('r'))));
    assert!(!render(&mut app, 80, 18).contains("start work:"), "a command matches: no start offered");
    assert!(!app.event(press(KeyCode::Backspace)));
    assert!(!app.event(press(KeyCode::Char('#'))));
    let screen = render(&mut app, 80, 18);
    assert!(screen.contains("start work: #"), "{screen}");
    assert!(app.event(press(KeyCode::Enter)));
    assert_eq!(app.then, Some(Then::StartWork("#".into())));
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
    for _ in 0..4 {
        app.event(press(KeyCode::Down));
    }
    assert!(app.event(press(KeyCode::Enter)));
    assert_eq!(
        app.then,
        Some(Then::Fork { agent: "claude".into(), session: "4f1c-77".into(), cwd: "/repos/api".into(), title: "Google sign-in".into(), prompt: String::new() })
    );
    let mut app = App::new(commands(&[]), Some(claude()), None);
    for _ in 0..5 {
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

/// The demo's agents as both halves list them: herdr panes, the app's terminals, and a session cockpit drives.
fn demo_agents(frontend: &str) -> Value {
    let row = |t: &str, title: &str, stage: &str, mins: u64, tab: &str| {
        let terminal = if frontend == "tui" { format!("w1:{t}") } else { format!("app:{t}") };
        json!({"terminal": terminal, "address": t, "frontend": frontend, "title": title, "stage": stage, "status": "idle",
               "since": NOW - mins * 60_000, "space": {"name": "api"}, "tab": {"name": tab}})
    };
    json!({"agents": [
        row("t1", "Invoice CSV export", "working", 4, "csv"),
        row("t2", "Enforce daily quotas", "needs_you", 7, "quotas"),
        row("t3", "Settings page copy", "done", 10, "copy"),
        row("t4", "Quota banner and disabled state", "needs_you", 3, "banner"),
        row("t5", "Flaky checkout e2e test", "idle", 15, "e2e"),
        json!({"terminal": "claude:1793611000000", "address": "claude:1793611000000", "frontend": frontend, "driven": true,
               "title": "Make the quota configurable", "stage": "needs_you", "status": "blocked", "since": NOW - 20 * 60_000}),
    ]})
}

/// Both halves' agents in one agents view, as the hub serves it to the herdr TUI.
fn both_halves() -> Value {
    let mut all = demo_agents("tui");
    let app = demo_agents("app");
    all["agents"].as_array_mut().unwrap().push(app["agents"][0].clone());
    all
}

const NOW: u64 = 1_793_611_400_000;

#[test]
fn lists_the_agents_waiting_on_you_first_then_the_latest() {
    let keys = [("inbox", "prefix+i")].iter().map(|(s, k)| (s.to_string(), k.to_string())).collect();
    let mut commands = cockpit_palette::agent_entries(&both_halves(), NOW);
    commands.extend(keyed_commands(&["inbox".into()], &keys));
    let mut app = App::new(commands, None, None);
    insta::assert_snapshot!(render(&mut app, 80, 24));
    assert!(app.event(press(KeyCode::Enter)));
    assert_eq!(app.then, Some(Then::Agent("w1:t2".into())), "the one waiting longest");
}

#[test]
fn the_tui_lists_its_own_panes_and_no_driven_session() {
    let entries = cockpit_palette::agent_entries(&both_halves(), NOW);
    let terminals: Vec<String> = entries.iter().filter_map(|c| if let Command::Agent { jump, .. } = c { Some(jump.terminal.clone()) } else { None }).collect();
    assert_eq!(terminals, ["w1:t2", "w1:t4", "w1:t3", "w1:t1", "w1:t5"], "no app terminal, no driven session");
}

#[test]
fn the_order_is_the_hubs_for_every_palette() {
    let order: Vec<String> = cockpit_palette::jumps(&demo_agents("app"), Some("app"), NOW).into_iter().map(|j| j.title).collect();
    assert_eq!(
        order,
        [
            "Make the quota configurable",
            "Enforce daily quotas",
            "Quota banner and disabled state",
            "Settings page copy",
            "Invoice CSV export",
            "Flaky checkout e2e test"
        ]
    );
    let driven = cockpit_palette::jumps(&demo_agents("app"), Some("app"), NOW).into_iter().find(|j| j.driven).map(|j| j.json());
    assert_eq!(driven.map(|j| j["driven"].clone()), Some(json!(true)), "a desktop palette knows which one has no terminal");
}

#[test]
fn the_selection_stays_on_its_entry_when_the_agents_arrive() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(commands(&["inbox".into()]), None, Some(rx));
    app.event(press(KeyCode::Down));
    tx.send(Arrived { over: None, links: Vec::new(), agents: cockpit_palette::agent_entries(&both_halves(), NOW) }).unwrap();
    assert!(app.tick());
    assert!(app.event(press(KeyCode::Enter)));
    assert_eq!(app.then, Some(Then::RefreshInbox), "the entry picked before the agents arrived");
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(commands(&["inbox".into()]), None, Some(rx));
    tx.send(Arrived { over: None, links: Vec::new(), agents: cockpit_palette::agent_entries(&both_halves(), NOW) }).unwrap();
    app.tick();
    assert!(app.event(press(KeyCode::Enter)));
    assert_eq!(app.then, Some(Then::Agent("w1:t2".into())), "a selection left at the top stays there: the agent waiting longest");
}
