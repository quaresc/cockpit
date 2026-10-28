//! The app's state and what changes it: the document the hub pushes, the minute moving the "2 h ago" texts, focus
//! requests, folds and the selection kept by key, and the actions sent through the bridge.

use std::collections::HashSet;
use std::time::SystemTime;

use cockpit_core::bridge::Bridge;
use cockpit_core::dialog::Dialog;
use cockpit_core::files::{self, Density};
use cockpit_core::hub::{self, Docs};
use cockpit_core::palette::Palette;
use cockpit_core::toast::{Kind as Toast, Toasts};
use ratatui::layout::Rect;
use serde_json::Value;

use crate::data::{Clock, Doc};
use crate::pinned;

/// What a palette entry, a key or a button runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    OpenUrl(String),
    /// An app and what to look at: {"release": id}, {"issue": id}, {"monitor": id} or {} for the app as a whole.
    Investigate(String, Value),
    Fix(String, String),
    Bug(String, Value),
    /// An app, its release and the tag it goes back to.
    RollBack(String, String, String),
    Talk(String, String),
    Pin(String, bool),
    Refresh,
    Fold(String),
    View,
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
    /// None until the mod first writes the document.
    pub doc: Option<Doc>,
    /// The document as written, its apps handed whole to the hub's actions.
    pub raw: Option<Value>,
    docs: Option<Docs>,
    seen: Option<u64>,
    pub clock: Clock,
    fixed_clock: bool,
    minute: i64,
    pub density: Density,
    pub selected: Option<String>,
    pub opened: HashSet<String>,
    pub closed: HashSet<String>,
    /// Releases across apps, newest first, instead of apps.
    pub timeline: bool,
    pub scroll: usize,
    pub follow: bool,
    pub toasts: Toasts,
    toasting: bool,
    pub palette: Palette,
    pub cmds: Vec<Cmd>,
    /// A question asked before a call: the call and the confirmation it carries once answered yes.
    pub dialog: Option<(Dialog, Cmd, String)>,
    bridge: Bridge,
    calls: usize,
    /// Every call sent, newest last: what tests read.
    pub sent: Vec<Sent>,
    pub hits: Hits,
    focus_seen: SystemTime,
    notified_seen: SystemTime,
    pinned: bool,
    reported: Option<String>,
    live: bool,
}

impl App {
    /// The app on the document the hub pushes, the clock running.
    pub fn start() -> Self {
        let mut app = Self::with(None, files::density(), pinned::is_pinned());
        app.live = true;
        app.docs = Some(hub::subscribe(&["monitor"]));
        app.reload();
        app
    }

    /// The app at this time (None is the system's), with no document yet, for the binary and tests.
    pub fn with(clock: Option<Clock>, density: Density, pinned: bool) -> Self {
        let mut palette = Palette::default();
        palette.app = "monitor".into();
        Self {
            doc: None,
            raw: None,
            docs: None,
            seen: None,
            clock: clock.unwrap_or_else(Clock::system),
            fixed_clock: clock.is_some(),
            minute: 0,
            density,
            selected: None,
            opened: HashSet::new(),
            closed: HashSet::new(),
            timeline: false,
            scroll: 0,
            follow: true,
            toasts: Toasts::default(),
            toasting: false,
            palette,
            cmds: Vec::new(),
            dialog: None,
            bridge: Bridge::default(),
            calls: 0,
            sent: Vec::new(),
            hits: Hits::default(),
            focus_seen: SystemTime::now(),
            notified_seen: SystemTime::now(),
            pinned,
            reported: None,
            live: false,
        }
    }

    /// A document given whole: what tests draw.
    pub fn set_doc(&mut self, value: &Value) {
        self.doc = Doc::parse(value);
        self.raw = Some(value.clone());
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
        let value = docs.get("monitor").cloned();
        self.doc = value.as_ref().and_then(Doc::parse);
        self.raw = value;
        self.select_default();
        true
    }

    pub fn report(&mut self) {
        let Some(doc) = &self.doc else { return };
        let (text, bad) = pinned::sidebar_text(doc);
        if self.pinned && self.reported.as_ref() != Some(&text) {
            pinned::report(text.clone(), bad);
        }
        self.reported = Some(text);
    }

