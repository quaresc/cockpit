//! The app's state and what changes it: the document the hub pushes, the clock moving "in 4 min" and the running
//! times, focus requests, folds and the selection kept by key, what changed since you last looked, the form, and the
//! actions sent through the bridge.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

use cockpit_core::bridge::Bridge;
use cockpit_core::dialog::Dialog;
use cockpit_core::files::{self, Density};
use cockpit_core::hub::{self, Docs};
use cockpit_core::palette::Palette;
use cockpit_core::seen::Seen;
use cockpit_core::toast::{Kind as Toast, Toasts};
use ratatui::layout::Rect;
use serde_json::Value;

use crate::data::{Clock, Doc};
use crate::form::Form;
use crate::pinned;

/// What a palette entry, a key or a button runs; a routine by its slug, a run by its id.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Add,
    Run(String),
    Stop(String),
    Pause(String, bool),
    /// A routine and the model it runs on next.
    Model(String, String),
    Delete(String),
    Edit(String),
    Output(String, Option<String>),
    Talk(String, Option<String>),
    Fold(String),
    Refresh,
    GoTo(String),
}

/// A call to the hub, as the app sent it: the action, its arguments and the confirmation it carries.
#[derive(Clone, Debug, PartialEq)]
pub struct Sent {
    pub action: String,
    pub args: Vec<Value>,
    pub armed: Option<String>,
}

/// Where the last frame drew what a click can land on.
#[derive(Default)]
pub struct Hits {
    pub list: Rect,
    /// Each row's line and the cells of its fold glyph.
    pub rows: Vec<(Rect, String, Rect)>,
    pub buttons: Vec<(Rect, Cmd)>,
}

pub struct App {
    /// None until the hub first writes the document.
    pub doc: Option<Doc>,
    docs: Option<Docs>,
    seen: Option<u64>,
    pub clock: Clock,
    fixed_clock: bool,
    second: u64,
    pub density: Density,
    pub selected: Option<String>,
    pub opened: HashSet<String>,
    pub scroll: usize,
    pub follow: bool,
    pub toasts: Toasts,
    toasting: bool,
    pub palette: Palette,
    pub cmds: Vec<Cmd>,
    /// A question asked before a call: the call and the confirmation it carries once answered yes.
    pub dialog: Option<(Dialog, Cmd, String)>,
    pub form: Option<Form>,
    bridge: Bridge,
    calls: usize,
    /// Every call sent, newest last: what tests read.
    pub sent: Vec<Sent>,
    pub hits: Hits,
    focus_seen: SystemTime,
    pinned: bool,
    reported: Option<String>,
    live: bool,
    /// What changed since you last looked at the routines.
    pub looked: Seen,
    /// Where the runs' outputs are, and the ones read so far.
    pub runs_dir: PathBuf,
    outputs: HashMap<(String, String), Vec<String>>,
}

impl App {
    /// The app on the document the hub pushes, the clock running.
    pub fn start() -> Self {
        let mut app = Self::with(None, files::density(), pinned::is_pinned(), files::cache().join("routines"));
        app.live = true;
        app.docs = Some(hub::subscribe(&["routines"]));
        app.looked = Seen::start("routines");
        app.reload();
        app
    }

    /// The app at this time (None is the system's), with no document yet, its runs' outputs in `runs_dir`.
    pub fn with(clock: Option<Clock>, density: Density, pinned: bool, runs_dir: PathBuf) -> Self {
        let mut palette = Palette::default();
        palette.app = "routines".into();
        Self {
            doc: None,
            docs: None,
            seen: None,
            clock: clock.unwrap_or_else(Clock::system),
            fixed_clock: clock.is_some(),
            second: 0,
            density,
            selected: None,
            opened: HashSet::new(),
            scroll: 0,
            follow: true,
            toasts: Toasts::default(),
            toasting: false,
            palette,
            cmds: Vec::new(),
            dialog: None,
            form: None,
            bridge: Bridge::default(),
            calls: 0,
            sent: Vec::new(),
            hits: Hits::default(),
            focus_seen: SystemTime::now(),
            pinned,
            reported: None,
            live: false,
            looked: Seen::fixture(0, Vec::new()),
            runs_dir,
            outputs: HashMap::new(),
        }
    }

    /// The hub's events and the time you last looked, as tests give them.
    pub fn feed_events(&mut self, since_ms: u64, events: Vec<Value>) {
        self.looked = Seen::fixture(since_ms, events);
    }

    /// A document given whole: what tests draw.
    pub fn set_doc(&mut self, value: &Value) {
        self.doc = Doc::parse(value);
        self.select_default();
    }

    /// The selection on the first row, unless it is on one still shown.
    fn select_default(&mut self) {
        let rows = self.rows();
        if self.selected.as_ref().is_some_and(|k| rows.iter().any(|r| &r.key == k)) {
            return;
        }
        self.selected = rows.first().map(|r| r.key.clone());
    }

