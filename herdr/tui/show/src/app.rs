//! The show app: the view's blocks one after the other in a scrolling body under a header, redrawn when the spec file
//! changes; a new call fades its new blocks in one after the other, a refresh of the same call redraws in place and its
//! charts glide to their new values. Idle, it draws nothing.

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
use ratatui_image::FontSize;
use ratatui_image::picker::Picker;
use ratatui_image::sliced::{SignedPosition, SlicedImage};
use serde_json::{Value, json};
use tachyonfx::{Effect, Interpolation, fx};

use cockpit_core::bridge::Bridge;
use cockpit_core::files::Density;
use cockpit_core::palette::{Entry, Outcome, Palette};
use cockpit_core::theme::{BG, DIM, FG, LINE, ORANGE, PANEL, base, bold, dim, fg};
use cockpit_core::toast::{Kind, Toasts};
use cockpit_core::{files, focus, hub, term};

use crate::answers::Outbox;
use crate::blocks::{Ctx, Msg, View};
use crate::picture::FRAME;
use crate::question::Key;
use crate::row::{Act, Row, Target};

/// herdr-reviewr, when installed, takes over a repo diff for line comments sent back to the agent.
const REVIEWR: &str = "persiyanov.reviewr";
/// The longest wait between ticks: background results, file changes and bridge replies wake the loop sooner.
const IDLE: Duration = Duration::from_millis(250);

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
    font: FontSize,
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
    /// A block to scroll to once the rows are laid out: a palette jump from another app.
    go_to: Option<usize>,
    focus_seen: SystemTime,
    fades: Vec<Option<Effect>>,
    last_frame: Instant,
    palette: Palette,
    commands: Vec<Command>,
    toasts: Toasts,
    toasting: bool,
    bridge: Bridge,
    outbox: Outbox,
    /// The block whose picture the mouse is over, and the one whose split it drags.
    hovered: Option<usize>,
    dragging: Option<usize>,
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
        let font = picker.font_size();
        let compact = files::density() == Density::Compact;
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
            ctx: Ctx { tx, picker, cwd: files::home(), compact, running: Default::default() },
            font,
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
            go_to: None,
            focus_seen: SystemTime::now(),
            fades: Vec::new(),
            last_frame: Instant::now(),
            palette: Palette::default(),
            commands: Vec::new(),
            toasts: Toasts::default(),
            toasting: false,
            bridge: Bridge::default(),
            outbox: Outbox::default(),
            hovered: None,
            dragging: None,
        };
        app.palette.app = "show".into();
        app.reload();
        app
    }

    /// The density, for tests that draw both.
    pub fn with_density(mut self, density: Density) -> Self {
        self.ctx.compact = density == Density::Compact;
        self.dirty = true;
        self
    }

    /// Reads the spec again when the file changed; true when it did. A new call (its `at` changed) builds every block
    /// afresh and fades in those that are new; a refresh keeps the blocks whose spec did not change, and the scroll. A
    /// rebuilt block keeps what the one before it at that place showed (a chart's image, a log's output) until it
    /// draws its own.
    fn reload(&mut self) -> bool {
        let stamp = std::fs::metadata(&self.spec_path).ok().map(|m| (m.modified().ok(), m.len()));
        if self.loaded && stamp == self.stamp {
            return false;
        }
        (self.loaded, self.stamp) = (true, stamp);
        let spec = files::load(&self.spec_path);
        let at = spec.as_ref().map(|s| s["at"].clone());
        let is_new = at != self.shown_at;
        self.shown_at = at;
        let blocks: Vec<Value> = spec.as_ref().and_then(|s| s["blocks"].as_array().cloned()).unwrap_or_default();
        let mut old: Vec<Option<View>> = std::mem::take(&mut self.views).into_iter().map(Some).collect();
        let mut fresh = Vec::new();
        for (n, block) in blocks.into_iter().enumerate() {
            let before = old.get_mut(n).and_then(Option::take);
            let view = match before {
                Some(view) if !is_new && view.spec == block => view,
                before => {
                    self.next_uid += 1;
                    let mut view = View::new(block, self.next_uid);
                    // A new call that redraws the same block in the same place updates it rather than fading it in.
                    let same = before.as_ref().is_some_and(|b| b.spec["type"] == view.spec["type"] && b.spec["title"] == view.spec["title"] && b.spec["kind"] == view.spec["kind"]);
                    fresh.push(is_new && !same);
                    if let Some(before) = before.filter(|_| !is_new || same) {
                        view.inherit(before);
                    }
                    self.views.push(view);
                    continue;
                }
            };
            fresh.push(false);
            self.views.push(view);
        }
        self.ctx.cwd = spec.as_ref().and_then(|s| s["cwd"].as_str()).map_or_else(files::home, PathBuf::from);
        if is_new {
            self.scroll = 0;
            self.focus = None;
            let mut k = 0;
            self.fades = fresh
                .iter()
                .map(|&fresh| {
                    k += u32::from(fresh);
                    fresh.then(|| fx::prolong_start((k - 1) * 60, fx::fade_from(BG, BG, (400, Interpolation::CubicOut))))
                })
                .collect();
            self.last_frame = Instant::now();
        } else {
            self.fades.resize_with(self.views.len(), || None);
            self.focus = self.focus.take().filter(|(n, t)| self.views.get(*n).is_some_and(|v| v.targets().contains(t)));
        }
        self.hovered = self.hovered.filter(|n| *n < self.views.len());
        self.spec = spec;
        self.dirty = true;
        true
    }

    /// Takes in what came back from the background: block results, spec changes, bridge replies; true when the
    /// screen changed.
    fn pump(&mut self) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.results.try_recv() {
            self.ctx.running.set(self.ctx.running.get().saturating_sub(1));
            if let Some(view) = self.views.iter_mut().find(|v| v.uid == msg.uid)
                && view.arrived(msg.payload)
            {
                (self.dirty, changed) = (true, true);
            }
        }
        if self.changes.try_iter().count() > 0 {
            changed |= self.reload();
        }
        while let Ok(reply) = self.bridge.replies.try_recv() {
            match reply.error {
                Some(error) => self.toasts.push(error, Kind::Bad),
                None => self.toasts.push(reply.note, Kind::Note),
            }
            changed = true;
        }
        for error in self.outbox.flush() {
            self.toasts.push(error, Kind::Bad);
            changed = true;
        }
        changed
    }

    fn layout(&mut self) {
        self.rows.clear();
        self.starts.clear();
        let compact = self.ctx.compact;
        for (n, view) in self.views.iter_mut().enumerate() {
            self.starts.push(self.rows.len());
            let mut rows = Vec::new();
            let title = view.spec["title"].as_str().filter(|t| !t.is_empty()).map(str::to_string);
            // A block the session refreshes from a source says when it last did.
            let refreshed = view.spec["refreshed_at"]
                .as_f64()
                .and_then(|ms| chrono::Local.timestamp_millis_opt(ms as i64).single())
                .map(|at| format!("refreshed {}", at.format("%H:%M")));
            if compact {
                // The title inline in a rule, in place of a blank line and a title row.
                if let Some(title) = title {
                    let mut row = Row::text("── ", fg(LINE)).add(title, bold(FG));
                    if let Some(refreshed) = refreshed {
                        row = row.add(format!("  {refreshed}"), dim());
                    }
                    let used = row.line.width();
                    rows.push(row.add(format!(" {}", "─".repeat(self.width.saturating_sub(used + 1))), fg(LINE)));
                }
            } else {
                rows.push(Row::new());
                if let Some(title) = title {
                    rows.push(Row::text(title, bold(FG)).add(refreshed.map(|r| format!("   {r}")).unwrap_or_default(), dim()));
                }
            }
            if let Some(error) = view.spec["error"].as_str().filter(|e| !e.is_empty()) {
                rows.extend(crate::blocks::error_rows(error, self.width));
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

    /// Scrolls the focused place into view: the block's cursor row when it has one, else all of its focus stop.
    fn show_focus(&mut self) {
        let Some((n, target)) = &self.focus else { return };
        let in_block = |i: &usize| self.rows[*i].block == *n;
        let here: Vec<usize> = (0..self.rows.len()).filter(|i| in_block(i) && self.rows[*i].here).collect();
        let mine: Vec<usize> = if here.is_empty() { (0..self.rows.len()).filter(|i| in_block(i) && self.rows[*i].target.as_ref() == Some(target)).collect() } else { here };
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

    /// An answer for the mod to hand to the session; a question's closes it, a comment keeps the diff focused.
    fn answer(&mut self, n: usize, kind: &str, value: Value, label: String) {
        let at = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        let mut answer = json!({"block": n, "type": kind, "value": value, "label": label, "at": at});
        if kind != "comment" {
            answer["id"] = self.views[n].spec.get("id").cloned().unwrap_or(Value::Null);
            self.focus = None;
        }
        self.outbox.send(self.dir.join(format!("{}.{n}.answer", self.name)), answer);
        for error in self.outbox.flush() {
            self.toasts.push(error, Kind::Bad);
        }
    }

    /// What a block made of a key or a click: true when it used it.
    fn handle(&mut self, n: usize, key: Key) -> bool {
        match key {
            Key::Unused => return false,
            Key::Used => {}
            Key::Answer(value, label) => {
                let kind = self.views[n].kind().to_string();
                self.answer(n, &kind, value, label);
            }
            Key::Comment(value, label) => {
                self.answer(n, "comment", value, label.clone());
                self.toasts.push(format!("comment on {label} sent to the session"), Kind::Good);
            }
        }
        self.reveal = true;
        true
    }

    /// A request from the palette of another app: `name` or `name#block`, the block by its index, id or title.
    fn focus_request(&mut self, target: &Value) -> bool {
        let Some(key) = target["key"].as_str().or(target.as_str()) else { return false };
        let (name, block) = key.split_once('#').map_or((key, None), |(name, block)| (name, Some(block)));
        if name != self.name {
            return false;
        }
        let n = block
            .and_then(|b| b.parse::<usize>().ok().or_else(|| self.views.iter().position(|v| v.spec["id"].as_str() == Some(b) || v.title() == b)))
            .unwrap_or(0)
            .min(self.views.len().saturating_sub(1));
        self.focus = self.views.get(n).and_then(|v| v.targets().into_iter().next()).map(|t| (n, t));
        (self.go_to, self.dirty) = (Some(n), true);
        true
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
            Some(Command::GoTo(n)) => self.go_to = Some(*n),
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
        self.dirty = true;
        if self.palette.is_open {
            match self.palette.key(key) {
                Outcome::Chosen(id) => self.run_command(id),
                Outcome::Elsewhere(target) => hub::go(&target),
                Outcome::Open | Outcome::Closed => {}
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

    /// The row under a screen cell, and the column in it.
    fn under(&self, x: u16, y: u16) -> Option<(usize, usize)> {
        if y < self.body.y || y >= self.body.bottom() || x <= self.body.x {
            return None;
        }
        let i = self.scroll + (y - self.body.y) as usize;
        (i < self.rows.len()).then(|| (i, (x - self.body.x - 1) as usize))
    }

    /// The mouse moved: the picture under it hears where, the one it left that it left; true when either changed.
    fn hover(&mut self, x: u16, y: u16) -> bool {
        let over = self.under(x, y).and_then(|(i, col)| self.rows[i].image.map(|k| (self.rows[i].block, col, k)));
        let mut changed = false;
        if let Some(left) = self.hovered.filter(|n| over.is_none_or(|(b, _, _)| b != *n)) {
            changed |= self.views[left].hover(None, self.font);
        }
        self.hovered = over.map(|(n, _, _)| n);
        if let Some((n, col, k)) = over {
            changed |= self.views[n].hover(Some((col, k)), self.font);
        }
        changed
    }

    /// Returns true when the mouse changed what the view shows.
    fn mouse(&mut self, mouse: MouseEvent) -> bool {
        let (x, y) = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::Moved => return self.hover(x, y),
            MouseEventKind::Drag(MouseButton::Left) => {
                let col = x.saturating_sub(self.body.x + 1) as usize;
                return self.dragging.is_some_and(|n| self.views[n].drag(col));
            }
            MouseEventKind::Up(MouseButton::Left) => self.dragging = None,
            MouseEventKind::Down(MouseButton::Left) if !self.palette.is_open => {
                let Some((i, col)) = self.under(x, y) else { return false };
                let row = &self.rows[i];
                let (n, act) = (row.block, row.act_at(col).cloned());
                if let Some(target) = row.target.clone() {
                    self.focus = Some((n, target));
                }
                if row.image.is_some() && self.views[n].drag(col) {
                    self.dragging = Some(n);
                    self.focus = self.views[n].targets().into_iter().next().map(|t| (n, t));
                }
                match act {
                    Some(Act::Open(url)) => files::open_url(&url),
                    Some(act) => {
                        let used = self.views[n].act(&act);
                        self.handle(n, used);
                    }
                    None => {}
                }
                self.dirty = true;
            }
            _ => return false,
        }
        true
    }

    fn header(&self) -> Line<'static> {
        let Some(spec) = &self.spec else { return Line::styled(format!("waiting for the view {}…", self.name), dim()) };
        let at = chrono::Local.timestamp_millis_opt(spec["at"].as_f64().unwrap_or(0.0) as i64).single().map(|t| t.format("%H:%M:%S").to_string()).unwrap_or_default();
        let title = spec["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(&self.name).to_string();
        Line::from(vec![Span::styled("◆  ", fg(ORANGE)), Span::styled(title, bold(ORANGE)), Span::styled(format!("   {} · {at}", self.name), dim())])
    }

    fn footer(&self) -> Line<'static> {
        let url = self.spec.as_ref().is_some_and(|s| s["url"].is_string());
        let typing = self.focus.as_ref().is_some_and(|(n, _)| self.views[*n].is_typing());
        let mut keys = if typing { vec![] } else { vec![("q", "hide")] };
        if url && !typing {
            keys.push(("o", "browser"));
        }
        if self.views.iter().any(|v| v.repo().is_some()) && !typing {
            keys.push(("r", "reviewr"));
        }
        if !typing {
            keys.extend([("^p", "palette"), ("tab", "focus")]);
        }
        match self.focus.as_ref().map(|(_, t)| t) {
            _ if typing => keys.extend([("enter", "send the comment"), ("esc", "cancel")]),
            Some(Target::Table) => keys.extend([("←→", "scroll"), ("↑↓", "rows"), ("1-9", "sort"), ("esc", "leave")]),
            Some(Target::Diff) => keys.extend([("j/k", "line"), ("[ ]", "file"), ("c", "comment"), ("s", "split"), ("enter", "fold"), ("esc", "leave")]),
            Some(Target::Log) => keys.extend([("j/k", "scroll"), ("g/G", "top/follow"), ("esc", "leave")]),
            Some(Target::Chart) => keys.extend([("←→", "point"), ("+/-", "zoom"), ("0", "all"), ("esc", "leave")]),
            Some(Target::Compare) => keys.extend([("h/l", "split"), ("s", "side by side"), ("esc", "leave")]),
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
        for (n, view) in self.views.iter_mut().enumerate() {
            let top = (self.starts[n]..self.starts[n + 1]).find(|&i| self.rows[i].image == Some(0));
            let (Some(top), Some(image)) = (top, view.image()) else {
                view.set_visible(false);
                continue;
            };
            let y = top as i64 - self.scroll as i64;
            let visible = y + i64::from(image.size().height) > 0 && y < height as i64;
            if visible && self.fades[n].is_none() {
                frame.render_widget(SlicedImage::new(image, SignedPosition { x: 0, y: y as i16 }), inner);
                if let Ok(dir) = std::env::var("COCKPIT_SHOT_DIR") {
                    let size = image.size();
                    let place = serde_json::json!([inner.x, i64::from(inner.y) + y, size.width, size.height]);
                    let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{}.at", crate::picture::shot_key(&view.spec))), place.to_string());
                }
            }
            view.set_visible(visible);
        }
    }

    /// Waits until every background draw, read and run has come back, drawing as a frame would; for tests.
    pub fn settle(&mut self, width: u16, height: u16) {
        let started = Instant::now();
        loop {
            let _ = cockpit_core::testing::render(self, width, height);
            term::App::tick(self);
            let busy = self.ctx.running.get() > 0 || self.views.iter().any(View::pending);
            if !busy || started.elapsed() > Duration::from_secs(20) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.fades.iter_mut().for_each(|f| *f = None);
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
            // A layout can paint code, whose grammars it has just let go.
            crate::raster::give_back();
        }
        if let Some(n) = self.go_to.take() {
            let start = self.starts.get(n).copied().unwrap_or(0);
            self.scroll = (start..self.starts.get(n + 1).copied().unwrap_or(start)).find(|&i| self.rows[i].line.width() > 0).unwrap_or(start);
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
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.key(key),
            Event::Mouse(mouse) => {
                self.dirty |= self.mouse(mouse);
                false
            }
            _ => false,
        }
    }

    fn tick(&mut self) -> bool {
        let mut changed = self.pump();
        changed |= self.reload();
        if let Some(target) = focus::take("show", &mut self.focus_seen) {
            changed |= self.focus_request(&target);
        }
        for view in &mut self.views {
            changed |= view.tick(&self.ctx);
        }
        let toasting = self.toasts.any();
        changed |= toasting != self.toasting || self.fades.iter().any(Option::is_some);
        self.toasting = toasting;
        changed
    }

    fn timeout(&self) -> Duration {
        if self.fades.iter().any(Option::is_some) {
            return Duration::from_millis(16);
        }
        let wait = if self.views.iter().any(View::moving) { FRAME } else { IDLE };
        self.views.iter().filter_map(View::next_due).fold(wait, Duration::min).max(Duration::from_millis(5))
    }
}
