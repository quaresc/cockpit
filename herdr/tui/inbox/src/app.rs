//! The inbox app: the tree rebuilt when the caches change or the ages move, keys and clicks run the highlighted row's
//! actions through inbox.py, a dialog asking first when an action changes something.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use cockpit_core::bridge::Bridge;
use cockpit_core::dialog::{Answer, Dialog};
use cockpit_core::files;
use cockpit_core::palette::{self, Entry, Palette};
use cockpit_core::term;
use cockpit_core::theme::base;
use cockpit_core::toast::{Kind, Toasts};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Block;

use crate::facts::Config;
use crate::model;
use crate::panel;
use crate::requests::Requests;
use crate::row::Call;
use crate::sessions::Local;
use crate::tree::Tree;

/// What the cache files and the clock looked like at the last rebuild: a change rebuilds, and so does every 30 s.
type Stamp = (Vec<Option<SystemTime>>, u64, bool);

enum Command {
    Act(String, usize),
    GoTo(String),
    Refresh,
    Regroup,
}

/// What a dialog's answer runs.
enum Asked {
    Armed(Call, String),
    Picks(Vec<Call>),
}

pub struct App {
    config: Config,
    local: Local,
    bridge: Bridge,
    in_flight: usize,
    requests: Requests,
    tree: Tree,
    head: Vec<Line<'static>>,
    by_status: bool,
    seen: Option<Stamp>,
    _watcher: Option<notify::RecommendedWatcher>,
    changes: Receiver<()>,
    settled: Instant,
    palette: Palette,
    commands: Vec<Command>,
    dialog: Option<(Dialog, Asked)>,
    toasts: Toasts,
    buttons: Vec<(Rect, usize)>,
}

impl App {
    pub fn new() -> Self {
        let (watcher, changes) = files::watch(&[files::cache(), files::config_dir()]);
        let mut app = Self {
            config: Config::load(),
            local: Local::default(),
            bridge: Bridge::default(),
            in_flight: 0,
            requests: Requests::default(),
            tree: Tree::default(),
            head: Vec::new(),
            by_status: false,
            seen: None,
            _watcher: watcher,
            changes,
            settled: Instant::now(),
            palette: Palette::default(),
            commands: Vec::new(),
            dialog: None,
            toasts: Toasts::default(),
            buttons: Vec::new(),
        };
        app.reload(true);
        app
    }