    /// A notification click (monitor-focus.json) or a palette jump: that app selected, unfolded and in view.
    fn take_focus(&mut self) -> bool {
        let notified = pinned::take_focus(&mut self.notified_seen);
        let jumped = cockpit_core::focus::take("monitor", &mut self.focus_seen).and_then(|t| t["key"].as_str().map(str::to_string));
        let Some(app) = jumped.or(notified) else { return false };
        self.focus(&app);
        true
    }

    /// Selects the app known by this id or name, unfolded, in the view by app.
    pub fn focus(&mut self, app: &str) {
        let Some(id) = self.doc.as_ref().and_then(|d| d.apps.iter().find(|a| a.id == app || a.name == app)).map(|a| a.id.clone()) else {
            self.toasts.push(format!("{app} is not in the monitor"), Toast::Note);
            return;
        };
        self.timeline = false;
        self.closed.remove(&id);
        self.opened.insert(id.clone());
        self.selected = Some(id);
        self.follow = true;
    }

    pub fn is_busy(&self) -> bool {
        self.calls > 0
    }

    pub fn tick(&mut self) -> bool {
        if !self.fixed_clock {
            self.clock = Clock::system();
        }
        let mut changed = self.take_replies();
        let reloaded = self.reload();
        let minute = self.clock.minute();
        if reloaded || minute != self.minute {
            self.minute = minute;
            self.report();
            changed = true;
        }
        changed |= self.take_focus();
        let toasting = self.toasts.any();
        changed |= toasting || self.toasting;
        self.toasting = toasting;
        changed
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

    fn call(&mut self, action: &str, args: Vec<Value>, armed: Option<String>) {
        self.sent.push(Sent { action: action.into(), args: args.clone(), armed: armed.clone() });
        if self.live {
            self.calls += 1;
            self.bridge.call(action, "monitor", action, args, armed);
        }
    }

    /// The app as the document has it, with the environment, for the hub's actions.
    fn app_value(&self, id: &str) -> Value {
        let raw = self.raw.as_ref().and_then(|r| r["apps"].as_array()?.iter().find(|a| a["id"] == id).cloned());
        let mut app = raw.unwrap_or(Value::Null);
        if let (Some(fields), Some(doc)) = (app.as_object_mut(), &self.doc) {
            fields.insert("env".into(), Value::from(doc.env.clone()));
        }
        app
    }

    /// Runs a command; `armed` is the confirmation a dialog already asked for.
    pub fn run(&mut self, cmd: Cmd, armed: Option<String>) {
        match cmd {
            Cmd::OpenUrl(url) => self.call("open_url", vec![url.into()], None),
            Cmd::Investigate(app, target) => self.call("investigate", vec![self.app_value(&app), target], None),
            Cmd::Fix(app, issue) => self.call("fix", vec![self.app_value(&app), issue.into()], None),
            Cmd::Bug(app, target) => self.call("bug", vec![self.app_value(&app), target], None),
            Cmd::RollBack(app, release, previous) => self.call("roll_back", vec![self.app_value(&app), release.into(), previous.into()], armed),
            Cmd::Talk(app, release) => self.call("talk", vec![self.app_value(&app), release.into()], None),
            Cmd::Pin(app, pin) => self.call("pin", vec![app.into(), pin.into()], None),
            Cmd::Refresh => self.call("request_refresh", vec![], None),
            Cmd::Fold(app) => self.toggle(&app),
            Cmd::View => {
                self.timeline = !self.timeline;
                self.follow = true;
                self.select_default();
            }
            Cmd::GoTo(key) => {
                if let Some(app) = key.split('/').next().filter(|_| key.contains('/') && !self.timeline) {
                    self.closed.remove(app);
                    self.opened.insert(app.to_string());
                }
                self.selected = Some(key);
                self.follow = true;
            }
        }
    }

    /// Opens or folds an app's rows.
    pub fn toggle(&mut self, app: &str) {
        if self.is_open(app) {
            self.opened.remove(app);
            self.closed.insert(app.to_string());
        } else {
            self.closed.remove(app);
            self.opened.insert(app.to_string());
        }
    }

    /// Whether an app shows its rows: unfolded by hand, else when it is in trouble and not folded by hand.
    pub fn is_open(&self, app: &str) -> bool {
        let troubled = self.doc.as_ref().and_then(|d| d.app(app)).is_some_and(|a| a.is_troubled());
        self.opened.contains(app) || (troubled && !self.closed.contains(app))
    }
}
