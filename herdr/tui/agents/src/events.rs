//! herdr-core's fact changes, followed with `hc events`: a terminal made, ended or changing status wakes the agents view
//! before its next look, so it can look less often while nothing moves.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use cockpit_core::frontend;
use serde_json::Value;

/// Whether `hc events` runs and has sent its first snapshot: the agents view then trusts it to say when to look.
static FOLLOWING: AtomicBool = AtomicBool::new(false);

/// The fact changes that can change what the agents view shows; titles, labels, sizes and documents are not among them.
const WAKES: [&str; 7] = ["created", "lost", "restarted", "finished", "removed", "status", "turn"];

/// Whether a line of `hc events --json` calls for a look: one of `WAKES`, or `missed` (the core dropped some).
pub fn wakes(line: &str) -> bool {
    let Ok(event) = serde_json::from_str::<Value>(line) else { return false };
    match event["method"].as_str() {
        Some("event") => event["params"]["type"].as_str().is_some_and(|t| WAKES.contains(&t)),
        Some("missed") => true,
        _ => false,
    }
}

pub fn following() -> bool {
    FOLLOWING.load(Ordering::Relaxed)
}

/// How long the agents view waits before its next look. While `hc events` is followed, an idle look is rare because a
/// change wakes it; an agent that waits on a question is still read often, its menu moving as keys go in, and so is
/// the herdr TUI, whose agents herdr-core does not run.
pub fn wait(app: bool, following: bool, tui: bool, blocked: bool) -> Duration {
    match (app, following && !tui && !blocked) {
        (true, true) => Duration::from_secs(10),
        (true, false) => Duration::from_secs(2),
        (false, _) => Duration::from_secs(4),
    }
}

/// Follows `hc events` on a thread of its own for as long as the hub runs: one message on `tx` per change that calls
/// for a look, and again after a pause when the stream ends or hc cannot start.
pub fn follow(tx: Sender<()>) {
    std::thread::spawn(move || {
        let mut pause = Duration::from_secs(5);
        loop {
            if stream(&tx) {
                pause = Duration::from_secs(5);
            }
            FOLLOWING.store(false, Ordering::Relaxed);
            std::thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_secs(60));
        }
    });
}

/// One run of `hc events`, to its end: true when it sent anything.
fn stream(tx: &Sender<()>) -> bool {
    let Ok(mut child) = Command::new(frontend::program("hc", "HC_BIN"))
        .args(["events", "--json"])
        .env_remove("HC_TERMINAL_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let mut heard = false;
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if !heard {
                heard = true;
                FOLLOWING.store(true, Ordering::Relaxed);
            }
            if wakes(&line) && tx.send(()).is_err() {
                break;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    heard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_of_status_wakes_the_view_and_a_title_does_not() {
        let event = |kind: &str| format!(r#"{{"jsonrpc":"2.0","method":"event","params":{{"type":"{kind}","terminal":"t7","data":{{}}}}}}"#);
        for kind in WAKES {
            assert!(wakes(&event(kind)), "{kind}");
        }
        for kind in ["title", "labels", "size", "doc", "keep"] {
            assert!(!wakes(&event(kind)), "{kind}");
        }
        assert!(wakes(r#"{"jsonrpc":"2.0","method":"missed","params":{}}"#));
        assert!(!wakes(r#"{"jsonrpc":"2.0","method":"heartbeat","params":{"seq":683}}"#));
        assert!(!wakes(r#"{"channel":null,"seq":1,"terminals":[]}"#), "the first snapshot is the look the view already took");
        assert!(!wakes("not json"));
    }

    #[test]
    fn a_followed_core_looks_rarely_unless_an_agent_waits_or_the_tui_runs() {
        let s = Duration::from_secs;
        assert_eq!(wait(true, true, false, false), s(10));
        assert_eq!(wait(true, true, false, true), s(2), "a question's menu is read often");
        assert_eq!(wait(true, true, true, false), s(2), "the TUI's agents send no events");
        assert_eq!(wait(true, false, false, false), s(2), "without events, the old pace");
        assert_eq!(wait(false, true, false, false), s(4));
    }
}
