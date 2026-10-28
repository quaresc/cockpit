//! The inbox app: the tree rebuilt when the caches change or the ages move, what changed since the last rebuild shown
//! on its rows, details fetched for the highlighted PR; a frame drawn only when something changed.

use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cockpit_core::acts::Act;
use cockpit_core::anim::{self, Effects};
use cockpit_core::bridge::Bridge;
use cockpit_core::dialog::Dialog;
use cockpit_core::files::{self, Density};
use cockpit_core::focus;
use cockpit_core::input::Input;
use cockpit_core::palette::Palette;
use cockpit_core::term;
use cockpit_core::theme::base;
use cockpit_core::toast::{Kind as Toast, Toasts};
use cockpit_core::widgets::filter_line;
use crossterm::event::{Event, KeyEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};
use serde_json::Value;

use crate::changes::{self, Snapshot};
use crate::details::{Details, Fetched};
use crate::facts::{Config, is_failing, list, s};
use crate::history::History;
use crate::inputs::Inputs;
use crate::model::{self, Ctx};
use crate::panel;
use crate::requests::Requests;
use crate::row::{Call, Kind};
use crate::select::{self, Picked};
use crate::sessions::Local;
use crate::tree::{Paint, Tree};

/// The cache files' times at the last rebuild, and the 30 s period it fell in: the ages move every period.
type Stamp = (Vec<Option<SystemTime>>, u64);

pub(crate) enum Command {
    Act(String, usize),
    GoTo(String),
    Refresh,
    Regroup,
    Filter,
}

/// What a dialog's answer runs.
pub(crate) enum Asked {
    Armed(Call, String),
    Picks(Vec<Call>),
    /// A batch: each picked row's call, armed when it changes something.
    Batch(String, Vec<(Call, Option<String>)>),
    /// A batch whose action asks where first: each picked row's calls, one per choice.
    BatchPicks(String, Vec<Vec<Call>>),
}

pub struct App {
    config: Config,
    local: Local,
    bridge: Bridge,
    in_flight: usize,
    requests: Requests,
    pub(crate) tree: Tree,
    head: Vec<Line<'static>>,
    pub(crate) by_status: bool,
    compact: bool,
    /// False for fixtures: no file is read or watched, nothing is fetched.
    live: bool,
    inputs: Inputs,
    seen: Option<Stamp>,
    _watcher: Option<notify::RecommendedWatcher>,
    changes: Option<Receiver<()>>,
    settled: Instant,
    pub(crate) palette: Palette,
    pub(crate) commands: Vec<Command>,
    pub(crate) dialog: Option<(Dialog, Asked)>,
    pub(crate) toasts: Toasts,
    pub(crate) buttons: Vec<(Rect, usize)>,
    pub(crate) filter: Input,
    pub(crate) editing: bool,
    pub(crate) picked: Picked,
    effects: Effects,
    animating: bool,
    /// A toast shows, which goes on its own.
    toasting: bool,
    snapshot: Option<Snapshot>,
    details: Details,
    history: History,
    focus_seen: SystemTime,
}

impl App {
    fn base(config: Config, local: Local, compact: bool, live: bool, inputs: Inputs, series: HashMap<String, Vec<f64>>) -> Self {
        let mut palette = Palette::default();
        palette.app = "inbox".into();
        let mut tree = Tree::default();
        tree.compact = compact;
        Self {
            config,
            local,
            bridge: Bridge::default(),
            in_flight: 0,
            requests: Requests::default(),
            tree,
            head: Vec::new(),
            by_status: false,
            compact,
            live,
            inputs,
            seen: None,
            _watcher: None,
            changes: None,
            settled: Instant::now(),
            palette,
            commands: Vec::new(),
            dialog: None,
            toasts: Toasts::default(),
            buttons: Vec::new(),
            filter: Input::default(),
            editing: false,
            picked: Picked::default(),
            effects: Effects::default(),
            animating: false,
            toasting: false,
            snapshot: None,
            details: Details::new(live),
            history: History::new(live, series),
            focus_seen: SystemTime::now(),
        }
    }

    /// The app on the real caches, watched.
    pub fn start() -> Self {
        let dirs = [files::cache(), files::config_dir(), files::cache().join("focus")];
        let (watcher, changes) = files::watch(&dirs);
        let compact = files::density() == Density::Compact;
        let mut app = Self::base(Config::load(), Local::default(), compact, true, Inputs::default(), HashMap::new());
        app._watcher = watcher;
        app.changes = Some(changes);
        app.reload(true);
        app
    }

    /// The app on given data, for tests: nothing read, watched or fetched.
    pub fn fixture(inputs: Inputs, config: Value, compact: bool, local: Local, series: HashMap<String, Vec<f64>>) -> Self {
        let mut app = Self::base(Config::from(config), local, compact, false, inputs, series);
        app.rebuild();
        app
    }

