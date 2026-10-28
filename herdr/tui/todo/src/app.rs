//! The app's state and the edits that change the file: each one read back from disk first when another process wrote
//! it (/todo while one was typed), undoable, saved through a temporary file, and flashed on the item it touched.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use cockpit_core::anim::{Effects, Kind};
use cockpit_core::dialog::Dialog;
use cockpit_core::files::{self, Density};
use cockpit_core::input::Input;
use cockpit_core::palette::Palette;
use cockpit_core::toast::{Kind as Toast, Toasts};
use ratatui::layout::Rect;
use ratatui::style::Color;
use regex::Regex;
use serde_json::Value;

use crate::doc::{self, Doc, Item};
use crate::{links, pinned};

const UNDO_DEPTH: usize = 50;
/// Short flashes: a new item in the accent colour, a done one in teal.
pub const ADDED: Kind = Kind::Flash(Color::Rgb(90, 70, 50), 1.2);
pub const DONE: Kind = Kind::Flash(Color::Rgb(40, 100, 80), 1.2);
/// The effect key of the folded "N done" row, flashed when an item done goes under it.
pub const FOLD_FX: &str = "\u{0}fold";

/// What the selection is on, by identity, so it stays on its item when the file is read again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sel {
    Adder,
    Item(String),
    Fold,
}

/// What a palette entry, a button or a confirmed dialog runs.
#[derive(Clone, Debug)]
pub enum Cmd {
    Add,
    Fold,
    Undo,
    Toggle(Vec<String>),
    Edit(String),
    Delete(Vec<String>),
    Move(String, isize),
    Open(String),
    GoTo(String),
    Mark(String),
    ClearMarks,
}

/// The text field open on the adder or on an item.
pub struct Editing {
    pub item: Option<String>,
    pub input: Input,
}

/// Where the last frame drew what a click can land on.
#[derive(Default)]
pub struct Hits {
    pub list: Rect,
    pub rows: Vec<(Rect, Sel)>,
    pub glyphs: Vec<(Rect, String)>,
    pub tokens: Vec<(Rect, String)>,
    pub buttons: Vec<(Rect, Cmd)>,
}

pub struct App {
    pub path: PathBuf,
    pub doc: Doc,
    known: HashSet<String>,
    mtime: Option<SystemTime>,
    undo: Vec<Doc>,
    pub config: Value,
    pub inbox: Value,
    pub tokens: Regex,
    pub density: Density,
    pub sel: Sel,
    pub show_done: bool,
    pub editing: Option<Editing>,
    pub filter: Input,
    pub filtering: bool,
    pub marks: HashSet<String>,
    pub fx: Effects,
    pub animating: bool,
    toasting: bool,
    pub toasts: Toasts,
    pub palette: Palette,
    pub cmds: Vec<Cmd>,
    pub dialog: Option<(Dialog, Cmd)>,
    _watcher: Option<notify::RecommendedWatcher>,
    changes: Receiver<()>,
    polled: Instant,
    focus_seen: SystemTime,
    pinned: bool,
    reported: Option<String>,
    pub hits: Hits,
    /// The list's first line on screen, and whether it follows the selection (not after a wheel scroll).
    pub scroll: usize,
    pub follow: bool,
}

pub fn todo_path() -> PathBuf {
    files::config_dir().join("todo.md")
}

impl App {
    /// The app on the real list, watching its folder.
    pub fn start() -> Self {
        let inbox = files::load(&files::cache().join("inbox.json")).unwrap_or(Value::Null);
        let mut app = Self::with(todo_path(), files::config(), inbox, files::density(), pinned::is_pinned());
        let (watcher, changes) = files::watch(&[files::config_dir()]);
        (app._watcher, app.changes) = (watcher, changes);
        app.report();
        app
    }

    /// The app on this file with this config, for the binary and for tests.
    pub fn with(path: PathBuf, config: Value, inbox: Value, density: Density, pinned: bool) -> Self {
        let tokens = links::token_pattern(&links::team_keys(&config, &inbox));
        let mut palette = Palette::default();
        palette.app = "todo".into();
        let mut app = Self {
            path,
            doc: Doc::default(),
            known: HashSet::new(),
            mtime: None,
            undo: Vec::new(),
            config,
            inbox,
            tokens,
            density,
            sel: Sel::Adder,
            show_done: false,
            editing: None,
            filter: Input::default(),
            filtering: false,
            marks: HashSet::new(),
            fx: Effects::default(),
            animating: false,
            toasting: false,
            toasts: Toasts::default(),
            palette,
            cmds: Vec::new(),
            dialog: None,
            _watcher: None,
            changes: std::sync::mpsc::channel().1,
            polled: Instant::now(),
            focus_seen: SystemTime::now(),
            pinned,
            reported: None,
            hits: Hits::default(),
            scroll: 0,
            follow: true,
        };
        app.read();
        app.sel = app.selectable().into_iter().nth(1).unwrap_or(Sel::Adder);
        app
    }

