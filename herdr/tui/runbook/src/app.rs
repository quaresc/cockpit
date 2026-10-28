//! The app's state and what changes it on its own: the runbook files and handoffs read again, the steps that turned done
//! played, focus requests followed, sessions' steps pulsing, and the pinned space's duties.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use cockpit_core::anim::{Effects, Glide, Kind};
use cockpit_core::bridge::Bridge;
use cockpit_core::dialog::Dialog;
use cockpit_core::files::Density;
use cockpit_core::palette::Palette;
use cockpit_core::toast::{self, Toasts};
use ratatui::layout::Rect;
use ratatui::style::Color;
use serde_json::Value;

use crate::anim::{CASCADE, Clock, RowFx};
use crate::book::{self, Book};
use crate::edit::Editor;
use crate::filter::{self, Filter, Hit};
use crate::{handoff, pinned};

/// A session's step pulses every PULSE_EVERY while it runs.
const PULSE_EVERY: Duration = Duration::from_secs(4);
/// The short flash a step brought into view by a focus request gets.
pub const FOCUSED: Kind = Kind::Flash(Color::Rgb(90, 70, 50), 1.0);

/// What a palette entry or a confirmed dialog runs: an action on a runbook or one of its steps, a jump to a step, or
/// steps to mark done together.
#[derive(Clone)]
pub enum Cmd {
    Act { slug: String, n: Option<String>, act: cockpit_core::acts::Act, question: String },
    GoTo { slug: String, n: String },
    Done(Vec<(String, String)>),
}

/// Where the last frame drew what a click can land on; rows by their step's index.
#[derive(Default)]
pub struct Hits {
    pub tabs: Vec<(Rect, usize)>,
    pub rows: Vec<(Rect, usize)>,
    pub buttons: Vec<(Rect, usize)>,
    pub list: Rect,
}

/// The folders and files the app reads, the user's own unless a test points elsewhere.
#[derive(Clone, Debug)]
pub struct Paths {
    pub runbooks: PathBuf,
    pub handoffs: PathBuf,
    pub inbox: PathBuf,
    pub focus: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        Self { runbooks: book::dir(), handoffs: handoff::dir(), inbox: handoff::inbox_path(), focus: cockpit_core::files::cache().join("focus") }
    }
}

/// A step's key in the animations, the selection and focus requests: `slug#n`.
pub fn key(slug: &str, n: &str) -> String {
    format!("{slug}#{n}")
}

pub struct App {
    pub paths: Paths,
    pub density: Density,
    pub books: Vec<Book>,
    pub handoffs: HashMap<String, Value>,
    pub inbox: Value,
    done: Option<HashMap<String, Vec<String>>>,
    seen: Vec<(String, Option<SystemTime>)>,
    pub active: usize,
    /// The step under the cursor, by its index in the runbook.
    pub selected: usize,
    /// Rows to bring into view on the next frame, the first and the last, once their place is known.
    pub reveal: Option<(usize, usize)>,
    pub scroll: HashMap<String, Glide>,
    pub gauges: HashMap<String, Glide>,
    pub underline: (Glide, Glide),
    pub fx: HashMap<String, RowFx>,
    pub effects: Effects,
    effects_on: bool,
    pub clock: Clock,
    pub filter: Filter,
    /// The steps the filter matched, by runbook then number; None while it is off.
    pub found: Option<HashMap<String, HashMap<String, Hit>>>,
    /// Steps picked for a batch action, by key.
    pub picked: BTreeSet<String>,
    pub editor: Option<Editor>,
    /// Background refreshes: the pid of their Claude and when they started.
    pub refreshing: HashMap<String, (u32, Instant)>,
    /// Refreshes asked of the bridge whose pid has not come back yet, so a second R does not start another.
    pub asking: HashSet<String>,
    pub newly: HashMap<String, Vec<String>>,
    pub reasons: HashMap<String, String>,
    pub bridge: Bridge,
    pub calls: usize,
    pub toasts: Toasts,
    toasting: bool,
    pub palette: Palette,
    pub cmds: Vec<Cmd>,
    pub dialog: Option<(Dialog, Cmd)>,
    _watcher: Option<notify::RecommendedWatcher>,
    changes: Receiver<()>,
    polled: Instant,
    pulsed: Instant,
    /// What the handoff lines said at the last poll: their texts change with time alone.
    said: Vec<String>,
    focus_at: SystemTime,
    focus_seen: SystemTime,
    pinned: bool,
    pub hits: Hits,
    pub started: Instant,
}