    fn stamp(&self) -> Stamp {
        let paths = [files::cache().join("inbox.json"), files::config_dir().join("muted.json"), files::cache().join("ci.json"), files::cache().join("brief.json")];
        let times = paths.iter().map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()).collect();
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        (times, now / 30, self.by_status)
    }

    fn reload(&mut self, force: bool) {
        let seen = self.stamp();
        if force || self.seen.as_ref() != Some(&seen) {
            self.seen = Some(seen);
            let (head, top) = model::model(self.by_status, &self.config, &mut self.local);
            self.head = head;
            self.tree.replace(top);
        }
    }

    /// The bridge's replies, the files' changes and the requests in flight, whichever came in.
    fn pump(&mut self) {
        let mut force = false;
        while let Ok(reply) = self.bridge.replies.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            if let Some(error) = reply.error {
                self.toasts.push(error, Kind::Bad);
            }
            self.toasts.push(reply.note, Kind::Note);
            self.requests.merge(&reply.pending);
            force = true;
        }
        let changed = self.changes.try_iter().count() > 0;
        if self.settled.elapsed() >= Duration::from_secs(1) {
            self.settled = Instant::now();
            for note in self.requests.settle() {
                self.toasts.push(note, Kind::Note);
            }
        }
        if force || changed || self.seen.as_ref().is_none_or(|s| s.1 != self.stamp().1) {
            self.reload(force);
        }
    }

    fn call(&mut self, call: Call, armed: Option<String>) {
        self.in_flight += 1;
        self.bridge.call(call.function, "inbox", call.function, call.args, armed);
    }

    /// Runs a row's action: a choice asks which, a change asks first, the rest runs at once.
    fn act(&mut self, node: usize, n: usize) {
        let row = &self.tree.nodes[node].row;
        let Some(act) = row.acts.get(n) else { return };
        if !act.picks.is_empty() {
            let dialog = Dialog::Pick { question: format!("{}: {}", act.act.label, row.name), options: act.act.choices.clone(), selected: 0 };
            self.dialog = Some((dialog, Asked::Picks(act.picks.clone())));
        } else if let (Some(key), Some(call)) = (&act.act.confirm, &act.call) {
            let dialog = Dialog::Confirm { question: format!("{}: {}?", act.act.label, row.name) };
            self.dialog = Some((dialog, Asked::Armed(call.clone(), key.clone())));
        } else if let Some(call) = act.call.clone() {
            self.call(call, None);
        }
    }

    fn answer(&mut self, answer: Answer) {
        let Some((_, asked)) = self.dialog.take() else { return };
        match (answer, asked) {
            (Answer::Yes, Asked::Armed(call, key)) => self.call(call, Some(key)),
            (Answer::Picked(n), Asked::Picks(calls)) => {
                if let Some(call) = calls.get(n) {
                    self.call(call.clone(), None);
                }
            }
            _ => {}
        }
    }

    fn primary(&mut self) {
        let Some(node) = self.tree.current() else { return };
        match self.tree.nodes[node].row.primary.clone() {
            Some(call) => self.call(call, None),
            None => self.tree.toggle(node),
        }
    }

    fn regroup(&mut self) {
        self.by_status = !self.by_status;
        self.reload(true);
    }

    fn refresh(&mut self) {
        self.call(crate::row::call("request_refresh", vec![]), None);
    }

    /// Every row's actions and a "go to" for each, then refresh and regroup; the main actions show before typing.
    fn open_palette(&mut self) {
        let mut entries = Vec::new();
        self.commands.clear();
        let mut push = |commands: &mut Vec<Command>, text: String, help: String, featured: bool, command: Command| {
            entries.push(Entry { text, help, featured, id: commands.len() });
            commands.push(command);
        };
        for node in &self.tree.nodes {
            let row = &node.row;
            for (n, act) in row.acts.iter().enumerate() {
                push(&mut self.commands, format!("{}  {}", act.act.label, row.name), row.section.clone(), act.act.main, Command::Act(node.path.clone(), n));
            }
            if !row.name.is_empty() {
                push(&mut self.commands, format!("go to  {}", row.name), row.section.clone(), false, Command::GoTo(node.path.clone()));
            }
        }
        push(&mut self.commands, "refresh the inbox".into(), "asks the mod for fresh data".into(), true, Command::Refresh);
        let regroup = format!("group work by {}", if self.by_status { "feature" } else { "status" });
        push(&mut self.commands, regroup, "g".into(), true, Command::Regroup);
        self.palette.open(entries);
    }

    fn run_command(&mut self, id: usize) {
        match self.commands.get(id) {
            Some(Command::Act(path, n)) => {
                let n = *n;
                if let Some(node) = self.tree.node(&path.clone()) {
                    self.act(node, n);
                }
            }
            Some(Command::GoTo(path)) => {
                if let Some(node) = self.tree.node(&path.clone()) {
                    self.tree.go_to(node);
                }
            }
            Some(Command::Refresh) => self.refresh(),
            Some(Command::Regroup) => self.regroup(),
            None => {}
        }
    }

    /// A key; true quits.
    fn key(&mut self, key: KeyEvent) -> bool {
        if let Some((dialog, _)) = &mut self.dialog {
            if let Some(answer) = dialog.key(key) {
                self.answer(answer);
            }
            return false;
        }
        if self.palette.is_open {
            if let palette::Outcome::Chosen(id) = self.palette.key(key) {
                self.run_command(id);
            }
            return false;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => return true,
            KeyCode::Char('p') if ctrl => self.open_palette(),
            KeyCode::Char(c) if self.act_key(c) => {}
            KeyCode::Char('q') => return true,
            KeyCode::Char('g') => self.regroup(),
            KeyCode::Char('R') => self.refresh(),
            KeyCode::Char('j') | KeyCode::Down => self.tree.step(1),
            KeyCode::Char('k') | KeyCode::Up => self.tree.step(-1),
            KeyCode::PageDown => self.tree.step(self.tree.page()),
            KeyCode::PageUp => self.tree.step(-self.tree.page()),
            KeyCode::Home => self.tree.step(isize::MIN / 2),
            KeyCode::End => self.tree.step(isize::MAX / 2),
            KeyCode::Char(' ') => {
                if let Some(node) = self.tree.current() {
                    self.tree.toggle(node);
                }
            }
            KeyCode::Enter => self.primary(),
            _ => {}
        }
        false
    }

    /// The highlighted row's action bound to this key, run; false when it has none.
    fn act_key(&mut self, c: char) -> bool {
        let Some(node) = self.tree.current() else { return false };
        let Some(n) = self.tree.nodes[node].row.acts.iter().position(|a| a.act.key == Some(c)) else { return false };
        self.act(node, n);
        true
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        if self.dialog.is_some() || self.palette.is_open {
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollDown => self.tree.scroll(3),
            MouseEventKind::ScrollUp => self.tree.scroll(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((node, on_glyph)) = self.tree.at(mouse.column, mouse.row) {
                    self.tree.select(node);
                    let row = &self.tree.nodes[node].row;
                    if on_glyph || (row.primary.is_none() && !self.tree.nodes[node].kids.is_empty()) {
                        self.tree.toggle(node);
                    }
                    return;
                }
                let pressed = self.buttons.iter().find(|(r, _)| r.contains((mouse.column, mouse.row).into())).map(|(_, n)| *n);
                if let (Some(n), Some(node)) = (pressed, self.tree.current()) {
                    self.act(node, n);
                }
            }
            _ => {}
        }
    }
}

impl term::App for App {
    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::new().style(base()), area);
        let head_height = panel::height(&self.head, area.width.saturating_sub(2)).max(1);
        let (info, acts) = match self.tree.current() {
            Some(node) => {
                let row = &self.tree.nodes[node].row;
                (row.details(), row.acts.iter().map(|a| a.act.clone()).collect())
            }
            None => (Vec::new(), Vec::new()),
        };
        let panel_height = panel::panel_height(&info, &acts, area.width);
        let [head, tree, bottom, footer] =
            Layout::vertical([Constraint::Length(head_height), Constraint::Min(1), Constraint::Length(panel_height), Constraint::Length(1)]).areas(area);
        panel::header(frame, head, &self.head);
        self.tree.render(frame, panel::inset(tree), &self.requests);
        self.buttons = panel::panel(frame, bottom, &info, &acts);
        panel::footer(frame, footer);
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
        quit
    }

    fn tick(&mut self) {
        self.pump();
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(if self.in_flight > 0 { 100 } else { 250 })
    }
}
