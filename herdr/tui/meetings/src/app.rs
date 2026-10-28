//! The app's state and what changes it: the week the hub pushes and the briefs read again when they change, the minute
//! ticking the "in N min" texts, focus requests, the actions sent through the bridge, and the pulse of a meeting about to
//! start.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime};

use cockpit_core::anim::{Effects, Kind};
use cockpit_core::bridge::Bridge;
use cockpit_core::files::{self, Density};
use cockpit_core::hub::{self, Docs};
use cockpit_core::palette::Palette;
use cockpit_core::toast::{Kind as Toast, Toasts};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui_image::picker::Picker;
use serde_json::Value;

use crate::picture::Picture;
use crate::pinned;
use crate::week::{self, Clock, Meeting, Week};

/// A meeting about to start beats in the accent colour, every few seconds until it does.
const PULSE: Kind = Kind::Pulse(Color::Rgb(96, 66, 44), 2, 1.4);
const PULSE_EVERY: Duration = Duration::from_secs(4);

/// Where the week came from at the last read, to tell when it changed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stamp {
    Pushed(u64),
    File(Option<SystemTime>),
}

/// What a palette entry or a button runs.
#[derive(Clone, Debug)]
pub enum Cmd {
    Join(String),
    Open(String),
    Brief(String),
    Prepare(String),
    GoTo(String),
    Week,
}

/// Where the last frame drew what a click can land on.
#[derive(Default)]
pub struct Hits {
    pub list: Rect,
    pub rows: Vec<(Rect, String)>,
    pub titles: Vec<(Rect, String)>,
    pub buttons: Vec<(Rect, Cmd)>,
    pub picture: Rect,
}

pub struct App {
    /// None until the mod first writes the cache.
    pub week: Option<Week>,
    /// Bumped on every read, for the week view to know it must draw again.
    pub version: u64,
    seen: (Stamp, Option<SystemTime>),
    pub briefs: HashMap<String, Vec<String>>,
    pub preparing: HashSet<String>,
    pub clock: Clock,
    fixed_clock: bool,
    minute: i64,
    pub selected: Option<String>,
    pub open: HashSet<String>,
    pub week_view: bool,
    pub picture: Picture,
    pub density: Density,
    pub brief_enabled: bool,
    pub fx: Effects,
    pub animating: bool,
    pulsed: HashMap<String, Instant>,
    pub toasts: Toasts,
    toasting: bool,
    pub palette: Palette,
    pub cmds: Vec<Cmd>,
    pub bridge: Bridge,
    calls: usize,
    focus_seen: SystemTime,
    notified_seen: SystemTime,
    pinned: bool,
    reported: Option<String>,
    pub hits: Hits,
    pub scroll: usize,
    pub follow: bool,
    /// A meeting to bring near the top on the next frame: a notification or palette jump.
    pub reveal: Option<String>,
    /// The week as the hub pushes it; None reads the cache file.
    docs: Option<Docs>,
}

impl App {
    /// The app on the real cache, the clock running.
    pub fn start(picker: Picker) -> Self {
        let config = files::config();
        let mut app = Self::with(None, &config, files::density(), picker, pinned::is_pinned());
        app.docs = Some(hub::subscribe(&["meetings"]));
        app.reload();
        app
    }

    /// The app on this week (None reads the cache) at this time (None is the system's), for the binary and tests.
    pub fn with(clock: Option<Clock>, config: &Value, density: Density, picker: Picker, pinned: bool) -> Self {
        let mut palette = Palette::default();
        palette.app = "meetings".into();
        Self {
            week: None,
            version: 0,
            seen: (Stamp::File(None), None),
            briefs: HashMap::new(),
            preparing: HashSet::new(),
            clock: clock.unwrap_or_else(Clock::system),
            fixed_clock: clock.is_some(),
            minute: 0,
            selected: None,
            open: HashSet::new(),
            week_view: false,
            picture: Picture::new(picker),
            density,
            brief_enabled: config["meetingBrief"]["enabled"].as_bool().unwrap_or(true),
            fx: Effects::default(),
            animating: false,
            pulsed: HashMap::new(),
            toasts: Toasts::default(),
            toasting: false,
            palette,
            cmds: Vec::new(),
            bridge: Bridge::default(),
            calls: 0,
            focus_seen: SystemTime::now(),
            notified_seen: SystemTime::now(),
            pinned,
            reported: None,
            hits: Hits::default(),
            scroll: 0,
            follow: true,
            reveal: None,
            docs: None,
        }
    }

    /// A week given whole, its briefs too: what tests draw.
    pub fn set_week(&mut self, data: &Value, briefs: HashMap<String, Vec<String>>) {
        self.week = Some(Week::parse(data));
        self.briefs = briefs;
        self.version += 1;
        self.select_default();
    }

    /// The meetings in the list, in order.
    pub fn shown(&self) -> Vec<&Meeting> {
        self.week.as_ref().map(|w| w.shown(&self.clock)).unwrap_or_default()
    }

    pub fn meeting(&self, id: &str) -> Option<&Meeting> {
        self.week.as_ref()?.meetings.iter().find(|m| m.id == id)
    }

    pub fn selected_meeting(&self) -> Option<&Meeting> {
        self.meeting(self.selected.as_deref()?)
    }

    /// The selection on the running or next meeting, unless it is on one still shown.
    fn select_default(&mut self) {
        let shown = self.shown();
        if self.selected.as_ref().is_some_and(|id| shown.iter().any(|m| &m.id == id)) {
            return;
        }
        let upcoming = shown.iter().find(|m| m.end > self.clock.now).or(shown.first());
        self.selected = upcoming.map(|m| m.id.clone());
    }