    fn disk_mtime(&self) -> Option<SystemTime> {
        std::fs::metadata(&self.path).and_then(|m| m.modified()).ok()
    }

    fn read(&mut self) {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        self.doc = Doc::parse(&text);
        self.mtime = self.disk_mtime();
        self.known = self.doc.items().into_iter().map(|i| i.text).collect();
    }

    pub fn items(&self) -> Vec<Item> {
        self.doc.items()
    }

    pub fn item(&self, text: &str) -> Option<Item> {
        self.items().into_iter().find(|i| i.text == text)
    }

    pub fn open_count(&self) -> usize {
        self.items().iter().filter(|i| !i.done).count()
    }

    /// Whether an item shows under the filter: every word of it somewhere in the text, in any case.
    pub fn matches(&self, item: &Item) -> bool {
        let text = item.text.to_lowercase();
        self.filter.text.to_lowercase().split_whitespace().all(|word| text.contains(word))
    }

    /// The open items then the done ones, as shown: filtered, the done ones only while unfolded or filtered.
    pub fn shown(&self) -> (Vec<Item>, Vec<Item>) {
        let items = self.items();
        let open = items.iter().filter(|i| !i.done && self.matches(i)).cloned().collect();
        let done = items.iter().filter(|i| i.done && self.matches(i)).cloned().collect();
        (open, done)
    }

    pub fn done_unfolded(&self) -> bool {
        self.show_done || !self.filter.text.is_empty()
    }

    /// What the selection can stop on, top to bottom.
    pub fn selectable(&self) -> Vec<Sel> {
        let (open, done) = self.shown();
        let mut all = vec![Sel::Adder];
        all.extend(open.into_iter().map(|i| Sel::Item(i.text)));
        if !done.is_empty() {
            all.push(Sel::Fold);
            if self.done_unfolded() {
                all.extend(done.into_iter().map(|i| Sel::Item(i.text)));
            }
        }
        all
    }

    pub fn sel_index(&self) -> usize {
        self.selectable().iter().position(|s| *s == self.sel).unwrap_or(0)
    }

    pub fn select_at(&mut self, index: usize) {
        let all = self.selectable();
        self.sel = all.get(index.min(all.len().saturating_sub(1))).cloned().unwrap_or(Sel::Adder);
        self.follow = true;
    }

    pub fn move_sel(&mut self, by: isize) {
        let at = self.sel_index() as isize + by;
        self.select_at(at.max(0) as usize);
    }

