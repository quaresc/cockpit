//! Next steps ask the followed agent once per finished turn, kept aside from its history: a fork of its session where
//! its table row can fork, else a fresh run on the conversation; the chat gets what it answered, and nothing runs again.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use cockpit_agents::next;
use serde_json::{Value, json};

static TOUCHED: AtomicUsize = AtomicUsize::new(0);

fn touched() {
    TOUCHED.fetch_add(1, Ordering::SeqCst);
}

fn wait_for_touch(before: usize) {
    let start = Instant::now();
    while TOUCHED.load(Ordering::SeqCst) == before && start.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(TOUCHED.load(Ordering::SeqCst) > before, "the chat was built again when the run ended");
}

const ANSWER: &str = "Done. The settings page says what the daily limit counts, that it resets at midnight UTC, and how much is left.";

fn said(role: &str, text: &str) -> Value {
    json!({"role": role, "at": "", "text": text, "tool": null})
}

/// A stand-in agent that notes its arguments, one per line and `---` after each run, and answers `reply`.
fn stub(dir: &std::path::Path, name: &str, reply: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let (bin, log) = (dir.join(name), dir.join(format!("{name}.log")));
    std::fs::write(&bin, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\necho --- >> '{}'\ncat <<'EOF'\n{reply}\nEOF\n", log.display(), log.display()))
        .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

fn runs(log: &std::path::Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    text.split("---\n").filter(|r| !r.is_empty()).map(|r| r.lines().map(str::to_string).collect()).collect()
}

#[test]
fn a_finished_turn_asks_its_agent_once_and_the_chat_gets_what_it_answered() {
    let home = tempfile::tempdir().unwrap();
    let reply = "Here you go:\n[{\"label\": \"Run the tests\", \"prompt\": \"Run the quota tests\"}, {\"label\": \"Lint\", \"prompt\": \"Fix the lint failure\\u202e now\"}]";
    let (claude, claude_log) = stub(home.path(), "claude", reply);
    let (pi, pi_log) = stub(home.path(), "pi", "[]");
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("COCKPIT_CLAUDE", &claude);
        std::env::set_var("COCKPIT_PI", &pi);
        std::env::remove_var("ANTHROPIC_API_KEY");
    }
    let cwd = home.path().to_string_lossy().into_owned();
    let row = |status: &str| json!({"terminal": "app:t43", "agent": "claude", "program": "claude", "status": status, "question": null, "interrupted": false, "session": "s-copy", "cwd": cwd});
    let messages = vec![said("user", "Write the copy for the quota settings page."), said("assistant", ANSWER)];
    let on = json!({"chat": {"nextSteps": true}});

    assert_eq!(next::of(&row("done"), &messages, &json!({}), None, Some(touched)), Value::Null, "off unless the config says so");
    assert!(runs(&claude_log).is_empty());

    // An agent's tools reading `chat` never start a run billed to the person.
    use cockpit_core::peer::{Sender, Who};
    let read_by_agent = cockpit_core::peer::with(Sender::Placed(Who::Agent("w1:p9".into())), || next::of(&row("done"), &messages, &on, None, Some(touched)));
    assert_eq!(read_by_agent, Value::Null);
    assert!(runs(&claude_log).is_empty());
    // The person's own window looks.
    cockpit_core::peer::trusted();

    let before = TOUCHED.load(Ordering::SeqCst);
    let first = next::of(&row("done"), &messages, &on, Some("claude-haiku-4-5"), Some(touched));
    assert_eq!(first["state"], "loading", "the person's own sign-in serves it: no API key needed");
    assert_eq!(first["by"], "Claude Code", "the strip names who writes them");
    wait_for_touch(before);
    let ready = next::of(&row("done"), &messages, &on, Some("claude-haiku-4-5"), Some(touched));
    assert_eq!(ready["state"], "ready");
    assert_eq!(ready["items"], json!([{"label": "Run the tests", "prompt": "Run the quota tests"}, {"label": "Lint", "prompt": "Fix the lint failure now"}]));
    assert_eq!(next::of(&row("idle"), &messages, &on, None, Some(touched))["state"], "ready", "looked at again: the same, not asked again");
    assert_eq!(next::of(&row("working"), &messages, &on, None, Some(touched)), Value::Null, "hidden while it works");

    let asked = runs(&claude_log);
    assert_eq!(asked.len(), 1, "one run for the turn");
    let args = &asked[0];
    let at = |a: &str| args.iter().position(|x| x == a);
    assert_eq!(args[0], "-p");
    assert_eq!(args[at("--resume").unwrap() + 1], "s-copy", "a fork of its session");
    assert!(at("--fork-session").is_some());
    for aside in ["--no-session-persistence", "--safe-mode", "--strict-mcp-config", "--disable-slash-commands", "--no-chrome", "--max-turns"] {
        assert!(at(aside).is_some(), "kept aside: {aside}");
    }
    assert_eq!(args[at("--tools").unwrap() + 1], "", "no read, write, shell or network tools");
    assert!(at("--bare").is_none(), "bare mode would refuse the person's sign-in");
    assert_eq!(args[at("--setting-sources").unwrap() + 1], "", "no hooks from user or project settings");
    assert_eq!(args[at("--model").unwrap() + 1], "haiku", "the agent's lightest, not the session's own");
    assert_eq!(args[at("--effort").unwrap() + 1], "low", "at its lowest effort");
    assert!(args.iter().any(|a| a.starts_with("Do not carry on with the work.")), "the question");
    assert!(!args.iter().any(|a| a.contains("<conversation>")), "a fork reads its own session");

    // The next turn asks again; what was kept is on disk, so a restart asks no turn twice.
    let saved = cockpit_core::files::load(&home.path().join(".cache/cockpit/agents/next/s-copy.json")).unwrap();
    assert_eq!(saved["items"].as_array().map(Vec::len), Some(2));

    // A supported agent without a session reads the bounded conversation in a fresh run.
    let fresh = json!({"terminal": "app:t44", "agent": "claude", "status": "idle", "question": null, "cwd": cwd});
    let before = TOUCHED.load(Ordering::SeqCst);
    assert_eq!(next::of(&fresh, &messages, &on, None, Some(touched))["state"], "loading");
    wait_for_touch(before);
    let asked = runs(&claude_log);
    assert_eq!(asked.len(), 2);
    assert!(asked[1].join("\n").contains("<conversation>\nPerson: Write the copy for the quota settings page.\n\nAgent: Done."));
    assert!(!asked[1].iter().any(|a| a == "--fork-session"));

    // Pi runs with tools, so it never answers: Claude Code reads Pi's conversation instead, with none of Pi's session.
    let pi_row = json!({"terminal": "w1:p2", "agent": "pi", "program": "pi", "status": "idle", "question": null, "session": "pi-1", "cwd": cwd});
    let before = TOUCHED.load(Ordering::SeqCst);
    let for_pi = next::of(&pi_row, &messages, &on, Some("pi-model"), Some(touched));
    assert_eq!((for_pi["state"].as_str(), for_pi["by"].as_str()), (Some("loading"), Some("Claude Code")), "Claude Code writes them, and says so");
    wait_for_touch(before);
    let asked = runs(&claude_log);
    let last = asked.last().unwrap();
    assert!(last.join("\n").contains("<conversation>\nPerson: Write the copy for the quota settings page."), "Pi's conversation, as cockpit read it");
    assert!(!last.iter().any(|a| a == "--fork-session" || a == "pi-1" || a == "pi-model"), "nothing of Pi's session or model");
    assert!(last.iter().any(|a| a == "--safe-mode") && last[last.iter().position(|a| a == "--tools").unwrap() + 1].is_empty(), "still no tools");
    assert!(runs(&pi_log).is_empty(), "Pi itself never runs");
}