impl App {
    pub fn new() -> Self {
        Self::open(Paths::from_env(), cockpit_core::files::density(), pinned::is_pinned())
    }

    pub fn open(paths: Paths, density: Density, pinned: bool) -> Self {
        let _ = std::fs::create_dir_all(&paths.runbooks);
        let (watcher, changes) = cockpit_core::files::watch(&[paths.runbooks.clone(), paths.handoffs.clone(), paths.focus.clone()]);
        let mut palette = Palette::default();
        palette.app = "runbook".into();
        let mut app = Self {
            paths,
            density,
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
            effects: Effects::default(),
            effects_on: false,
            clock: Clock::default(),
            filter: Filter::default(),
            found: None,
            picked: BTreeSet::new(),
            editor: None,
            refreshing: HashMap::new(),
            asking: HashSet::new(),
            newly: HashMap::new(),
            reasons: HashMap::new(),
            bridge: Bridge::default(),
            calls: 0,
            toasts: Toasts::default(),
            toasting: false,
            palette,
            cmds: Vec::new(),
            dialog: None,
            _watcher: watcher,
            changes,
            polled: Instant::now(),
            pulsed: Instant::now(),
            said: Vec::new(),
            focus_at: SystemTime::now(),
            focus_seen: SystemTime::now(),
            pinned,
            hits: Hits::default(),
            started: Instant::now(),
        };
        app.poll();
        app
    }

    pub fn book(&self) -> Option<&Book> {
        self.books.get(self.active)
    }

    /// The indexes of the shown runbook's steps the filter lets through, all of them while it is off.
    pub fn shown(&self) -> Vec<usize> {
        let Some(book) = self.book() else { return Vec::new() };
        match &self.found {
            Some(found) => (0..book.steps.len()).filter(|&i| found.get(&book.slug).is_some_and(|hits| hits.contains_key(&book.steps[i].n))).collect(),
            None => (0..book.steps.len()).collect(),
        }
    }

    /// Where the filter matched a step, for its row to highlight.
    pub fn hit(&self, slug: &str, n: &str) -> Option<&Hit> {
        self.found.as_ref()?.get(slug)?.get(n)
    }

    /// The cursor on a step the filter shows, the first one when it hid the one it was on.
    pub fn keep_cursor(&mut self) {
        let shown = self.shown();
        if !shown.contains(&self.selected)
            && let Some(&first) = shown.first()
        {
            self.selected = first;
            self.reveal = Some((first, first));
        }
    }

    /// The filter matched again, after the query or the runbooks changed; across all runbooks, a tab with no match
    /// gives way to the first that has one.
    pub fn refilter(&mut self) {
        self.found = filter::find(&self.books, self.filter.query());
        if self.filter.global
            && let Some(found) = &self.found
            && self.book().is_some_and(|b| found.get(&b.slug).is_none_or(HashMap::is_empty))
            && let Some(i) = self.books.iter().position(|b| found.get(&b.slug).is_some_and(|h| !h.is_empty()))
        {
            self.go_tab(i);
        }
        self.keep_cursor();
    }

    pub fn is_animating(&self) -> bool {
        !self.fx.is_empty()
            || self.effects_on
            || self.gauges.values().any(Glide::is_moving)
            || self.scroll.values().any(Glide::is_moving)
            || self.underline.0.is_moving()
            || self.underline.1.is_moving()
    }

    pub fn is_busy(&self) -> bool {
        self.calls > 0 || !self.refreshing.is_empty()
    }

    /// Starts a status animation on a step, the loop ticking fast until it ends.
    pub fn flash(&mut self, key: &str, kind: Kind) {
        self.effects.start(key, kind);
        self.effects_on = true;
    }

    /// Everything that changes without a key: true only when the screen has to be drawn again, so an idle app costs nothing.
    pub fn tick(&mut self) -> bool {
        let mut changed = self.is_animating();
        changed |= self.take_replies();
        let now = self.clock.now();
        self.fx.retain(|_, fx| !fx.is_over(now));
        self.effects_on = self.effects.active();
        if self.changes.try_iter().count() > 0 || self.polled.elapsed() >= Duration::from_secs(1) {
            changed |= self.poll();
        }
        changed |= self.pulse();
        // A toast going away, and the timer of a running refresh.
        let toasting = self.toasts.any();
        changed |= toasting != self.toasting || !self.refreshing.is_empty();
        self.toasting = toasting;
        changed
    }

