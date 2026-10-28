//! Since you last looked: the time an app's pane last lost the terminal focus, one file per app in
//! ~/.cache/cockpit/seen/, and the hub's events after it, fetched in the background. A row an event touched gets a
//! marker until you select it or leave the app again, and the head says how many changes came and what they were.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::text::{Line, Span};
use serde_json::{Value, json};

use crate::files;
use crate::text::{fit, width};
use crate::theme::{FG, ORANGE, bold, fg};

/// The marker in front of a changed row's text, and the cells it takes.
pub const MARK: &str = "◆ ";
pub const MARK_WIDTH: usize = 2;

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn mark() -> Span<'static> {
    Span::styled(MARK, fg(ORANGE))
}

pub struct Seen {
    app: String,
    /// Where the time is kept; None keeps it in memory only.
    path: Option<PathBuf>,
    /// Whether events are asked of the hub.
    fetching: bool,
    /// When you last looked: events after it are new.
    since: u64,
    /// Rows selected since, with the newest event of theirs seen then.
    selected: HashMap<String, u64>,
    /// The hub's events for this app, newest first.
    events: Vec<Value>,
    busy: bool,
    again: bool,
    tx: Sender<Vec<Value>>,
    rx: Receiver<Vec<Value>>,
}

impl Seen {
    /// The app's time from its file (now when there is none), its events asked of the hub.
    pub fn start(app: &str) -> Self {
        let mut seen = Self::open(app, Some(files::cache().join("seen").join(format!("{app}.json"))), true);
        seen.fetch();
        seen
    }

    /// The time from `path` when given, else now; `fetching` false asks the hub nothing.
    pub fn open(app: &str, path: Option<PathBuf>, fetching: bool) -> Self {
        let stored = path.as_deref().and_then(files::load).and_then(|v| v["at"].as_u64());
        let (tx, rx) = channel();
        Self {
            app: app.into(),
            path,
            fetching,
            since: stored.unwrap_or_else(now_ms),
            selected: HashMap::new(),
            events: Vec::new(),
            busy: false,
            again: false,
            tx,
            rx,
        }
    }

    /// Given events against a given time, for tests: nothing read, written or fetched.
    pub fn fixture(since: u64, events: Vec<Value>) -> Self {
        let mut seen = Self::open("", None, false);
        seen.since = since;
        seen.events = events;
        seen
    }

    pub fn since(&self) -> u64 {
        self.since
    }

    /// The pane got the focus (markers then compare against the time kept when it lost it) or lost it (now is kept,
    /// and every marker clears).
    pub fn focus(&mut self, gained: bool) {
        if gained {
            self.fetch();
            return;
        }
        self.since = now_ms();
        self.selected.clear();
        if let Some(path) = &self.path {
            let _ = files::write(path, &json!({"at": self.since}));
        }
    }

