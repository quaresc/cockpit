//! The show app: the view's blocks one after the other in a scrolling body under a header, redrawn when the spec file
//! changes; a new call fades its blocks in one after the other, a refresh of the same call redraws in place.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime};

use chrono::TimeZone;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use notify::RecommendedWatcher;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui_image::picker::Picker;
use ratatui_image::sliced::{SignedPosition, SlicedImage};
use serde_json::{Value, json};
use tachyonfx::{Effect, Interpolation, fx};

use cockpit_core::bridge::Bridge;
use cockpit_core::palette::{Entry, Outcome, Palette};
use cockpit_core::theme::{BG, DIM, FG, LINE, ORANGE, PANEL, base, bold, dim, fg};
use cockpit_core::toast::{Kind, Toasts};
use cockpit_core::{files, term};

use crate::blocks::{Ctx, Msg, View};
use crate::question::Key;
use crate::row::{Act, Row, Target};

/// herdr-reviewr, when installed, takes over a repo diff for line comments sent back to the agent.
const REVIEWR: &str = "persiyanov.reviewr";

enum Command {
    GoTo(usize),
    Copy(usize),
    Page,
}

pub struct ShowApp {
    name: String,
    dir: PathBuf,
    spec_path: PathBuf,
    spec: Option<Value>,
    stamp: Option<(Option<SystemTime>, u64)>,
    loaded: bool,
    shown_at: Option<Value>,
    views: Vec<View>,
    next_uid: u64,
    ctx: Ctx,
    results: Receiver<Msg>,
    _watcher: Option<RecommendedWatcher>,
    changes: Receiver<()>,
    rows: Vec<Row>,
    starts: Vec<usize>,
    width: usize,
    dirty: bool,
    scroll: usize,
    body: Rect,
    focus: Option<(usize, Target)>,
    reveal: bool,
    fades: Vec<Option<Effect>>,
    last_frame: Instant,
    palette: Palette,
    commands: Vec<Command>,
    toasts: Toasts,
    bridge: Bridge,
}

fn same_place(a: &Target, b: &Target) -> bool {
    matches!((a, b), (Target::Button(_), Target::Button(_))) || a == b
}

fn has_reviewr() -> bool {
    files::load(&files::home().join(".config/herdr/plugins.json"))
        .and_then(|p| p.as_array().map(|a| a.iter().any(|p| p["plugin_id"] == REVIEWR)))
        .unwrap_or(false)
}

impl ShowApp {
    pub fn new(name: String, dir: PathBuf, picker: Picker) -> Self {
        let (tx, results) = channel();
        let (watcher, changes) = files::watch(std::slice::from_ref(&dir));
        let mut app = Self {
            spec_path: dir.join(format!("{name}.json")),
            name,
            dir,
            spec: None,
            stamp: None,
            loaded: false,
            shown_at: None,
            views: Vec::new(),
            next_uid: 0,
            ctx: Ctx { tx, picker, cwd: files::home() },
            results,
            _watcher: watcher,
            changes,
            rows: Vec::new(),
            starts: Vec::new(),
            width: 0,
            dirty: true,
            scroll: 0,
            body: Rect::default(),
            focus: None,
            reveal: false,
            fades: Vec::new(),
            last_frame: Instant::now(),
            palette: Palette::default(),
            commands: Vec::new(),
            toasts: Toasts::default(),
            bridge: Bridge::default(),
        };
        app.reload();
        app
    }

    /// Reads the spec again when the file changed. A new call (its `at` changed) builds every block afresh and fades
    /// them in; a refresh keeps the blocks whose spec did not change, and the scroll.
    fn reload(&mut self) {
        let stamp = std::fs::metadata(&self.spec_path).ok().map(|m| (m.modified().ok(), m.len()));
        if self.loaded && stamp == self.stamp {
            return;
        }
        (self.loaded, self.stamp) = (true, stamp);
        let spec = files::load(&self.spec_path);
        let at = spec.as_ref().map(|s| s["at"].clone());
        let is_new = at != self.shown_at;
        self.shown_at = at;
        let blocks: Vec<Value> = spec.as_ref().and_then(|s| s["blocks"].as_array().cloned()).unwrap_or_default();
        let mut old: Vec<Option<View>> = std::mem::take(&mut self.views).into_iter().map(Some).collect();
        for (n, block) in blocks.into_iter().enumerate() {
            let kept = if is_new { None } else { old.get_mut(n).filter(|v| v.as_ref().is_some_and(|v| v.spec == block)).and_then(Option::take) };
            let view = kept.unwrap_or_else(|| {
                self.next_uid += 1;
                View::new(block, self.next_uid)
            });
            self.views.push(view);
        }
        self.ctx.cwd = spec.as_ref().and_then(|s| s["cwd"].as_str()).map_or_else(files::home, PathBuf::from);
        if is_new {
            self.scroll = 0;
            self.focus = None;
            self.fades = (0..self.views.len())
                .map(|n| Some(fx::prolong_start(n as u32 * 60, fx::fade_from(BG, BG, (400, Interpolation::CubicOut)))))
                .collect();
            self.last_frame = Instant::now();
        } else {
            self.fades.resize_with(self.views.len(), || None);
            self.focus = self.focus.take().filter(|(n, t)| self.views.get(*n).is_some_and(|v| v.targets().contains(t)));
        }
        self.spec = spec;
        self.dirty = true;
    }