    /// Reads what changed on disk; true when there is something new to draw.
    pub fn poll(&mut self) -> bool {
        self.polled = Instant::now();
        let mut changed = false;
        let now = book::stamp(&[&self.paths.runbooks, &self.paths.handoffs], &[&self.paths.inbox]);
        if now != self.seen {
            self.seen = now;
            self.reload();
            if self.pinned {
                pinned::report(&self.books);
            }
            changed = true;
        }
        if self.pinned {
            if let Some((slug, steps)) = pinned::take_focus(&mut self.focus_at)
                && let Some(book) = self.books.iter().find(|b| b.slug == slug)
            {
                let steps = if steps.is_empty() { book.done_steps() } else { steps };
                self.play(&slug, steps);
                changed = true;
            }
            for (slug, reason) in pinned::take_requests() {
                if self.books.iter().any(|b| b.slug == slug) {
                    self.refresh(&slug);
                    self.reasons.insert(slug, reason);
                }
            }
        }
        if let Some(target) = cockpit_core::focus::take("runbook", &mut self.focus_seen)
            && let Some(key) = target["key"].as_str()
        {
            self.focus(key);
            changed = true;
        }
        let said: Vec<String> = self.handoffs.values().filter_map(handoff::line).map(|(text, _)| text).collect();
        changed |= said != self.said;
        self.said = said;
        self.follow_refreshes();
        changed
    }

    /// Reads the runbooks now, after the app wrote one, rather than when the watcher says so.
    pub fn reread(&mut self) {
        self.seen = book::stamp(&[&self.paths.runbooks, &self.paths.handoffs], &[&self.paths.inbox]);
        self.reload();
    }

