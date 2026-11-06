//! Whether a source is connected, so a space can tell "nothing is wired" from "nothing has arrived yet". The two
//! look the same on screen and read very differently: one waits for a minute, the other waits for ever.

use std::sync::atomic::{AtomicU8, Ordering};

use serde_json::Value;

use crate::files;

/// 0 reads the config, 1 and 2 answer "connected" and "not connected": a screen must not depend on what the
/// machine drawing it happens to have wired. The snapshot helper sets it.
static ASSUME: AtomicU8 = AtomicU8::new(0);

/// Answers every `connected` with this, whatever the config says; None goes back to reading it.
pub fn assume(connected: Option<bool>) {
    ASSUME.store(connected.map_or(0, |c| 1 + u8::from(!c)), Ordering::Relaxed);
}

fn assumed() -> Option<bool> {
    match ASSUME.load(Ordering::Relaxed) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// What a space is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waiting {
    /// Nothing is wired: `cockpit setup` is the answer, not patience.
    NotConnected,
    /// Wired, and the first fetch has not landed.
    FirstFetch,
}

/// The sources a space draws from, by the config block that connects them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Github,
    Tickets,
    Ci,
    Calendar,
    Mail,
    Platform,
}

impl Source {
    /// The config block's name, and what setup calls the thing.
    fn key(self) -> &'static str {
        match self {
            Source::Github => "github",
            Source::Tickets => "tickets",
            Source::Ci => "ci",
            Source::Calendar => "calendar",
            Source::Mail => "mail",
            Source::Platform => "platform",
        }
    }

    /// Connected when its block is on and it has something to reach: a server, or GitHub's own `gh`, or the
    /// calendar's secret address.
    pub fn connected_in(self, config: &Value) -> bool {
        if let Some(assumed) = assumed() {
            return assumed;
        }
        let block = &config[self.key()];
        if block["enabled"] == Value::Bool(false) {
            return false;
        }
        let has = |field: &str| block[field].as_str().is_some_and(|s| !s.is_empty());
        match self {
            // GitHub comes through gh, so the block being on is the whole of it; CI falls back to GitHub's checks.
            Source::Github | Source::Ci => block["enabled"] == Value::Bool(true),
            Source::Calendar => has("server") || has("ics"),
            Source::Tickets | Source::Mail | Source::Platform => has("server"),
        }
    }

    pub fn connected(self) -> bool {
        self.connected_in(&files::config())
    }
}

/// What the spaces drawing from these sources are waiting for: any one connected means the data is on its way.
pub fn waiting_for(sources: &[Source]) -> Waiting {
    let config = files::config();
    if sources.iter().any(|s| s.connected_in(&config)) { Waiting::FirstFetch } else { Waiting::NotConnected }
}

/// The empty state's line: the first fetch is on its way, or nothing is wired and waiting will not help.
pub fn empty_line(sources: &[Source], nothing: &str, soon: &str, connect: &str) -> String {
    match waiting_for(sources) {
        Waiting::FirstFetch => format!("{nothing}: {soon}"),
        Waiting::NotConnected => format!("{nothing}: {connect}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_block_that_is_off_or_reaches_nothing_is_not_connected() {
        let config = json!({
            "github": {"enabled": true},
            "tickets": {"enabled": true, "server": "claude.ai Linear"},
            "ci": {"enabled": true, "server": ""},
            "calendar": {"enabled": true, "server": "", "ics": "https://calendar.example/secret.ics"},
            "mail": {"enabled": true, "server": ""},
            "platform": {"enabled": false, "server": "backstage"},
        });
        assert!(Source::Github.connected_in(&config), "github needs only gh");
        assert!(Source::Tickets.connected_in(&config));
        assert!(Source::Ci.connected_in(&config), "no server means GitHub's own checks");
        assert!(Source::Calendar.connected_in(&config), "the secret address is enough");
        assert!(!Source::Mail.connected_in(&config), "on, but reaching nothing");
        assert!(!Source::Platform.connected_in(&config), "off, whatever its server");
    }

    #[test]
    fn a_cockpit_with_no_config_is_connected_to_nothing() {
        let empty = json!({});
        for source in [Source::Github, Source::Tickets, Source::Ci, Source::Calendar, Source::Mail, Source::Platform] {
            assert!(!source.connected_in(&empty), "{source:?} without a config");
        }
    }

    #[test]
    fn the_line_says_which_of_the_two_it_is() {
        assert!(Source::Mail.connected_in(&json!({"mail": {"enabled": true, "server": "gmail"}})));
        // The wording is the caller's; what matters is which branch it takes.
        assert_eq!(waiting_for(&[]), Waiting::NotConnected, "no source is never a wait");
    }
}