    /// Asks the hub for the events since you last looked, in the background; once more after the answer when asked
    /// while one is on its way.
    pub fn fetch(&mut self) {
        if !self.fetching {
            return;
        }
        if self.busy {
            self.again = true;
            return;
        }
        self.busy = true;
        let (app, since, tx) = (self.app.clone(), self.since, self.tx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(crate::hub::events(&app, since));
            crate::term::wake();
        });
    }

    /// Takes the events that arrived; true when they differ from those shown.
    pub fn pump(&mut self) -> bool {
        let Ok(events) = self.rx.try_recv() else { return false };
        self.busy = false;
        if std::mem::take(&mut self.again) {
            self.fetch();
        }
        let changed = events != self.events;
        self.events = events;
        changed
    }

    fn at(event: &Value) -> u64 {
        event["at"].as_u64().unwrap_or(0)
    }

    fn key(event: &Value) -> &str {
        event["key"].as_str().unwrap_or_default()
    }

    /// The events after you last looked, newest first, less those of rows selected since.
    pub fn fresh(&self) -> impl Iterator<Item = &Value> {
        self.events.iter().filter(|e| Self::at(e) > self.since && self.selected.get(Self::key(e)).is_none_or(|&at| Self::at(e) > at))
    }

    pub fn is_marked(&self, key: &str) -> bool {
        self.fresh().any(|e| Self::key(e) == key)
    }

    /// Clears the markers of these rows: the row the cursor moved onto, and those it hides folded. True when one was.
    pub fn select<'a>(&mut self, keys: impl IntoIterator<Item = &'a str>) -> bool {
        let mut cleared = false;
        for key in keys {
            let Some(newest) = self.fresh().filter(|e| Self::key(e) == key).map(Self::at).max() else { continue };
            self.selected.insert(key.to_string(), newest);
            cleared = true;
        }
        cleared
    }

    /// "N changes since HH:MM: <title> · <title>…" in `cells`, the time as `clock` writes it; None without changes.
    pub fn head(&self, cells: usize, clock: impl Fn(u64) -> String) -> Option<Line<'static>> {
        let titles: Vec<&str> = self.fresh().map(|e| e["title"].as_str().unwrap_or_default()).collect();
        if titles.is_empty() {
            return None;
        }
        let n = titles.len();
        let count = format!("{n} change{} since {}: ", if n == 1 { "" } else { "s" }, clock(self.since));
        let room = cells.saturating_sub(MARK_WIDTH + width(&count));
        let rest = fit(&titles.join(" · "), room).trim_end().to_string();
        Some(Line::from(vec![mark(), Span::styled(count, bold(ORANGE)), Span::styled(rest, fg(FG))]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(at: u64, key: &str, title: &str) -> Value {
        json!({"at": at, "doc": "inbox", "app": "inbox", "key": key, "title": title})
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cockpit-seen-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("inbox.json")
    }

    #[test]
    fn losing_the_focus_keeps_the_time_and_clears_the_markers() {
        let path = temp("lost");
        files::write(&path, &json!({"at": 1_000})).unwrap();
        let mut seen = Seen::open("inbox", Some(path.clone()), false);
        seen.events = vec![event(1_500, "ENG-1", "🎫 ENG-1: Todo → Done")];
        assert!(seen.is_marked("ENG-1"));
        let before = now_ms();
        seen.focus(false);
        let kept = files::load(&path).and_then(|v| v["at"].as_u64()).expect("the time is kept");
        assert!(kept >= before && seen.since() == kept);
        assert!(!seen.is_marked("ENG-1"), "leaving clears every marker");
        assert!(seen.head(90, |_| "10:00".into()).is_none());
    }

    #[test]
    fn coming_back_compares_against_the_time_kept_before() {
        let path = temp("gained");
        files::write(&path, &json!({"at": 1_000})).unwrap();
        let mut seen = Seen::open("inbox", Some(path.clone()), false);
        seen.events = vec![event(1_500, "https://github.com/acme/api/pull/7", "🔴 CI failing: api#7"), event(900, "ENG-1", "🎫 ENG-1: Todo → Done")];
        seen.focus(true);
        assert_eq!(seen.since(), 1_000, "gaining the focus moves nothing");
        assert!(seen.is_marked("https://github.com/acme/api/pull/7"));
        assert!(!seen.is_marked("ENG-1"), "an event before the time kept is old");
        assert_eq!(files::load(&path).unwrap()["at"], 1_000);
    }

    #[test]
    fn selecting_a_row_clears_its_marker_until_a_newer_event() {
        let mut seen = Seen::fixture(1_000, vec![event(1_200, "ENG-1", "a"), event(1_100, "ENG-2", "b")]);
        assert!(seen.select(["ENG-1"]));
        assert!(!seen.is_marked("ENG-1") && seen.is_marked("ENG-2"));
        assert!(!seen.select(["ENG-1"]), "nothing left to clear");
        seen.events.insert(0, event(1_300, "ENG-1", "c"));
        assert!(seen.is_marked("ENG-1"), "a newer event marks it again");
    }

    #[test]
    fn the_head_counts_every_change_and_fits_the_titles() {
        let seen = Seen::fixture(1_000, vec![event(1_300, "nowhere", "🟢 CI passed: api main"), event(1_200, "ENG-1", "🎫 ENG-1: Todo → In progress")]);
        let text = |line: Line| line.spans.iter().map(|s| s.content.to_string()).collect::<String>();
        let head = text(seen.head(200, |_| "09:30".into()).unwrap());
        assert_eq!(head, "◆ 2 changes since 09:30: 🟢 CI passed: api main · 🎫 ENG-1: Todo → In progress");
        let narrow = text(seen.head(40, |_| "09:30".into()).unwrap());
        assert!(width(&narrow) <= 40);
        assert!(narrow.ends_with('…'), "{narrow}");
    }
}