    /// The runbooks read again; a step that was open and is now done plays, a handoff that failed flashes, the selected
    /// row staying put.
    fn reload(&mut self) {
        let keep = self.book().map(|b| b.slug.clone());
        let keep_n = self.book().and_then(|b| b.steps.get(self.selected)).map(|s| s.n.clone());
        self.books = book::read_books(&self.paths.runbooks);
        let before_handoffs = std::mem::replace(&mut self.handoffs, handoff::read(&self.paths.handoffs));
        self.inbox = cockpit_core::files::load(&self.paths.inbox).unwrap_or(Value::Null);
        let done: HashMap<String, Vec<String>> = self.books.iter().map(|b| (b.slug.clone(), b.done_steps())).collect();
        let before = self.done.replace(done.clone());
        if let Some(i) = keep.and_then(|slug| self.books.iter().position(|b| b.slug == slug)) {
            self.active = i;
            let steps = &self.books[i].steps;
            self.selected = keep_n.and_then(|n| steps.iter().position(|s| s.n == n)).unwrap_or(self.selected.min(steps.len().saturating_sub(1)));
        } else {
            self.active = 0;
            self.selected = 0;
        }
        for book in &self.books {
            let done = (book.steps.len() - book.todo()) as f32;
            match self.gauges.get_mut(&book.slug) {
                Some(gauge) => gauge.go(done),
                None => {
                    self.gauges.insert(book.slug.clone(), Glide::new(done, 0.9));
                }
            }
        }
        let books = &self.books;
        self.picked.retain(|k| books.iter().any(|b| b.steps.iter().any(|s| key(&b.slug, &s.n) == *k && s.is_open())));
        self.refilter();
        let Some(before) = before else { return };
        self.flash_failures(&before_handoffs);
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

    /// A handoff that turned failed or blocked since the last read: its step flashes red and says why.
    fn flash_failures(&mut self, before: &HashMap<String, Value>) {
        let failed: Vec<Value> = self
            .handoffs
            .iter()
            .filter(|(id, h)| handoff::has_failed(h) && before.get(*id).is_none_or(|was| was["state"] != h["state"]))
            .map(|(_, h)| h.clone())
            .collect();
        for h in failed {
            let (slug, n) = (h["slug"].as_str().unwrap_or_default(), h["step"].as_str().unwrap_or_default());
            let state = h["state"].as_str().unwrap_or_default();
            self.flash(&key(slug, n), Kind::BROKEN);
            self.toasts.push(format!("{} step {n} {state}: {}", slug.to_uppercase(), h["message"].as_str().unwrap_or_default()), toast::Kind::Bad);
        }
    }

    /// The steps' done animation, one after the other, their runbook and as many of them as fit brought into view.
    pub fn play(&mut self, slug: &str, steps: Vec<String>) {
        let Some(i) = self.books.iter().position(|b| b.slug == slug) else { return };
        self.go_tab(i);
        let now = self.clock.now();
        for (k, n) in steps.iter().enumerate() {
            self.fx.insert(key(slug, n), RowFx::new(now, k as f32 * CASCADE));
        }
        let rows: Vec<usize> = steps.iter().filter_map(|n| self.books[i].steps.iter().position(|s| &s.n == n)).collect();
        if let (Some(&first), Some(&last)) = (rows.iter().min(), rows.iter().max()) {
            self.reveal = Some((first, last));
        }
        let s = if steps.len() > 1 { "s" } else { "" };
        self.toasts.push(format!("{}: step{s} {} done", slug.to_uppercase(), steps.join(", ")), toast::Kind::Good);
    }

    /// Every PULSE_EVERY, the open steps a session is running beat once; true when one did.
    fn pulse(&mut self) -> bool {
        if self.pulsed.elapsed() < PULSE_EVERY {
            return false;
        }
        self.pulsed = Instant::now();
        let running: Vec<String> = self
            .books
            .iter()
            .flat_map(|b| b.steps.iter().filter(|s| s.is_open()).map(move |s| (b, s)))
            .filter(|(b, s)| self.handoffs.get(&crate::acts::handoff_id(b, s)).is_some_and(handoff::is_running))
            .map(|(b, s)| key(&b.slug, &s.n))
            .collect();
        for k in &running {
            self.flash(k, Kind::DONE);
        }
        !running.is_empty()
    }

    /// Brings `slug` or `slug#n` into view: its tab, the step selected and scrolled to, a short flash on it.
    pub fn focus(&mut self, target: &str) {
        let (slug, n) = target.split_once('#').map_or((target, None), |(slug, n)| (slug, Some(n)));
        let Some(i) = self.books.iter().position(|b| b.slug == slug) else { return };
        self.go_tab(i);
        let steps = &self.books[i].steps;
        let at = match n {
            Some(n) => steps.iter().position(|s| s.n == n),
            None => steps.iter().position(|s| s.is_open()).or((!steps.is_empty()).then_some(0)),
        };
        let Some(at) = at else { return };
        if !self.shown().contains(&at) {
            self.filter = Filter::default();
            self.found = None;
        }
        self.selected = at;
        self.reveal = Some((at, at));
        let k = key(slug, &self.books[i].steps[at].n);
        self.flash(&k, FOCUSED);
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
                self.toasts.push(format!("{}: refresh failed, see ~/.cache/cockpit/runbook-refresh-{slug}.log", slug.to_uppercase()), toast::Kind::Bad);
                continue;
            }
            self.toasts.push(format!("{} refreshed", slug.to_uppercase()), toast::Kind::Good);
            if let (true, Some(book)) = (self.pinned, self.books.iter().find(|b| b.slug == slug)) {
                pinned::announce(book, newly, reason);
            }
        }
        // A refresh's newly done steps stay until it ends, for its notification to count them.
        let refreshing = &self.refreshing;
        self.newly.retain(|slug, _| refreshing.contains_key(slug));
    }

    /// The bridge's answers; true when one came.
    pub fn take_replies(&mut self) -> bool {
        let mut any = false;
        while let Ok(reply) = self.bridge.replies.try_recv() {
            any = true;
            self.calls = self.calls.saturating_sub(1);
            if let Some(slug) = reply.tag.strip_prefix("refresh:") {
                self.asking.remove(slug);
            }
            if let Some(error) = reply.error {
                self.toasts.push(error, toast::Kind::Bad);
            }
            self.toasts.push(reply.note, toast::Kind::Good);
            if let Some(started) = reply.refreshing.as_object() {
                for (slug, pid) in started {
                    if let Some(pid) = pid.as_u64() {
                        self.refreshing.insert(slug.clone(), (pid as u32, Instant::now()));
                    }
                }
            }
        }
        any
    }

    /// Lets every glide reach its end at once, for a snapshot that does not depend on how fast the test runs.
    pub fn settle(&mut self) {
        for glide in self.scroll.values_mut().chain(self.gauges.values_mut()) {
            glide.set(glide.target());
        }
        self.underline.0.set(self.underline.0.target());
        self.underline.1.set(self.underline.1.target());
    }

    /// Another tab, its first shown step selected and its scroll where it was left.
    pub fn go_tab(&mut self, i: usize) {
        if i != self.active && i < self.books.len() {
            self.active = i;
            self.selected = 0;
            self.keep_cursor();
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

pub fn is_alive(pid: u32) -> bool {
    std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}