    /// Reads the document again when the hub pushed a new one; true when it did.
    fn reload(&mut self) -> bool {
        let Some(docs) = &mut self.docs else { return false };
        docs.pump();
        if self.seen == Some(docs.revision) {
            return false;
        }
        self.seen = Some(docs.revision);
        self.doc = docs.get("routines").and_then(Doc::parse);
        self.select_default();
        self.looked.fetch();
        true
    }

    pub fn report(&mut self) {
        let Some(doc) = &self.doc else { return };
        let (text, bad) = pinned::sidebar_text(doc, &self.clock);
        if self.pinned && self.reported.as_ref() != Some(&text) {
            pinned::report(text.clone(), bad);
        }
        self.reported = Some(text);
    }

    /// A palette jump or a notification click: that routine (or run) selected and in view.
    fn take_focus(&mut self) -> bool {
        let Some(target) = cockpit_core::focus::take("routines", &mut self.focus_seen) else { return false };
        let Some(key) = target["key"].as_str() else { return false };
        self.focus(key);
        true
    }

    pub fn focus(&mut self, key: &str) {
        let slug = key.split('/').next().unwrap_or(key);
        if self.doc.as_ref().and_then(|d| d.routine(slug)).is_none() {
            self.toasts.push(format!("no routine {slug} any more"), Toast::Note);
            return;
        }
        if key != slug {
            self.opened.insert(slug.to_string());
        }
        self.selected = Some(key.to_string());
        self.follow = true;
    }

    /// Whether anything moves on screen: a run's time, a call on its way, a toast.
    pub fn is_busy(&self) -> bool {
        self.calls > 0 || self.toasting || self.doc.as_ref().is_some_and(|d| !d.running.is_empty())
    }

    pub fn tick(&mut self) -> bool {
        if !self.fixed_clock {
            self.clock = Clock::system();
        }
        let mut changed = self.take_replies();
        let reloaded = self.reload();
        let second = self.clock.second();
        let running = self.doc.as_ref().is_some_and(|d| !d.running.is_empty());
        // The spinner and the running time move every tick; "in 4 min" every minute.
        if reloaded || (running && second != self.second) || second / 60 != self.second / 60 {
            self.report();
            changed = true;
        }
        self.second = second;
        changed |= running;
        changed |= self.take_focus();
        changed |= self.looked.pump();
        let toasting = self.toasts.any();
        changed |= toasting || self.toasting;
        self.toasting = toasting;
        changed
    }

    /// A run's output, as lines, once it is written.
    pub fn output(&mut self, slug: &str, id: &str) -> Vec<String> {
        let key = (slug.to_string(), id.to_string());
        if let Some(lines) = self.outputs.get(&key) {
            return lines.clone();
        }
        let Ok(text) = std::fs::read_to_string(self.runs_dir.join(slug).join(format!("{id}.txt"))) else { return Vec::new() };
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        self.outputs.insert(key, lines.clone());
        lines
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Actions

    pub fn take_replies(&mut self) -> bool {
        let mut any = false;
        while let Ok(reply) = self.bridge.replies.try_recv() {
            any = true;
            self.calls = self.calls.saturating_sub(1);
            if let Some(error) = reply.error {
                self.toasts.push(error, Toast::Bad);
            }
            self.toasts.push(reply.note, Toast::Good);
        }
        any
    }

    pub fn call(&mut self, action: &str, args: Vec<Value>, armed: Option<String>) {
        self.sent.push(Sent { action: action.into(), args: args.clone(), armed: armed.clone() });
        if self.live {
            self.calls += 1;
            self.bridge.call(action, "routines", action, args, armed);
        }
    }

    /// Runs a command; `armed` is the confirmation a dialog already asked for.
    pub fn run(&mut self, cmd: Cmd, armed: Option<String>) {
        let id = |id: Option<String>| id.map_or(Value::Null, Value::from);
        match cmd {
            Cmd::Add => self.form = Some(Form::new()),
            Cmd::Run(slug) => self.call("run", vec![slug.into()], None),
            Cmd::Stop(slug) => self.call("stop", vec![slug.into()], None),
            Cmd::Pause(slug, paused) => self.call("pause", vec![slug.into(), paused.into()], None),
            Cmd::Model(slug, model) => self.call("model", vec![slug.into(), model.into()], None),
            Cmd::Delete(slug) => self.call("delete", vec![slug.into()], armed),
            Cmd::Edit(slug) => self.call("edit", vec![slug.into()], None),
            Cmd::Output(slug, run) => self.call("output", vec![slug.into(), id(run)], None),
            Cmd::Talk(slug, run) => self.call("talk", vec![slug.into(), id(run)], None),
            Cmd::Refresh => self.call("request_refresh", vec![], None),
            Cmd::Fold(slug) => self.toggle(&slug),
            Cmd::GoTo(key) => self.focus(&key),
        }
    }

    /// The form's values sent as a new routine.
    pub fn submit(&mut self, values: Vec<String>) {
        self.form = None;
        self.call("add", values.into_iter().map(Value::from).collect(), None);
    }

    /// Opens or folds a routine's runs.
    pub fn toggle(&mut self, slug: &str) {
        if !self.opened.remove(slug) {
            self.opened.insert(slug.to_string());
        }
    }
}