    /// Takes in what came back from the background: block results, spec changes, bridge replies.
    fn pump(&mut self) {
        while let Ok(msg) = self.results.try_recv() {
            if let Some(view) = self.views.iter_mut().find(|v| v.uid == msg.uid) {
                self.dirty |= view.arrived(msg.payload);
            }
        }
        if self.changes.try_iter().count() > 0 {
            self.reload();
        }
        while let Ok(reply) = self.bridge.replies.try_recv() {
            match reply.error {
                Some(error) => self.toasts.push(error, Kind::Bad),
                None => self.toasts.push(reply.note, Kind::Note),
            }
        }
    }

    fn layout(&mut self) {
        self.rows.clear();
        self.starts.clear();
        for (n, view) in self.views.iter_mut().enumerate() {
            self.starts.push(self.rows.len());
            let mut rows = vec![Row::new()];
            if let Some(title) = view.spec["title"].as_str().filter(|t| !t.is_empty()) {
                rows.push(Row::text(title.to_string(), bold(FG)));
            }
            if let Some(error) = view.spec["error"].as_str().filter(|e| !e.is_empty()) {
                rows.push(Row::text(error.to_string(), fg(cockpit_core::theme::RED)));
            }
            let focus = self.focus.as_ref().filter(|(b, _)| *b == n).map(|(_, t)| t);
            rows.extend(view.rows(self.width, focus, &self.ctx));
            for mut row in rows {
                row.block = n;
                self.rows.push(row);
            }
        }
        self.starts.push(self.rows.len());
    }

    fn height(&self) -> usize {
        self.body.height as usize
    }

    fn scroll_by(&mut self, rows: isize) {
        self.scroll = self.scroll.saturating_add_signed(rows).min(self.rows.len().saturating_sub(self.height()));
    }

    /// Scrolls the focused place into view.
    fn show_focus(&mut self) {
        let Some((n, target)) = &self.focus else { return };
        let mine: Vec<usize> = (0..self.rows.len()).filter(|&i| self.rows[i].block == *n && self.rows[i].target.as_ref().is_some_and(|t| same_place(t, target))).collect();
        let (Some(&first), Some(&last)) = (mine.first(), mine.last()) else { return };
        if last >= self.scroll + self.height() {
            self.scroll = (last + 1).saturating_sub(self.height());
        }
        self.scroll = self.scroll.min(first);
    }