    /// New data for a fixture app, as a rebuild after the caches changed would see it.
    pub fn feed(&mut self, inputs: Inputs) {
        self.inputs = inputs;
        self.rebuild();
    }

    /// Details for a PR as the hub would answer them, by kind (review_threads, failed_tests, ci_log).
    pub fn show_details(&mut self, url: &str, answers: &[(&str, Value)]) {
        let mut fetched = Fetched::default();
        for (kind, answer) in answers {
            fetched.absorb(kind, answer);
        }
        self.details.set(url, fetched);
    }

    /// Whether some row's effect still plays, for tests.
    pub fn playing(&mut self) -> bool {
        self.effects.active()
    }

    fn stamp(&self) -> Stamp {
        let paths = [files::cache().join("inbox.json"), files::config_dir().join("muted.json"), files::cache().join("ci.json"), files::cache().join("brief.json")];
        let times = paths.iter().map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()).collect();
        (times, period())
    }

    /// Reads the caches again when they changed or the ages moved; true when it did.
    fn reload(&mut self, force: bool) -> bool {
        let seen = self.stamp();
        if !force && self.seen.as_ref() == Some(&seen) {
            return false;
        }
        self.seen = Some(seen);
        self.inputs = Inputs::read();
        let ids = model::usable(&self.inputs).map(|d| list(d, "tickets").iter().map(|t| s(t, "id").to_string()).collect()).unwrap_or_default();
        self.history.want(ids);
        self.rebuild();
        true
    }

    /// The rows from the inputs at hand; what changed since the last rebuild starts its effect.
    pub(crate) fn rebuild(&mut self) {
        let ctx = Ctx { config: &self.config, by_status: self.by_status, compact: self.compact, series: self.history.series() };
        let (head, top) = model::model(&self.inputs, &ctx, &mut self.local);
        self.head = head;
        self.tree.replace(top);
        self.picked.keep_known(&self.tree);
        let Some(data) = model::usable(&self.inputs) else {
            self.snapshot = None;
            return;
        };
        let snapshot = changes::snapshot(data, &self.inputs.ci, &self.local.agents());
        if let Some(before) = &self.snapshot {
            for (key, change) in changes::diff(before, &snapshot) {
                self.effects.start(&key, change.effect());
                self.details.forget(&key);
            }
        }
        self.snapshot = Some(snapshot);
    }

    /// The bridge's replies, the files' changes, focus requests and fetches, whichever came in; true when any did.
    fn pump(&mut self) -> bool {
        let mut changed = false;
        let mut force = false;
        while let Ok(reply) = self.bridge.replies.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            if let Some(error) = reply.error {
                self.toasts.push(error, Toast::Bad);
            }
            self.toasts.push(reply.note, Toast::Note);
            self.requests.merge(&reply.pending);
            force = true;
        }
        if self.history.pump() {
            self.rebuild();
            changed = true;
        }
        changed |= self.details.pump();
        if !self.live {
            return changed;
        }
        let files_changed = self.changes.as_ref().is_some_and(|rx| rx.try_iter().count() > 0);
        if self.settled.elapsed() >= Duration::from_secs(1) {
            self.settled = Instant::now();
            let before = self.requests.count();
            for note in self.requests.settle() {
                self.toasts.push(note, Toast::Note);
            }
            changed |= self.requests.count() != before;
        }
        if let Some(target) = focus::take("inbox", &mut self.focus_seen) {
            self.focus(&target);
            changed = true;
        }
        if force || files_changed || self.seen.as_ref().is_none_or(|s| s.1 != period()) {
            changed |= self.reload(force);
        }
        changed
    }

    /// Brings the row a palette jump or another app asked for into view, and flashes it.
    pub fn focus(&mut self, target: &Value) {
        let key = target["key"].as_str().or(target.as_str()).unwrap_or_default().to_string();
        let Some(node) = self.tree.by_key(&key) else {
            self.toasts.push(format!("{key} is not in the inbox any more"), Toast::Note);
            return;
        };
        if self.tree.filtering() {
            self.clear_filter();
        }
        self.tree.go_to(node);
        self.effects.start(&key, anim::tone("focus"));
    }

    pub(crate) fn clear_filter(&mut self) {
        self.filter = Input::default();
        self.editing = false;
        self.tree.set_filter("");
    }

    pub(crate) fn call(&mut self, call: Call, armed: Option<String>) {
        // A `run` call is a plain command: its argv runs here, off the keys' thread, without the hub.
        if call.function == "run" {
            let argv: Vec<String> = call.args.first().and_then(|a| a.as_array()).into_iter().flatten().filter_map(|a| a.as_str().map(str::to_string)).collect();
            if let Some((program, args)) = argv.split_first() {
                let (program, args) = (program.clone(), args.to_vec());
                std::thread::spawn(move || std::process::Command::new(program).args(args).output());
            }
            return;
        }
        self.in_flight += 1;
        self.bridge.call(call.function, "inbox", call.function, call.args, armed);
    }

    /// The panel's lines and buttons: the picked rows' summary and shared actions, else the highlighted row's details.
    fn panel_content(&mut self) -> (Vec<Line<'static>>, Vec<Act>) {
        if !self.picked.is_empty() {
            let rows = self.picked.rows(&self.tree);
            let shared = select::shared(&self.tree, &rows);
            let names: Vec<String> = rows.iter().map(|&n| self.tree.nodes[n].row.name.clone()).collect();
            return (panel::picked_info(&select::describe(&self.tree, &rows), &names, !shared.is_empty()), shared);
        }
        let Some(node) = self.tree.current() else { return (Vec::new(), Vec::new()) };
        let row = &self.tree.nodes[node].row;
        let mut info = row.details();
        let acts = row.acts.iter().map(|a| a.act.clone()).collect();
        if let (Kind::Pr | Kind::Review, Some(pr)) = (row.kind, row.item.clone()) {
            let run = list(&self.inputs.ci, "runs").iter().find(|r| s(r, "repo") == s(&pr, "repo") && s(r, "branch") == s(&pr, "branch")).cloned();
            let failing = is_failing(&pr) || run.as_ref().is_some_and(|r| s(r, "outcome") == "failed");
            let url = row.key.clone();
            self.details.want(&url, &pr, run.as_ref(), failing);
            info.extend(self.details.lines(&url));
        }
        (info, acts)
    }

    /// Whether the highlighted row waits for its details, for the spinner to turn.
    fn waiting(&self) -> bool {
        !self.compact && self.tree.current().is_some_and(|n| self.details.loading(&self.tree.nodes[n].row.key))
    }

    /// Notes whether effects play and toasts show, which want frames of their own.
    fn pace(&mut self) {
        self.animating = self.effects.active();
        self.toasting = self.toasts.any();
    }

    pub(crate) fn row_acts(&self) -> Vec<Act> {
        self.tree.current().map(|n| self.tree.nodes[n].row.acts.iter().map(|a| a.act.clone()).collect()).unwrap_or_default()
    }
}