    /// The selected item, when the selection is on one.
    pub fn selected_item(&self) -> Option<Item> {
        match &self.sel {
            Sel::Item(text) => self.item(text),
            _ => None,
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Reading again and the pinned space's duties

    /// The file read again when another process wrote it, unless a to-do is being typed; true when it was.
    fn reload(&mut self) -> bool {
        self.polled = Instant::now();
        if self.editing.is_some() || self.disk_mtime() == self.mtime {
            return false;
        }
        let at = self.sel_index();
        self.read();
        // Undoing past another writer's change would drop it.
        self.undo.clear();
        self.keep_selection(at);
        self.report();
        true
    }

    /// The selection back on its item, else on the row now where it was.
    fn keep_selection(&mut self, at: usize) {
        if !self.selectable().contains(&self.sel) {
            self.select_at(at);
        }
    }

    pub fn report(&mut self) {
        let text = pinned::count_text(self.open_count());
        if self.pinned && self.reported.as_ref() != Some(&text) {
            pinned::report(text.clone());
        }
        self.reported = Some(text);
    }

    /// A palette jump from another app: the item with that text selected and in view.
    fn take_focus(&mut self) -> bool {
        let Some(target) = cockpit_core::focus::take("todo", &mut self.focus_seen) else { return false };
        let Some(key) = target["key"].as_str() else { return false };
        let items = self.items();
        let found = items.iter().find(|i| i.text == key).or_else(|| items.iter().find(|i| doc::undated(&i.text) == doc::undated(key)));
        let Some(item) = found.cloned() else {
            self.toasts.push(format!("not on the list any more: {key}"), Toast::Note);
            return true;
        };
        if !self.matches(&item) {
            self.filter = Input::default();
        }
        if item.done {
            self.show_done = true;
        }
        self.sel = Sel::Item(item.text);
        self.follow = true;
        true
    }

    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Reads the file again on the watcher's word (or every few seconds), takes focus requests, steps the flashes.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        if self.changes.try_iter().count() > 0 || self.polled.elapsed() >= Duration::from_secs(3) {
            changed |= self.reload();
        }
        changed |= self.take_focus();
        let toasting = self.toasts.any();
        changed |= toasting || self.toasting;
        self.toasting = toasting;
        let animating = self.fx.active();
        changed |= animating || self.animating;
        self.animating = animating;
        changed
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Edits

    /// Writes the list, keeping the to-dos another process added since it was read (/todo while one was typed).
    fn save(&mut self) {
        if self.disk_mtime() != self.mtime {
            let disk = Doc::parse(&std::fs::read_to_string(&self.path).unwrap_or_default());
            for item in disk.items().into_iter().rev().filter(|i| !self.known.contains(&i.text)) {
                self.doc.add(&item.text);
                if item.done {
                    let line = self.doc.items()[0].line;
                    self.doc.set(line, true, &item.text);
                }
            }
            self.doc.order();
        }
        if let Err(err) = self.write() {
            self.toasts.push(format!("cannot write {}: {err}", self.path.display()), Toast::Bad);
        }
        self.mtime = self.disk_mtime();
        self.known = self.doc.items().into_iter().map(|i| i.text).collect();
        self.report();
    }

    fn write(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        std::fs::write(&tmp, self.doc.text())?;
        std::fs::rename(&tmp, &self.path)
    }

    /// One change to the list: undoable, saved, the selection kept in place.
    fn change(&mut self, edit: impl FnOnce(&mut Doc)) {
        let at = self.sel_index();
        let before = self.doc.clone();
        if self.doc.items().is_empty() && self.doc.text().trim().is_empty() {
            self.doc = Doc::parse(doc::NEW_FILE);
        }
        edit(&mut self.doc);
        if self.doc == before {
            return;
        }
        self.undo.push(before);
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.save();
        self.keep_selection(at);
    }

    pub fn add(&mut self, text: &str) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.change(|doc| doc.add(&text));
        self.flash(&text, ADDED);
        self.sel = Sel::Item(text);
        self.follow = true;
    }

    pub fn rename(&mut self, from: &str, to: &str) {
        let to = to.trim().to_string();
        let Some(item) = self.item(from).filter(|_| !to.is_empty() && to != from) else { return };
        self.change(|doc| doc.set(item.line, item.done, &to));
        if self.sel == Sel::Item(from.to_string()) {
            self.sel = Sel::Item(to);
        }
    }

    /// Items done, or open again; several at once when they are marked.
    pub fn toggle(&mut self, texts: &[String]) {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let was: Vec<Item> = texts.iter().filter_map(|t| self.item(t)).collect();
        if was.is_empty() {
            return;
        }
        let follow_sel = self.selected_item().map(|i| i.text);
        self.change(|doc| {
            // Last first: each one done goes on top of the done ones, so they keep their order.
            for item in was.iter().rev() {
                let line = doc.items().into_iter().find(|i| i.text == item.text && i.done == item.done).map(|i| i.line);
                if let Some(line) = line {
                    doc.toggle(line, &today);
                }
            }
        });
        for item in &was {
            let now = if item.done { doc::undated(&item.text).to_string() } else { format!("{} (done {today})", doc::undated(&item.text)) };
            if !item.done && !self.done_unfolded() {
                self.flash(FOLD_FX, DONE);
            } else {
                self.flash(&now, if item.done { ADDED } else { DONE });
            }
            // An item opened again, or done in view, keeps the selection.
            if follow_sel.as_deref() == Some(item.text.as_str()) && (item.done || self.done_unfolded()) {
                self.sel = Sel::Item(now);
            }
        }
        let n = was.len();
        if n > 1 {
            self.toasts.push(format!("{n} to-dos toggled"), Toast::Good);
        }
        self.marks.clear();
    }

