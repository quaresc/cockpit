//! The app's state and what changes it: the runbook files read again, the steps that turned done played, background
//! refreshes followed, and the pinned space's duties.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use cockpit_core::bridge::Bridge;
use cockpit_core::dialog::Dialog;
use cockpit_core::palette::Palette;
use cockpit_core::toast::{Kind, Toasts};
use ratatui::layout::Rect;
use serde_json::Value;

use crate::anim::{CASCADE, Glide, RowFx};
use crate::book::{self, Book};
use crate::{handoff, pinned};

/// What a palette entry or a confirmed dialog runs: an action on a runbook or one of its steps, or a jump to a step.
#[derive(Clone)]
pub enum Cmd {
    Act { slug: String, n: Option<String>, act: cockpit_core::acts::Act, question: String },
    GoTo { slug: String, n: String },
}

/// Where the last frame drew what a click can land on.
#[derive(Default)]
pub struct Hits {
    pub tabs: Vec<(Rect, usize)>,
    pub rows: Vec<(Rect, usize)>,
    pub buttons: Vec<(Rect, usize)>,
    pub list: Rect,
}

pub struct App {
    pub dir: PathBuf,
    pub books: Vec<Book>,
    pub handoffs: HashMap<String, Value>,
    pub inbox: Value,
    done: Option<HashMap<String, Vec<String>>>,
    seen: Vec<(String, Option<SystemTime>)>,
    pub active: usize,
    pub selected: usize,
    /// Rows to bring into view on the next frame, the first and the last, once their place is known.
    pub reveal: Option<(usize, usize)>,
    pub scroll: HashMap<String, Glide>,
    pub gauges: HashMap<String, Glide>,
    pub underline: (Glide, Glide),
    pub fx: HashMap<(String, String), RowFx>,
    /// Background refreshes: the pid of their Claude and when they started.
    pub refreshing: HashMap<String, (u32, Instant)>,
    /// Refreshes asked of the bridge whose pid has not come back yet, so a second R does not start another.
    asking: HashSet<String>,
    newly: HashMap<String, Vec<String>>,
    reasons: HashMap<String, String>,
    pub bridge: Bridge,
    calls: usize,
    pub toasts: Toasts,
    pub palette: Palette,
    pub cmds: Vec<Cmd>,
    pub dialog: Option<(Dialog, Cmd)>,
    _watcher: Option<notify::RecommendedWatcher>,
    changes: Receiver<()>,
    polled: Instant,
    focus_at: SystemTime,
    pinned: bool,
    pub hits: Hits,
    pub started: Instant,
}

impl App {
    pub fn new() -> Self {
        let dir = book::dir();
        let _ = std::fs::create_dir_all(&dir);
        let (watcher, changes) = cockpit_core::files::watch(&[dir.clone(), handoff::dir()]);
        let mut app = Self {
            dir,
            books: Vec::new(),
            handoffs: HashMap::new(),
            inbox: Value::Null,
            done: None,
            seen: Vec::new(),
            active: 0,
            selected: 0,
            reveal: None,
            scroll: HashMap::new(),
            gauges: HashMap::new(),
            underline: (Glide::new(0.0, 0.3), Glide::new(0.0, 0.3)),
            fx: HashMap::new(),
            refreshing: HashMap::new(),
            asking: HashSet::new(),
            newly: HashMap::new(),
            reasons: HashMap::new(),
            bridge: Bridge::default(),
            calls: 0,
            toasts: Toasts::default(),
            palette: Palette::default(),
            cmds: Vec::new(),
            dialog: None,
            _watcher: watcher,
            changes,
            polled: Instant::now(),
            focus_at: SystemTime::now(),
            pinned: pinned::is_pinned(),
            hits: Hits::default(),
            started: Instant::now(),
        };
        app.poll();
        app
    }

    pub fn book(&self) -> Option<&Book> {
        self.books.get(self.active)
    }

    pub fn is_animating(&self) -> bool {
        !self.fx.is_empty()
            || self.gauges.values().any(Glide::is_moving)
            || self.scroll.values().any(Glide::is_moving)
            || self.underline.0.is_moving()
            || self.underline.1.is_moving()
    }

    pub fn is_busy(&self) -> bool {
        self.calls > 0 || !self.refreshing.is_empty()
    }

    /// Reads the runbooks again when the watcher or the fallback second says something changed.
    pub fn tick(&mut self) {
        self.take_replies();
        self.fx.retain(|_, fx| !fx.is_over());
        let changed = self.changes.try_iter().count() > 0;
        if changed || self.polled.elapsed() >= Duration::from_secs(1) {
            self.poll();
        }
    }

