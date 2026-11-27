//! A driven session end to end against `tests/fixtures/stub-claude`, which speaks Claude Code's stream-json and control
//! protocol as measured: its asks arrive, an answer reaches the one it names, and a stale answer is refused.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use cockpit_agents::session::{self, CHANGED};
use serde_json::Value;

fn wait_for(key: &str, what: &str, done: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let now = session::snapshot(key).expect("the session lives");
        if done(&now) {
            return now;
        }
        assert!(started.elapsed() < Duration::from_secs(15), "waited too long for {what}: {now:#}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn pending(s: &Value) -> Vec<String> {
    s["pending"].as_array().into_iter().flatten().filter_map(|p| p["id"].as_str().map(str::to_string)).collect()
}

#[test]
fn an_answer_reaches_the_ask_it_names_and_a_stale_one_is_refused() {
    if !std::process::Command::new("/usr/bin/python3").arg("--version").output().is_ok_and(|o| o.status.success()) {
        eprintln!("no /usr/bin/python3: the stub agent cannot run here");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let work = home.path().join("api");
    std::fs::create_dir_all(&work).unwrap();
    let stub = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stub-claude");
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    let env = home.path().join("stub-env.json");
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("COCKPIT_CLAUDE", &stub);
        std::env::set_var("STUB_ENV", &env);
        // A hub started from inside a Claude Code session.
        std::env::set_var("CLAUDE_CODE_SESSION_ID", "acme-parent-session");
        std::env::set_var("CLAUDE_CODE_MESSAGING_TOKEN", "acme-token");
    }
    // The test acts for the person until it says otherwise.
    cockpit_core::peer::trusted();
    let key = session::start("claude", &work.to_string_lossy(), "default", &session::Picks::default()).expect("the stub starts");

    // It knows itself by its key, never by the session the hub was started from.
    let start = std::time::Instant::now();
    while cockpit_core::files::load(&env).is_none() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let told = cockpit_core::files::load(&env).expect("the stub noted its environment");
    assert_eq!(
        told,
        serde_json::json!({"COCKPIT_SESSION_KEY": key, "CLAUDE_CODE_SESSION_ID": null, "CLAUDE_CODE_MESSAGING_TOKEN": null, "CLAUDE_CONFIG_DIR": null})
    );
    // The views it shows are its own: its row lists them as a terminal's does.
    let show = home.path().join(".cache/cockpit/show");
    std::fs::create_dir_all(&show).unwrap();
    let now = cockpit_core::seen::now_ms();
    let view = serde_json::json!({"title": "Deploy plan", "terminal": key, "at": now, "placement": "right", "blocks": []});
    cockpit_core::files::write(&show.join("deploy-plan.json"), &view).unwrap();
    let rows = cockpit_agents::gui::view(&serde_json::json!({}), &|_| Value::Null);
    let row = rows["agents"].as_array().unwrap().iter().find(|r| r["terminal"] == key.as_str()).cloned().unwrap();
    assert_eq!(row["documents"], serde_json::json!([{"name": "deploy-plan", "title": "Deploy plan"}]));

    // Two asks at once: each has its own id, and each is answered on its own.
    session::send(&key, "ask both").unwrap();
    let both = pending(&wait_for(&key, "two asks", |s| pending(s).len() == 2));
    assert_ne!(both[0], both[1]);
    // The agent's own process (and anything it runs) cannot answer its permission, whatever it names.
    let stub = session::keyed_pids()[0].1;
    cockpit_core::peer::set(Some(stub));
    let from_it = cockpit_agents::gui::act("answer", &[serde_json::json!({"terminal": key, "choice": 1, "ask": both[0], "source": "test:command"})]).unwrap();
    assert!(from_it.unwrap_err().contains("by a person"));
    // Nor through the session app, which `cockpit session answer` and a raw socket write reach.
    let direct = session::act("answer", &[serde_json::json!({"key": key, "ask": both[0], "allow": true, "source": "test:socket"})]).unwrap();
    assert!(direct.unwrap_err().contains("by a person"));
    // Nor may it turn on a mode that runs tools unasked, confirmed or not.
    let mode = session::act("mode", &[serde_json::json!({"key": key, "mode": "bypassPermissions", "confirm": true})]).unwrap();
    assert!(mode.unwrap_err().contains("by a person"));
    // Its tools may write to their own session, never send to, stop or steer another, nor start one.
    let own = session::act("deliver", &[serde_json::json!({"key": key, "text": "and run credo"})]).unwrap();
    assert!(own.unwrap_err().contains("waiting on a question"), "its own session, refused only for its pending ask");
    let other = session::act("send", &[serde_json::json!({"key": "claude:another", "text": "rm -rf build"})]).unwrap();
    assert!(other.unwrap_err().contains("own agent only"));
    let bypass = serde_json::json!({"cwd": "/tmp", "mode": "bypassPermissions", "confirm": true});
    assert!(session::act("start", std::slice::from_ref(&bypass)).unwrap().unwrap_err().contains("by a person"));
    // Work started on another thread carries where who asked stood; a thread that carries no one starts nothing.
    let sender = cockpit_core::peer::carry();
    let carried = std::thread::spawn({
        let bypass = bypass.clone();
        move || {
            cockpit_core::peer::adopt(sender);
            session::act("start", &[bypass]).unwrap()
        }
    });
    assert!(carried.join().unwrap().unwrap_err().contains("by a person"), "Start work's background start is checked against who asked");
    let nobody = std::thread::spawn(move || session::act("start", &[bypass]).unwrap()).join().unwrap();
    assert!(nobody.unwrap_err().contains("cannot tell"), "a thread with no sender fails closed");
    // A connection that names no process answers nothing.
    cockpit_core::peer::set(None);
    assert!(session::answer(&key, &both[0], true, "test:socket").unwrap_err().contains("cannot tell which process"));
    cockpit_core::peer::trusted();
    assert_eq!(pending(&session::snapshot(&key).unwrap()).len(), 2, "both still wait");
    assert_eq!(session::answer(&key, "toolu_nobody_asked", true, "test:command").unwrap_err(), CHANGED);
    assert_eq!(session::answer(&key, &both[1], false, "test:click").unwrap(), "denied");
    assert_eq!(session::answer(&key, &both[1], true, "test:click").unwrap_err(), CHANGED, "answered once, never twice");
    assert_eq!(pending(&session::snapshot(&key).unwrap()), [both[0].clone()], "the other one still waits");
    assert_eq!(session::answer(&key, &both[0], true, "test:click").unwrap(), "allowed");
    let ended = wait_for(&key, "the turn to end", |s| s["working"] == false && pending(s).is_empty());
    let calls: Vec<Value> = ended["messages"].as_array().unwrap().iter().filter(|m| m["tool"].is_object()).map(|m| m["tool"]["ok"].clone()).collect();
    assert_eq!(calls, [Value::Bool(true), Value::Bool(false)], "the allowed call ran, the denied one did not");

    // A card left open on the first ask cannot answer the one that comes after it.
    session::send(&key, "ask twice").unwrap();
    let first = pending(&wait_for(&key, "the first ask", |s| pending(s).len() == 1))[0].clone();
    // Words for it wait until the question is answered: Enter would land in the permission instead.
    cockpit_core::deliver::on_driven(session::deliver_to);
    let refused = cockpit_core::deliver::deliver(&key, "and run credo too");
    assert_eq!(refused, Err("Claude Code is waiting on a question: answer it first".into()));
    assert_eq!(session::answer(&key, &first, true, "test:click").unwrap(), "allowed");
    let second = pending(&wait_for(&key, "the second ask", |s| pending(s).first().is_some_and(|p| *p != first)))[0].clone();
    assert_eq!(session::answer(&key, &first, true, "test:click").unwrap_err(), CHANGED, "the stale card is refused");
    assert_eq!(pending(&session::snapshot(&key).unwrap()), std::slice::from_ref(&second), "and the newer ask is still pending");

    assert!(!session::snapshot(&key).unwrap()["messages"].to_string().contains("and run credo too"), "nothing was sent");

    // Stopping the turn takes the ask back: nothing is left to answer.
    session::act("interrupt", &[serde_json::json!({"key": key})]).unwrap().unwrap();
    wait_for(&key, "the ask to be taken back", |s| pending(s).is_empty());
    assert_eq!(session::answer(&key, &second, true, "test:click").unwrap_err(), CHANGED);

    wait_for(&key, "the turn to end", |s| s["working"] == false);
    assert_eq!(cockpit_core::deliver::deliver(&key, "now the credo run"), Ok(()), "with nothing asked, the words go");
    let log = std::fs::read_to_string(session::dir().join(format!("{key}.log"))).unwrap();
    assert!(log.contains(&format!("{} denied (Bash) from test:click", both[1])), "{log}");
    assert!(log.contains(&format!("refused an answer to {first} from test:click")), "{log}");
    assert!(log.contains(&format!("the agent took back {second}")), "{log}");
    assert!(session::dir().starts_with(home.path().join(".cache/cockpit/driven")));
    session::stop(&key);
    assert!(session::snapshot(&key).is_none());
}