    pub fn move_sel(&mut self, by: isize) {
        let ids: Vec<String> = self.shown().iter().map(|m| m.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|id| ids.iter().position(|i| i == id)).unwrap_or(0) as isize;
        self.selected = Some(ids[(at + by).clamp(0, ids.len() as isize - 1) as usize].clone());
        self.follow = true;
    }

    pub fn select_edge(&mut self, last: bool) {
        let shown = self.shown();
        let edge = if last { shown.last() } else { shown.first() };
        self.selected = edge.map(|m| m.id.clone());
        self.follow = true;
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Reading again

    /// The week's revision as pushed (else its file's time), and the briefs folder's time.
    fn stamps(&mut self) -> (Stamp, Option<SystemTime>) {
        let at = |p: std::path::PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let week = match &mut self.docs {
            Some(docs) => {
                docs.pump();
                Stamp::Pushed(docs.revision)
            }
            None => Stamp::File(at(week::cache_path())),
        };
        (week, at(week::briefs_dir()))
    }

    /// The week and the briefs read again when either changed; true when they were.
    fn reload(&mut self) -> bool {
        let now = self.stamps();
        if now == self.seen && self.version > 0 {
            return false;
        }
        self.seen = now;
        let data = match &self.docs {
            Some(docs) => docs.get("meetings").cloned(),
            None => files::load(&week::cache_path()),
        };
        self.week = data.map(|data| Week::parse(&data));
        self.version += 1;
        self.briefs.clear();
        self.preparing.clear();
        let ids: Vec<String> = self.week.iter().flat_map(|w| w.meetings.iter().map(|m| m.id.clone())).collect();
        for id in ids {
            if let Some(brief) = week::read_brief(&id) {
                self.briefs.insert(id.clone(), brief);
            }
            if week::is_preparing(&id) {
                self.preparing.insert(id);
            }
        }
        self.select_default();
        true
    }

    pub fn report(&mut self) {
        let Some(week) = &self.week else { return };
        let text = pinned::next_text(week, &self.clock);
        if self.pinned && self.reported.as_ref() != Some(&text) {
            pinned::report(text.clone());
        }
        self.reported = Some(text);
    }

    /// A notification click (meetings-focus.json) or a palette jump (the core's focus request): that meeting selected,
    /// in view, and its brief open.
    fn take_focus(&mut self) -> bool {
        let notified = pinned::take_focus(&mut self.notified_seen);
        let jumped = cockpit_core::focus::take("meetings", &mut self.focus_seen).and_then(|t| t["key"].as_str().map(str::to_string));
        let Some(id) = jumped.or(notified) else { return false };
        self.focus(&id);
        true
    }

    pub fn focus(&mut self, id: &str) {
        if self.meeting(id).is_none() {
            self.toasts.push("that meeting is not in this week's calendar any more", Toast::Note);
            return;
        }
        self.open.insert(id.to_string());
        self.selected = Some(id.to_string());
        self.reveal = Some(id.to_string());
        self.week_view = false;
    }

    /// Every few seconds, a beat on each meeting starting within five minutes.
    fn pulse(&mut self) -> bool {
        let soon: Vec<String> = self.shown().iter().filter(|m| m.is_soon(&self.clock)).map(|m| m.id.clone()).collect();
        self.pulsed.retain(|id, _| soon.contains(id));
        let mut started = false;
        for id in soon {
            if self.pulsed.get(&id).is_none_or(|at| at.elapsed() >= PULSE_EVERY) {
                self.fx.start(&id, PULSE);
                self.pulsed.insert(id, Instant::now());
                started = true;
            }
        }
        self.animating |= started;
        started
    }

    pub fn is_animating(&self) -> bool {
        self.animating
    }

    pub fn is_busy(&self) -> bool {
        self.calls > 0 || self.picture.is_drawing()
    }

    pub fn tick(&mut self) -> bool {
        if !self.fixed_clock {
            self.clock = Clock::system();
        }
        let mut changed = self.take_replies();
        changed |= self.picture.take();
        let minute = self.clock.minute();
        let reloaded = self.reload();
        if reloaded || minute != self.minute {
            self.minute = minute;
            self.report();
            changed = true;
        }
        changed |= self.take_focus();
        changed |= self.pulse();
        let toasting = self.toasts.any();
        changed |= toasting || self.toasting;
        self.toasting = toasting;
        let animating = self.fx.active();
        changed |= animating || self.animating;
        self.animating = animating;
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

    fn call(&mut self, tag: &str, action: &str, args: Vec<Value>) {
        self.calls += 1;
        self.bridge.call(tag, "meetings", action, args, None);
    }

    pub fn run(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Join(id) => {
                if let Some(url) = self.meeting(&id).and_then(|m| m.join.clone()) {
                    self.call("join", "open_url", vec![url.into()]);
                }
            }
            Cmd::Open(id) => {
                if let Some(url) = self.meeting(&id).map(|m| m.url.clone()).filter(|u| !u.is_empty()) {
                    self.call("open", "open_url", vec![url.into()]);
                }
            }
            Cmd::Brief(id) => {
                if !self.open.remove(&id) {
                    self.open.insert(id);
                }
            }
            Cmd::Prepare(id) => {
                if let Some(raw) = self.meeting(&id).map(|m| m.raw.clone()) {
                    self.open.insert(id.clone());
                    self.call(&format!("prepare:{id}"), "prepare", vec![raw]);
                }
            }
            Cmd::GoTo(id) => {
                self.selected = Some(id.clone());
                self.reveal = Some(id);
            }
            Cmd::Week => self.week_view = !self.week_view,
        }
    }
}
