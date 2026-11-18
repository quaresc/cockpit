//! How fresh a piece of fetched data is: when it was last read, when it is read next, and what went wrong if the last
//! read failed. Sources, usage windows and anything else that can go stale describe themselves with it, the same way.

use serde_json::{Value, json};

/// Why a read failed, in the words every surface shows and the action it offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The token or login was refused: signing in again fixes it.
    Auth,
    /// The service asked to wait: it is read again once the limit resets.
    Rate,
    /// The service could not be reached.
    Network,
    /// What was asked for is not there, or not visible to this account.
    NotFound,
    /// The service answered with an error of its own.
    Server,
    Other,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Auth => "auth",
            Kind::Rate => "rate",
            Kind::Network => "network",
            Kind::NotFound => "not_found",
            Kind::Server => "server",
            Kind::Other => "other",
        }
    }

    pub fn parse(word: &str) -> Kind {
        match word {
            "auth" => Kind::Auth,
            "rate" => Kind::Rate,
            "network" => Kind::Network,
            "not_found" => Kind::NotFound,
            "server" => Kind::Server,
            _ => Kind::Other,
        }
    }

    /// The kind an error message most likely means, for sources that only report text.
    pub fn guess(message: &str) -> Kind {
        let m = message.to_ascii_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| m.contains(w));
        // A certificate that expired is the connection's, and GitHub's "could not resolve to a Repository" a missing one.
        if has(&["certificate"]) {
            return Kind::Network;
        }
        if has(&["could not resolve to a"]) {
            return Kind::NotFound;
        }
        if has(&["401", "unauthorized", "token refused", "auth login", "sign in", "expired", "invalid_grant", "forbidden"]) {
            Kind::Auth
        } else if has(&["429", "rate limit", "rate-limit", "too many requests", "quota exhausted"]) {
            Kind::Rate
        } else if has(&["timed out", "timeout", "connection refused", "could not resolve", "network", "offline", "unreachable"]) {
            Kind::Network
        } else if has(&["404", "not found"]) {
            Kind::NotFound
        } else if has(&["500", "502", "503", "504", "server error", "bad gateway"]) {
            Kind::Server
        } else {
            Kind::Other
        }
    }
}

/// What a source knows about its own freshness.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Freshness {
    /// What it is ("github", "tickets", "claude-statusline"…).
    pub source: String,
    /// When the data was last read without error, in ms.
    pub at: Option<u64>,
    /// When it is read next, in ms, if known.
    pub next: Option<u64>,
    /// How often it is read, in ms: well past the next read (`stale_after`) the data is stale.
    pub every: Option<u64>,
    /// The last read's failure, while it lasts: (kind, message, since ms).
    pub error: Option<(Kind, String, u64)>,
}

/// The four states a surface draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Never read yet.
    Never,
    Fresh,
    /// Read, but not again when it was due (`stale_after`).
    Stale,
    /// The last read failed.
    Failing,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Never => "never",
            State::Fresh => "fresh",
            State::Stale => "stale",
            State::Failing => "failing",
        }
    }
}

impl Freshness {
    pub fn new(source: &str) -> Self {
        Freshness { source: source.to_string(), ..Default::default() }
    }

    /// A good read at `now`: the error is over.
    pub fn read(&mut self, now: u64) {
        self.at = Some(now);
        self.error = None;
    }

    /// A failed read at `now`; the first failure of a streak keeps its time.
    pub fn failed(&mut self, now: u64, kind: Kind, message: &str) {
        let since = match &self.error {
            Some((_, _, since)) => *since,
            None => now,
        };
        self.error = Some((kind, message.to_string(), since));
    }

    pub fn state(&self, now: u64) -> State {
        if self.error.is_some() {
            return State::Failing;
        }
        let Some(at) = self.at else { return State::Never };
        match self.every {
            Some(every) if now > stale_after(at, self.next, every) => State::Stale,
            _ => State::Fresh,
        }
    }

    /// The shape every surface reads: `{source, at, next, state, error: {kind, message, since}|null}`.
    pub fn json(&self, now: u64) -> Value {
        json!({
            "source": self.source, "at": self.at, "next": self.next, "every": self.every, "state": self.state(now).word(),
            "error": self.error.as_ref().map(|(kind, message, since)| json!({"kind": kind.word(), "message": message, "since": since})),
        })
    }