    fn poll(&mut self) {
        self.polled = Instant::now();
        let (inbox, handoffs) = (handoff::inbox_path(), handoff::dir());
        let now = book::stamp(&[&self.dir, &handoffs], &[&inbox]);
        if now != self.seen {
            self.seen = now;
            self.reload();
            if self.pinned {
                pinned::report(&self.books);
            }
        }
        if self.pinned {
            if let Some((slug, steps)) = pinned::take_focus(&mut self.focus_at)
                && let Some(book) = self.books.iter().find(|b| b.slug == slug)
            {
                let steps = if steps.is_empty() { book.done_steps() } else { steps };
                self.play(&slug, steps);
            }
            for (slug, reason) in pinned::take_requests() {
                if self.books.iter().any(|b| b.slug == slug) {
                    self.refresh(&slug);
                    self.reasons.insert(slug, reason);
                }
            }
        }
        self.follow_refreshes();
    }

    /// The runbooks read again; a step that was open and is now done plays, the selected row staying put.
    fn reload(&mut self) {
        let keep = self.book().map(|b| b.slug.clone());
        let keep_n = self.book().and_then(|b| b.steps.get(self.selected)).map(|s| s.n.clone());
        self.books = book::read_books(&self.dir);
        self.handoffs = handoff::read();
        self.inbox = cockpit_core::files::load(&handoff::inbox_path()).unwrap_or(Value::Null);
        let done: HashMap<String, Vec<String>> = self.books.iter().map(|b| (b.slug.clone(), b.done_steps())).collect();
        let before = self.done.replace(done.clone());
        if let Some(i) = keep.and_then(|slug| self.books.iter().position(|b| b.slug == slug)) {
            if i != self.active {
                self.active = i;
            }
            let steps = &self.books[i].steps;
            self.selected = keep_n.and_then(|n| steps.iter().position(|s| s.n == n)).unwrap_or(self.selected.min(steps.len().saturating_sub(1)));
        } else {
            self.active = 0;
            self.selected = 0;
        }
        for book in &self.books {
            let gauge = self.gauges.entry(book.slug.clone()).or_insert_with(|| Glide::new(0.0, 0.9));
            gauge.go((book.steps.len() - book.todo()) as f32);
        }
        let Some(before) = before else { return };
        // In tab order, so the runbook brought into view when several changed at once is always the same one.
        let mut played = Vec::new();
        for book in &self.books {
            if let (Some(was), Some(steps)) = (before.get(&book.slug), done.get(&book.slug)) {
                let newly: Vec<String> = steps.iter().filter(|n| !was.contains(n)).cloned().collect();
                if !newly.is_empty() {
                    played.push((book.slug.clone(), newly));
                }
            }
        }
        for (slug, newly) in played {
            // A refresh can write the file several times: its notification counts every step it finished.
            let all = self.newly.entry(slug.clone()).or_default();
            for n in &newly {
                if !all.contains(n) {
                    all.push(n.clone());
                }
            }
            self.play(&slug, newly);
        }
    }

    /// The steps' done animation, one after the other, their runbook and as many of them as fit brought into view.
    fn play(&mut self, slug: &str, steps: Vec<String>) {
        let Some(i) = self.books.iter().position(|b| b.slug == slug) else { return };
        self.go_tab(i);
        for (k, n) in steps.iter().enumerate() {
            self.fx.insert((slug.to_string(), n.clone()), RowFx::new(k as f32 * CASCADE));
        }
        let rows: Vec<usize> = steps.iter().filter_map(|n| self.books[i].steps.iter().position(|s| &s.n == n)).collect();
        if let (Some(&first), Some(&last)) = (rows.iter().min(), rows.iter().max()) {
            self.reveal = Some((first, last));
        }
        let s = if steps.len() > 1 { "s" } else { "" };
        self.toasts.push(format!("{}: step{s} {} done", slug.to_uppercase(), steps.join(", ")), Kind::Good);
    }

    /// A background refresh ending says so, and in the pinned space sends its notification.
    fn follow_refreshes(&mut self) {
        let ended: Vec<String> = self.refreshing.iter().filter(|(_, (pid, _))| !is_alive(*pid)).map(|(slug, _)| slug.clone()).collect();
        for slug in ended {
            self.refreshing.remove(&slug);
            let newly = self.newly.remove(&slug).unwrap_or_default();
            let reason = self.reasons.remove(&slug).unwrap_or_default();
            // runbook.update writes the exit status beside the log, since a pid alone cannot tell a failure.
            let status = std::fs::read_to_string(cockpit_core::files::cache().join(format!("runbook-refresh-{slug}.exit")));
            if status.ok().and_then(|s| s.trim().parse::<i32>().ok()).is_some_and(|code| code != 0) {
                self.toasts.push(format!("{}: refresh failed, see ~/.cache/cockpit/runbook-refresh-{slug}.log", slug.to_uppercase()), Kind::Bad);
                continue;
            }
            self.toasts.push(format!("{} refreshed", slug.to_uppercase()), Kind::Good);
            if let (true, Some(book)) = (self.pinned, self.books.iter().find(|b| b.slug == slug)) {
                pinned::announce(book, newly, reason);
            }
        }
        // A refresh's newly done steps stay until it ends, for its notification to count them.
        let refreshing = &self.refreshing;
        self.newly.retain(|slug, _| refreshing.contains_key(slug));
    }