    /// An open item one place up or down among the open ones.
    pub fn shift(&mut self, text: &str, by: isize) {
        let open: Vec<Item> = self.items().into_iter().filter(|i| !i.done).collect();
        let Some(at) = open.iter().position(|i| i.text == text) else { return };
        let target = at as isize + by;
        if target < 0 || target as usize >= open.len() {
            return;
        }
        let (a, b) = (open[at].line, open[target as usize].line);
        self.change(|doc| doc.swap(a, b));
        self.follow = true;
    }

    pub fn delete(&mut self, texts: &[String]) {
        let lines: Vec<usize> = texts.iter().filter_map(|t| self.item(t)).map(|i| i.line).collect();
        if lines.is_empty() {
            return;
        }
        self.change(|doc| {
            let mut lines = lines.clone();
            lines.sort_unstable_by(|a, b| b.cmp(a));
            for line in lines {
                doc.remove(line);
            }
        });
        let n = lines.len();
        self.toasts.push(if n == 1 { "deleted · u undoes".to_string() } else { format!("{n} to-dos deleted · u undoes") }, Toast::Good);
        self.marks.clear();
    }

    pub fn undo(&mut self) {
        let Some(before) = self.undo.pop() else {
            self.toasts.push("nothing to undo", Toast::Note);
            return;
        };
        let at = self.sel_index();
        self.doc = before;
        self.save();
        self.keep_selection(at);
        self.toasts.push("undone", Toast::Good);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn open_link(&mut self, token: &str) {
        match links::url(token, &self.config, &self.inbox) {
            Some(url) => {
                files::open_url(&url);
                self.toasts.push(format!("opened {url}"), Toast::Good);
            }
            None => self.toasts.push(format!("no URL known for {}: set tickets.urlTemplate in the cockpit config", token.to_uppercase()), Toast::Note),
        }
    }

    pub fn flash(&mut self, key: &str, kind: Kind) {
        self.fx.start(key, kind);
        self.animating = true;
    }

    /// Opens the text field on the adder, or on an item with its text.
    pub fn start_edit(&mut self, item: Option<String>) {
        let input = Input::new(item.as_deref().unwrap_or_default());
        self.sel = item.clone().map_or(Sel::Adder, Sel::Item);
        self.follow = true;
        self.editing = Some(Editing { item, input });
    }

    pub fn commit_edit(&mut self) {
        let Some(editing) = self.editing.take() else { return };
        match editing.item {
            Some(from) => self.rename(&from, &editing.input.text),
            None => self.add(&editing.input.text),
        }
    }

    /// The marked items still on the list, in list order, or the selected one alone.
    pub fn targets(&self) -> Vec<String> {
        if self.marks.is_empty() {
            return self.selected_item().map(|i| vec![i.text]).unwrap_or_default();
        }
        self.items().into_iter().filter(|i| self.marks.contains(&i.text)).map(|i| i.text).collect()
    }

    pub fn toggle_mark(&mut self, text: &str) {
        if !self.marks.remove(text) {
            self.marks.insert(text.to_string());
        }
    }

    /// Runs a command; deleting asks first.
    pub fn run(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Add => self.start_edit(None),
            Cmd::Fold => self.show_done = !self.show_done,
            Cmd::Undo => self.undo(),
            Cmd::Toggle(texts) => self.toggle(&texts),
            Cmd::Edit(text) => self.start_edit(Some(text)),
            Cmd::Delete(texts) if !texts.is_empty() => {
                let question = match texts.as_slice() {
                    [one] => format!("Delete \u{201c}{one}\u{201d}?"),
                    many => format!("Delete these {} to-dos?", many.len()),
                };
                self.dialog = Some((Dialog::Confirm { question }, Cmd::Delete(texts)));
            }
            Cmd::Delete(_) => {}
            Cmd::Move(text, by) => self.shift(&text, by),
            Cmd::Open(token) => self.open_link(&token),
            Cmd::GoTo(text) => {
                if let Some(item) = self.item(&text) {
                    if !self.matches(&item) {
                        self.filter = Input::default();
                    }
                    self.show_done |= item.done;
                    self.sel = Sel::Item(text);
                    self.follow = true;
                }
            }
            Cmd::Mark(text) => {
                self.toggle_mark(&text);
                self.move_sel(1);
            }
            Cmd::ClearMarks => self.marks.clear(),
        }
    }
}