    pub fn from_json(value: &Value) -> Freshness {
        let error = value["error"].as_object().map(|e| {
            let kind = Kind::parse(e.get("kind").and_then(Value::as_str).unwrap_or(""));
            let message = e.get("message").and_then(Value::as_str).unwrap_or("").to_string();
            (kind, message, e.get("since").and_then(Value::as_u64).unwrap_or(0))
        });
        Freshness {
            source: value["source"].as_str().unwrap_or("").to_string(),
            at: value["at"].as_u64(),
            next: value["next"].as_u64(),
            every: value["every"].as_u64(),
            error,
        }
    }
}

/// Sources are read one after another, so a read is late only well past when it was due: two intervals, five minutes
/// at least, after the later of its next read and one interval on.
pub const GRACE_MIN: u64 = 5 * 60_000;

/// When data read at `at`, due again at `next`, every `every` ms, turns stale.
pub fn stale_after(at: u64, next: Option<u64>, every: u64) -> u64 {
    let due = next.unwrap_or(0).max(at.saturating_add(every));
    due.saturating_add(every.saturating_mul(2).max(GRACE_MIN))
}

/// "just now", "4 min ago", "2 h ago", "3 d ago": how long ago `at` was, for a chip.
pub fn ago(at: u64, now: u64) -> String {
    let secs = now.saturating_sub(at) / 1000;
    match secs {
        0..=44 => "just now".into(),
        45..=3599 => format!("{} min ago", (secs + 30) / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_is_stale_well_past_its_next_read_and_failing_while_its_error_lasts() {
        let mut f = Freshness { every: Some(60_000), ..Freshness::new("github") };
        assert_eq!(f.state(0), State::Never);
        f.read(1_000);
        assert_eq!(f.state(100_000), State::Fresh);
        // Due at 61 000, then five minutes of grace (two intervals are less).
        assert_eq!(f.state(61_000 + 300_000), State::Fresh);
        assert_eq!(f.state(61_000 + 300_001), State::Stale);
        f.next = Some(500_000);
        assert_eq!(f.state(400_000), State::Fresh, "a later next read moves it on");
        f.failed(200_000, Kind::Auth, "HTTP 401: token refused");
        f.failed(260_000, Kind::Auth, "HTTP 401: token refused");
        assert_eq!(f.state(260_000), State::Failing);
        assert_eq!(f.error.as_ref().map(|e| e.2), Some(200_000), "a streak keeps the time it began");
        f.read(300_000);
        assert_eq!(f.state(300_000), State::Fresh);
        let hourly = Freshness { every: Some(3_600_000), at: Some(0), ..Freshness::new("calendar") };
        assert_eq!((hourly.state(3 * 3_600_000), hourly.state(3 * 3_600_000 + 1)), (State::Fresh, State::Stale), "two intervals past due");
    }

    #[test]
    fn the_json_shape_reads_back() {
        let mut f = Freshness { every: Some(30_000), next: Some(9), ..Freshness::new("ci") };
        f.read(5);
        f.failed(7, Kind::Rate, "HTTP 429");
        let back = Freshness::from_json(&f.json(8));
        assert_eq!(back, f);
        assert_eq!(f.json(8)["state"], "failing");
        assert_eq!(f.json(8)["error"]["kind"], "rate");
    }

    #[test]
    fn messages_are_read_as_their_kind() {
        assert_eq!(Kind::guess("HTTP 401: token refused"), Kind::Auth);
        assert_eq!(Kind::guess("run gh auth login"), Kind::Auth);
        assert_eq!(Kind::guess("HTTP 429 Too Many Requests"), Kind::Rate);
        assert_eq!(Kind::guess("connection refused"), Kind::Network);
        assert_eq!(Kind::guess("HTTP 503"), Kind::Server);
        assert_eq!(Kind::guess("odd"), Kind::Other);
        assert_eq!(Kind::guess("invalid peer certificate: Expired"), Kind::Network);
        assert_eq!(Kind::guess("Could not resolve to a Repository with the name 'acme/api'"), Kind::NotFound);
    }

    #[test]
    fn ago_reads_like_a_person_would_say_it() {
        assert_eq!(ago(0, 10_000), "just now");
        assert_eq!(ago(0, 4 * 60_000), "4 min ago");
        assert_eq!(ago(0, 2 * 3_600_000), "2 h ago");
        assert_eq!(ago(0, 3 * 86_400_000), "3 d ago");
    }
}