/// The 30 s period now falls in.
fn period() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) / 30
}

impl term::App for App {
    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::new().style(base()), area);
        let head_height = panel::height(&self.head, area.width.saturating_sub(2)).max(1);
        let (info, acts) = if self.compact { (Vec::new(), Vec::new()) } else { self.panel_content() };
        let panel_height = if self.compact { 0 } else { panel::panel_height(&info, &acts, area.width).min(area.height / 2) };
        let bar = u16::from(self.editing || self.tree.filtering());
        let [head, filter, tree, bottom, footer] =
            Layout::vertical([Constraint::Length(head_height), Constraint::Length(bar), Constraint::Min(1), Constraint::Length(panel_height), Constraint::Length(1)]).areas(area);
        panel::header(frame, head, &self.head);
        if bar > 0 {
            frame.render_widget(Paragraph::new(filter_line(&self.filter, self.editing, self.tree.matched())), panel::inset(filter));
        }
        let row_acts = if self.compact && self.picked.is_empty() { self.row_acts() } else { Vec::new() };
        let paint = Paint { requests: &self.requests, picked: &self.picked.keys, effects: &mut self.effects, buttons: self.compact.then_some(row_acts.as_slice()) };
        let clicks = self.tree.render(frame, panel::inset(tree), paint);
        self.buttons = if self.compact { clicks } else { panel::panel(frame, bottom, &info, &acts) };
        if self.compact && !self.picked.is_empty() {
            let rows = self.picked.rows(&self.tree);
            let shared = select::shared(&self.tree, &rows);
            self.buttons = panel::picked_footer(frame, footer, &select::describe(&self.tree, &rows), &shared);
        } else {
            panel::footer(frame, footer, !self.picked.is_empty());
        }
        if let Some((dialog, _)) = &self.dialog {
            dialog.render(frame, area);
        }
        if self.palette.is_open {
            self.palette.render(frame, area);
        }
        self.toasts.render(frame, Rect { height: area.height.saturating_sub(1), ..area });
    }

    fn event(&mut self, event: Event) -> bool {
        let quit = match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.key(key),
            Event::Mouse(mouse) => {
                self.mouse(mouse);
                false
            }
            _ => false,
        };
        self.pump();
        self.pace();
        quit
    }

    fn tick(&mut self) -> bool {
        let changed = self.pump();
        let (was_animating, was_toasting) = (self.animating, self.toasting);
        self.pace();
        // One more frame after the last effect or toast goes, to draw the screen at rest.
        changed || self.animating || was_animating || self.toasting || was_toasting || self.waiting()
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(if self.animating {
            33
        } else if self.toasting || self.waiting() || self.in_flight > 0 {
            100
        } else {
            1000
        })
    }
}
