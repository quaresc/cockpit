//! Keys and clicks: the text fields first (adding, editing, the filter), then the rows, the selection's actions, the
//! marks, and the palette over all of them.

use cockpit_core::acts::Act;
use cockpit_core::dialog::Answer;
use cockpit_core::input::Edit;
use cockpit_core::palette::{Entry, Outcome};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Position;

use crate::app::{App, Cmd, Sel};
use crate::links;

/// An action without a key of its own: the palette and a click reach it.
fn keyless(label: &str, id: &str) -> Act {
    Act { label: label.into(), key: None, id: id.into(), ..Default::default() }
}

impl App {
    /// The selection's actions, each as the command it runs; the marked items' when some are marked.
    pub fn acts(&self) -> Vec<(Act, Cmd)> {
        if !self.marks.is_empty() {
            let targets = self.targets();
            let n = targets.len();
            return vec![
                (Act::new(&format!("toggle {n}"), 'x', "toggle").main(), Cmd::Toggle(targets.clone())),
                (Act::new(&format!("delete {n}"), 'D', "delete"), Cmd::Delete(targets)),
                (keyless("unmark all", "unmark"), Cmd::ClearMarks),
            ];
        }
        match &self.sel {
            Sel::Adder => vec![(Act::new("add", 'a', "add").main(), Cmd::Add)],
            Sel::Fold => vec![(Act::new(if self.done_unfolded() { "hide done" } else { "show done" }, 'z', "fold").main(), Cmd::Fold)],
            Sel::Item(text) => {
                let Some(item) = self.item(text) else { return Vec::new() };
                let text = item.text.clone();
                let mut acts = vec![
                    (Act::new(if item.done { "reopen" } else { "done" }, 'x', "toggle").main(), Cmd::Toggle(vec![text.clone()])),
                    (Act::new("edit", 'e', "edit"), Cmd::Edit(text.clone())),
                ];
                if let Some((token, _)) = links::split(&text, &self.tokens).into_iter().find(|p| p.1) {
                    let shown = if token.to_lowercase().starts_with("http") { "link".to_string() } else { token.to_uppercase() };
                    acts.push((Act::new(&format!("open {shown}"), 'o', "open"), Cmd::Open(token.to_string())));
                }
                if !item.done {
                    acts.push((Act::new("up", 'K', "up"), Cmd::Move(text.clone(), -1)));
                    acts.push((Act::new("down", 'J', "down"), Cmd::Move(text.clone(), 1)));
                }
                acts.push((Act::new("mark", 'v', "mark"), Cmd::Mark(text.clone())));
                acts.push((Act::new("delete", 'D', "delete"), Cmd::Delete(vec![text])));
                acts
            }
        }
    }

    /// The list's own actions first, then every item's and every link's; before anything is typed, the main ones.
    fn palette_entries(&mut self) -> Vec<Entry> {
        let mut cmds = Vec::new();
        let mut entries = Vec::new();
        let mut push = |text: String, help: &str, featured: bool, cmd: Cmd| {
            entries.push(Entry { text, help: help.to_string(), featured, id: cmds.len() });
            cmds.push(cmd);
        };
        push("add a to-do".into(), "a", true, Cmd::Add);
        if self.can_undo() {
            push("undo the last change".into(), "u", true, Cmd::Undo);
        }
        let done = self.items().iter().filter(|i| i.done).count();
        if done > 0 {
            push(format!("{} the {done} done", if self.show_done { "hide" } else { "show" }), "z", true, Cmd::Fold);
        }
        for item in self.items() {
            let text = item.text.clone();
            for (token, is_token) in links::split(&text, &self.tokens) {
                if is_token {
                    let url = links::url(token, &self.config, &self.inbox).unwrap_or_default();
                    push(format!("open {token}  {text}"), &url, !item.done, Cmd::Open(token.to_string()));
                }
            }
            let (verb, status) = if item.done { ("reopen", "done") } else { ("done", "open") };
            push(format!("{verb}  {text}"), status, false, Cmd::Toggle(vec![text.clone()]));
            push(format!("edit  {text}"), status, false, Cmd::Edit(text.clone()));
            if !item.done {
                push(format!("move up  {text}"), status, false, Cmd::Move(text.clone(), -1));
                push(format!("move down  {text}"), status, false, Cmd::Move(text.clone(), 1));
            }
            push(format!("delete  {text}"), status, false, Cmd::Delete(vec![text.clone()]));
            push(format!("go to  {text}"), status, false, Cmd::GoTo(text));
        }
        self.cmds = cmds;
        entries
    }