    fn move_focus(&mut self, forward: bool) {
        let all: Vec<(usize, Target)> = self.views.iter().enumerate().flat_map(|(n, v)| v.targets().into_iter().map(move |t| (n, t))).collect();
        if all.is_empty() {
            return;
        }
        let at = self.focus.as_ref().and_then(|f| all.iter().position(|a| a == f));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => all.len() - 1,
            (Some(i), true) => (i + 1) % all.len(),
            (Some(i), false) => (i + all.len() - 1) % all.len(),
        };
        self.focus = Some(all[next].clone());
        self.reveal = true;
    }

    fn answer(&mut self, n: usize, value: Value, label: String) {
        let view = &self.views[n];
        let at = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        let answer = json!({"block": n, "id": view.spec.get("id").cloned().unwrap_or(Value::Null), "type": view.kind(), "value": value, "label": label, "at": at});
        if let Err(err) = files::write(&self.dir.join(format!("{}.{n}.answer", self.name)), &answer) {
            self.toasts.push(format!("the answer was not written: {err}"), Kind::Bad);
        }
        self.focus = None;
    }

    fn handle(&mut self, n: usize, key: Key) -> bool {
        match key {
            Key::Unused => false,
            Key::Used => true,
            Key::Answer(value, label) => {
                self.answer(n, value, label);
                true
            }
        }
    }

    fn browser(&mut self) {
        if let Some(url) = self.spec.as_ref().and_then(|s| s["url"].as_str()).map(str::to_string) {
            files::open_url(&url);
            self.toasts.push(format!("opened {url}"), Kind::Note);
        }
    }

    fn reviewr(&mut self) {
        match self.views.iter().find_map(|v| v.repo().map(str::to_string)) {
            None => self.toasts.push("no repo diff in this view", Kind::Note),
            Some(_) if !has_reviewr() => self.toasts.push("herdr-reviewr is not installed", Kind::Note),
            Some(repo) => self.bridge.call("r", "show", "open_reviewr", vec![json!(repo)], None),
        }
    }

    fn open_palette(&mut self) {
        self.commands.clear();
        let mut entries = Vec::new();
        let mut add = |commands: &mut Vec<Command>, command, text: String, help: &str| {
            entries.push(Entry { text, help: help.into(), featured: true, id: commands.len() });
            commands.push(command);
        };
        for (n, view) in self.views.iter().enumerate() {
            add(&mut self.commands, Command::GoTo(n), format!("Go to: {}", view.title()), "scroll the view to this block");
            if view.table().is_some() {
                add(&mut self.commands, Command::Copy(n), format!("Copy as CSV: {}", view.title()), "the table's rows to the clipboard");
            }
        }
        if let Some(url) = self.spec.as_ref().and_then(|s| s["url"].as_str()) {
            add(&mut self.commands, Command::Page, "Open the view's page in the browser".into(), url);
        }
        self.palette.open(entries);
    }

    fn run_command(&mut self, id: usize) {
        match self.commands.get(id) {
            Some(Command::GoTo(n)) => self.scroll = self.starts.get(*n).map_or(0, |s| s + 1),
            Some(Command::Copy(n)) => {
                let (title, csv) = (self.views[*n].title(), self.views[*n].table().map(|t| t.csv()).unwrap_or_default());
                if files::copy(&csv) {
                    self.toasts.push(format!("copied {title} as CSV"), Kind::Good);
                } else {
                    self.toasts.push("no clipboard tool answered", Kind::Bad);
                }
            }
            Some(Command::Page) => self.browser(),
            None => {}
        }
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        if self.palette.is_open {
            if let Outcome::Chosen(id) = self.palette.key(key) {
                self.run_command(id);
            }
            return false;
        }
        if let Some((n, target)) = self.focus.clone() {
            let used = self.views[n].key(&target, key);
            if self.handle(n, used) {
                return false;
            }
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.height().saturating_sub(1).max(1) as isize;
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('c') if ctrl => return true,
            KeyCode::Char('p') if ctrl => self.open_palette(),
            KeyCode::Char('j') | KeyCode::Down => self.scroll_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_by(-1),
            KeyCode::Char(' ') | KeyCode::Char('f') | KeyCode::PageDown => self.scroll_by(page),
            KeyCode::Char('b') | KeyCode::PageUp => self.scroll_by(-page),
            KeyCode::Char('d') => self.scroll_by(page / 2),
            KeyCode::Char('u') => self.scroll_by(-page / 2),
            KeyCode::Char('g') | KeyCode::Home => self.scroll = 0,
            KeyCode::Char('G') | KeyCode::End => self.scroll = usize::MAX,
            KeyCode::Tab => self.move_focus(true),
            KeyCode::BackTab => self.move_focus(false),
            KeyCode::Esc => self.focus = None,
            KeyCode::Char('o') => self.browser(),
            KeyCode::Char('r') => self.reviewr(),
            _ => {}
        }
        false
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::Down(MouseButton::Left) if !self.palette.is_open => {
                let (x, y) = (mouse.column, mouse.row);
                if y < self.body.y || y >= self.body.bottom() || x <= self.body.x {
                    return;
                }
                let i = self.scroll + (y - self.body.y) as usize;
                let Some(row) = self.rows.get(i) else { return };
                let (n, act) = (row.block, row.act_at((x - self.body.x - 1) as usize).cloned());
                if let Some(target) = row.target.clone() {
                    self.focus = Some((n, target));
                }
                match act {
                    Some(Act::Open(url)) => files::open_url(&url),
                    Some(act) => {
                        let used = self.views[n].act(&act);
                        self.handle(n, used);
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    fn header(&self) -> Line<'static> {
        let Some(spec) = &self.spec else { return Line::styled(format!("waiting for the view {}…", self.name), dim()) };
        let at = chrono::Local.timestamp_millis_opt(spec["at"].as_i64().unwrap_or(0)).single().map(|t| t.format("%H:%M:%S").to_string()).unwrap_or_default();
        let title = spec["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(&self.name).to_string();
        Line::from(vec![Span::styled("◆  ", fg(ORANGE)), Span::styled(title, bold(ORANGE)), Span::styled(format!("   {} · {at}", self.name), dim())])
    }

    fn footer(&self) -> Line<'static> {
        let url = self.spec.as_ref().is_some_and(|s| s["url"].is_string());
        let mut keys = vec![("q", "hide")];
        if url {
            keys.push(("o", "browser"));
        }
        if self.views.iter().any(|v| v.repo().is_some()) {
            keys.push(("r", "reviewr"));
        }
        keys.extend([("^p", "palette"), ("tab", "focus")]);
        match self.focus.as_ref().map(|(_, t)| t) {
            Some(Target::Table) => keys.extend([("←→", "scroll"), ("↑↓", "rows"), ("1-9", "sort"), ("esc", "leave")]),
            Some(Target::File(_)) => keys.extend([("enter", "fold"), ("esc", "leave")]),
            Some(Target::Options) => keys.extend([("↑↓", "move"), ("enter", "pick"), ("esc", "leave")]),
            Some(Target::Input) => keys.extend([("enter", "send"), ("esc", "leave")]),
            Some(Target::Button(_)) => keys.extend([("enter", "press"), ("esc", "leave")]),
            None => keys.extend([("j/k", "scroll"), ("space/b", "page"), ("g/G", "top/bottom")]),
        }
        let spans: Vec<Span> = keys
            .into_iter()
            .flat_map(|(k, what)| [Span::styled(format!(" {k}"), bold(ORANGE).bg(PANEL)), Span::styled(format!(" {what} "), fg(FG).bg(PANEL))])
            .collect();
        Line::from(spans)
    }

    fn draw_body(&mut self, frame: &mut Frame) {
        let body = self.body;
        let inner = Rect { x: body.x + 1, width: self.width as u16, ..body };
        let shown = self.rows.len().saturating_sub(self.scroll).min(self.height());
        for k in 0..shown {
            let row = &self.rows[self.scroll + k];
            frame.render_widget(Paragraph::new(row.line.clone()), Rect { y: body.y + k as u16, height: 1, ..inner });
        }
        let total = self.rows.len();
        if total > self.height() && body.width > 0 {
            let h = self.height();
            let (len, start) = ((h * h / total).max(1), self.scroll * h / total);
            for k in 0..h {
                let thumb = k >= start && k < start + len;
                let cell = Span::styled(if thumb { "┃" } else { "│" }, fg(if thumb { DIM } else { LINE }));
                frame.render_widget(Paragraph::new(Line::from(cell)), Rect { x: body.right() - 1, y: body.y + k as u16, width: 1, height: 1 });
            }
        }
        let now = Instant::now();
        let elapsed = now - self.last_frame;
        self.last_frame = now;
        let height = self.height();
        for n in 0..self.views.len() {
            let Some(effect) = self.fades[n].as_mut() else { continue };
            let (a, b) = (self.starts[n].max(self.scroll), self.starts[n + 1].min(self.scroll + height));
            let area = Rect { y: body.y + a.saturating_sub(self.scroll) as u16, height: b.saturating_sub(a) as u16, ..body };
            effect.process(elapsed.into(), frame.buffer_mut(), area);
            if effect.done() {
                self.fades[n] = None;
            }
        }
        for (n, view) in self.views.iter().enumerate() {
            let (Some(image), None) = (view.image(), &self.fades[n]) else { continue };
            let Some(top) = (self.starts[n]..self.starts[n + 1]).find(|&i| self.rows[i].image == Some(0)) else { continue };
            let y = top as i64 - self.scroll as i64;
            if y + i64::from(image.size().height) <= 0 || y >= self.height() as i64 {
                continue;
            }
            frame.render_widget(SlicedImage::new(image, SignedPosition { x: 0, y: y as i16 }), inner);
        }
    }
}

impl term::App for ShowApp {
    fn draw(&mut self, frame: &mut Frame) {
        self.pump();
        let area = frame.area();
        frame.render_widget(Block::new().style(base()), area);
        let [head, body, foot] = Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(area);
        frame.render_widget(Paragraph::new(self.header()), Rect { x: head.x + 1, width: head.width.saturating_sub(2), ..head });
        self.body = body;
        let width = body.width.saturating_sub(2) as usize;
        if self.dirty || width != self.width {
            self.width = width;
            self.layout();
            self.dirty = false;
        }
        if std::mem::take(&mut self.reveal) {
            self.show_focus();
        }
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(self.height()));
        self.draw_body(frame);
        frame.render_widget(Paragraph::new(self.footer()).style(Style::new().bg(PANEL)), foot);
        self.toasts.render(frame, Rect { height: area.height.saturating_sub(1), ..area });
        if self.palette.is_open {
            self.palette.render(frame, area);
        }
    }

    fn event(&mut self, event: Event) -> bool {
        self.dirty = true;
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.key(key),
            Event::Mouse(mouse) => {
                self.mouse(mouse);
                false
            }
            _ => false,
        }
    }

    fn tick(&mut self) {
        self.pump();
        self.reload();
        for view in &mut self.views {
            view.tick(&self.ctx);
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(if self.fades.iter().any(Option::is_some) { 16 } else { 250 })
    }
}