    pub fn take_replies(&mut self) {
        while let Ok(reply) = self.bridge.replies.try_recv() {
            self.calls = self.calls.saturating_sub(1);
            if let Some(slug) = reply.tag.strip_prefix("refresh:") {
                self.asking.remove(slug);
            }
            if let Some(error) = reply.error {
                self.toasts.push(error, Kind::Bad);
            }
            self.toasts.push(reply.note, Kind::Good);
            if let Some(started) = reply.refreshing.as_object() {
                for (slug, pid) in started {
                    if let Some(pid) = pid.as_u64() {
                        self.refreshing.insert(slug.clone(), (pid as u32, Instant::now()));
                    }
                }
            }
        }
    }

    /// The runbook as the bridge resolves it: by its slug, or whole when the app reads another folder than runbook.py's.
    fn book_arg(&self, book: &Book) -> Value {
        if std::env::var("COCKPIT_RUNBOOKS_DIR").is_ok() { book.json() } else { serde_json::json!({"$book": book.slug}) }
    }

    pub fn refresh(&mut self, slug: &str) {
        if self.asking.contains(slug) || self.refreshing.get(slug).is_some_and(|(pid, _)| is_alive(*pid)) {
            self.toasts.push(format!("{slug} is already refreshing"), Kind::Good);
            return;
        }
        self.newly.remove(slug);
        if let Some(book) = self.books.iter().find(|b| b.slug == slug) {
            let arg = self.book_arg(book);
            self.asking.insert(slug.to_string());
            self.call(&format!("refresh:{slug}"), "update", vec![arg], None);
        }
    }

    fn call(&mut self, tag: &str, function: &str, args: Vec<Value>, armed: Option<String>) {
        self.calls += 1;
        self.bridge.call(tag, "runbook", function, args, armed);
    }

    /// Runs an action; `armed` is the confirm key, given once its dialog said yes.
    pub fn perform(&mut self, slug: &str, n: Option<&str>, act: &cockpit_core::acts::Act, armed: Option<String>) {
        let Some(book) = self.books.iter().find(|b| b.slug == slug).cloned() else { return };
        let step = n.and_then(|n| book.steps.iter().find(|s| s.n == n)).cloned();
        let arg = self.book_arg(&book);
        match (act.id.as_str(), step) {
            ("refresh", _) => self.refresh(slug),
            ("talk", _) => self.call("talk", "talk", vec![arg], None),
            ("delete", _) => self.call("delete", "delete", vec![arg], armed),
            ("run", Some(step)) => self.call("run", "run_step", vec![arg, step.json()], armed),
            ("done", Some(step)) => self.call("done", "mark_done", vec![arg, step.json()], None),
            ("session", Some(step)) => {
                let id = crate::acts::handoff_id(&book, &step);
                self.call("session", "focus_session", vec![serde_json::json!({"$handoff": id})], None);
            }
            ("open", Some(step)) => {
                if let Some((repo, number)) = crate::acts::pr_ref(&step) {
                    self.call("open", "open_pr", vec![repo.into(), number.into()], None);
                }
            }
            ("copy", Some(step)) => {
                let note = if cockpit_core::files::copy(&step.command) {
                    format!("copied: {}", step.command)
                } else {
                    "no clipboard tool: install wl-copy or xclip".into()
                };
                self.toasts.push(note, Kind::Good);
            }
            _ => {}
        }
    }

    /// An action, asked about first when it changes something hard to undo.
    pub fn act(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Act { slug, n, act, question } if act.confirm.is_some() => {
                let ask = Dialog::Confirm { question: format!("{question}?") };
                self.dialog = Some((ask, Cmd::Act { slug, n, act, question }));
            }
            Cmd::Act { slug, n, act, .. } => self.perform(&slug, n.as_deref(), &act, None),
            Cmd::GoTo { slug, n } => {
                if let Some(i) = self.books.iter().position(|b| b.slug == slug) {
                    self.go_tab(i);
                    self.selected = self.books[i].steps.iter().position(|s| s.n == n).unwrap_or(0);
                    self.reveal = Some((self.selected, self.selected));
                }
            }
        }
    }

    /// Another tab, its first step selected and its scroll where it was left.
    pub fn go_tab(&mut self, i: usize) {
        if i != self.active && i < self.books.len() {
            self.active = i;
            self.selected = 0;
        }
    }
}

fn is_alive(pid: u32) -> bool {
    std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}