    pub fn event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Mouse(mouse) => {
                let at = Position::new(mouse.column, mouse.row);
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) if self.dialog.is_none() && !self.palette.is_open => self.click(at),
                    MouseEventKind::ScrollDown if self.hits.list.contains(at) => self.scroll_by(3),
                    MouseEventKind::ScrollUp if self.hits.list.contains(at) => self.scroll_by(-3),
                    _ => {}
                }
                false
            }
            _ => false,
        }
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('c' | 'q')) {
            return true;
        }
        if let Some((dialog, _)) = &mut self.dialog {
            match dialog.key(key) {
                Some(Answer::Yes) => {
                    if let Some((_, Cmd::Delete(texts))) = self.dialog.take() {
                        self.delete(&texts);
                    }
                }
                Some(_) => self.dialog = None,
                None => {}
            }
            return false;
        }
        if self.palette.is_open {
            match self.palette.key(key) {
                Outcome::Chosen(id) => {
                    if let Some(cmd) = self.cmds.get(id).cloned() {
                        self.run(cmd);
                    }
                }
                Outcome::Elsewhere(target) => cockpit_core::hub::go(&target),
                _ => {}
            }
            return false;
        }
        if let Some(editing) = &mut self.editing {
            match editing.input.key(key) {
                Edit::Submit => self.commit_edit(),
                Edit::Cancel => self.editing = None,
                _ => {}
            }
            return false;
        }
        if self.filtering {
            match self.filter.key(key) {
                Edit::Submit => self.filtering = false,
                Edit::Cancel => {
                    self.filtering = false;
                    self.filter = Default::default();
                }
                _ => {}
            }
            self.keep_visible();
            return false;
        }
        if ctrl && key.code == KeyCode::Char('p') {
            let entries = self.palette_entries();
            self.palette.open(entries);
            return false;
        }
        if ctrl || key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        let item = self.selected_item();
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::PageDown => self.move_sel(10),
            KeyCode::PageUp => self.move_sel(-10),
            KeyCode::Home | KeyCode::Char('g') => self.select_at(0),
            KeyCode::End | KeyCode::Char('G') => self.select_at(usize::MAX),
            KeyCode::Char('a') => self.start_edit(None),
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('z') => self.show_done = !self.show_done,
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Esc if !self.marks.is_empty() => self.marks.clear(),
            KeyCode::Esc => self.filter = Default::default(),
            KeyCode::Enter | KeyCode::Char('e') => match (&self.sel, item) {
                (Sel::Item(_), Some(item)) => self.start_edit(Some(item.text)),
                (Sel::Fold, _) => self.show_done = !self.show_done,
                _ => self.start_edit(None),
            },
            KeyCode::Char(' ' | 'x') if self.sel == Sel::Fold => self.show_done = !self.show_done,
            KeyCode::Char(' ' | 'x') => {
                let targets = self.targets();
                self.toggle(&targets);
            }
            KeyCode::Char('J') if self.marks.is_empty() => {
                if let Some(item) = item {
                    self.shift(&item.text, 1);
                }
            }
            KeyCode::Char('K') if self.marks.is_empty() => {
                if let Some(item) = item {
                    self.shift(&item.text, -1);
                }
            }
            KeyCode::Char('D') => self.run(Cmd::Delete(self.targets())),
            KeyCode::Char('v') => {
                if let Some(item) = item {
                    self.run(Cmd::Mark(item.text));
                }
            }
            KeyCode::Char('o') => {
                if let Some((_, Cmd::Open(token))) = self.acts().into_iter().find(|(a, _)| a.key == Some('o')) {
                    self.open_link(&token);
                }
            }
            _ => {}
        }
        false
    }

    /// The selection back on a row the filter shows.
    fn keep_visible(&mut self) {
        if !self.selectable().contains(&self.sel) {
            self.select_at(1);
        }
    }

    fn click(&mut self, at: Position) {
        if let Some(i) = hit(&self.hits.buttons, at) {
            let cmd = self.hits.buttons[i].1.clone();
            self.commit_edit();
            self.run(cmd);
        } else if let Some(i) = hit(&self.hits.tokens, at) {
            let token = self.hits.tokens[i].1.clone();
            self.open_link(&token);
        } else if let Some(i) = hit(&self.hits.glyphs, at) {
            let text = self.hits.glyphs[i].1.clone();
            self.commit_edit();
            self.toggle(&[text]);
        } else if let Some(i) = hit(&self.hits.rows, at) {
            let sel = self.hits.rows[i].1.clone();
            if self.editing.as_ref().is_some_and(|e| e.item.clone().map_or(Sel::Adder, Sel::Item) == sel) {
                return;
            }
            self.commit_edit();
            match sel {
                Sel::Adder => self.start_edit(None),
                Sel::Fold => {
                    self.sel = Sel::Fold;
                    self.show_done = !self.show_done;
                }
                sel => self.sel = sel,
            }
            self.follow = true;
        }
    }

    fn scroll_by(&mut self, lines: isize) {
        self.scroll = (self.scroll as isize + lines).max(0) as usize;
        self.follow = false;
    }
}

/// Which of these places the click landed in.
fn hit<T>(places: &[(ratatui::layout::Rect, T)], at: Position) -> Option<usize> {
    places.iter().position(|(r, _)| r.contains(at))
}
