//! Keys and clicks: rows, tabs, the selected step's actions, the runbook's own, the filter bar, picking steps for a batch,
//! the editor, and the palette over all of them.

use cockpit_core::acts::Act;
use cockpit_core::dialog::Answer;
use cockpit_core::input::Edit;
use cockpit_core::palette::{Entry, Outcome};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Position;

use crate::acts::{book_acts, step_acts};
use crate::app::{App, Cmd, key};
use crate::edit;
use crate::filter::Filter;
use crate::table::Column;

impl App {
    /// The selected step's actions, each as the command it runs.
    pub fn step_cmds(&self) -> Vec<Cmd> {
        let Some(book) = self.book() else { return Vec::new() };
        let Some(step) = book.steps.get(self.selected).filter(|_| self.shown().contains(&self.selected)) else { return Vec::new() };
        step_acts(book, step, &self.handoffs).into_iter().map(|act| step_cmd(&book.slug, &step.n, &step.step, act)).collect()
    }

    /// Every runbook's actions, then every step's and a jump to each; before anything is typed, only the main ones.
    fn palette_entries(&mut self) -> Vec<Entry> {
        let mut cmds = Vec::new();
        let mut entries = Vec::new();
        for book in &self.books {
            let name = book.slug.to_uppercase();
            for act in book_acts(book) {
                entries.push(Entry { text: format!("{}  {name} runbook", act.label), help: book.title.clone(), featured: act.main, id: cmds.len() });
                cmds.push(book_cmd(&book.slug, act));
            }
            for step in &book.steps {
                let what = format!("{name} #{} {}", step.n, step.step);
                for act in step_acts(book, step, &self.handoffs) {
                    entries.push(Entry { text: format!("{}  {what}", act.label), help: step.status.clone(), featured: false, id: cmds.len() });
                    cmds.push(step_cmd(&book.slug, &step.n, &step.step, act));
                }
                entries.push(Entry { text: format!("go to  {what}"), help: step.status.clone(), featured: false, id: cmds.len() });
                cmds.push(Cmd::GoTo { slug: book.slug.clone(), n: step.n.clone() });
            }
        }
        self.cmds = cmds;
        entries
    }

    pub fn event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Mouse(mouse) => {
                let at = Position::new(mouse.column, mouse.row);
                let free = self.dialog.is_none() && !self.palette.is_open && self.editor.is_none();
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) if free => self.click(at),
                    MouseEventKind::ScrollDown if self.hits.list.contains(at) => self.scroll_by(3.0),
                    MouseEventKind::ScrollUp if self.hits.list.contains(at) => self.scroll_by(-3.0),
                    _ => {}
                }
                false
            }
            _ => false,
        }
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        if let Some((dialog, _)) = &mut self.dialog {
            match dialog.key(key) {
                Some(Answer::Yes) => {
                    if let Some((_, cmd)) = self.dialog.take() {
                        self.confirmed(cmd);
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
                        self.act(cmd);
                    }
                }
                Outcome::Elsewhere(target) => cockpit_core::hub::go(&target),
                Outcome::Open | Outcome::Closed => {}
            }
            return false;
        }
        if let Some(editor) = &mut self.editor {
            match editor.key(key) {
                edit::Outcome::Save => self.save(),
                edit::Outcome::Cancel => self.editor = None,
                edit::Outcome::Editing => {}
            }
            return false;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if is_global_filter_key(key) {
            self.open_filter(true);
            return false;
        }
        if self.filter.editing {
            self.filter_key(key);
            return false;
        }
        match key.code {
            KeyCode::Char('c' | 'q') if ctrl => return true,
            KeyCode::Char('p') if ctrl => {
                let entries = self.palette_entries();
                self.palette.open(entries);
            }
            // ctrl+d or alt+r is not d or r: an action runs on its own key only.
            KeyCode::Char(_) if ctrl || key.modifiers.contains(KeyModifiers::ALT) => {}
            KeyCode::Char('q') => return true,
            KeyCode::Char('j') | KeyCode::Down => self.move_row(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_row(-1),
            KeyCode::Esc if !self.picked.is_empty() => self.picked.clear(),
            KeyCode::Esc if self.filter.is_on() => self.clear_filter(),
            KeyCode::Char('/') => self.open_filter(false),
            KeyCode::Char('v') => self.pick(),
            KeyCode::Char('V') => self.pick_open(),
            KeyCode::Char('d') if !self.picked.is_empty() => self.ask_done(),
            KeyCode::Char('c') if !self.picked.is_empty() => self.copy_picked(),
            KeyCode::Char('e') => self.edit_selected(Column::Command),
            KeyCode::Char('E') => self.edit_selected(Column::Status),
            KeyCode::Char('a') => self.add(),
            KeyCode::Enter => {
                if let Some(cmd) = self.step_cmds().into_iter().next() {
                    self.act(cmd);
                }
            }
            KeyCode::Char('n') => self.next_tab(1),
            KeyCode::Char('p') => self.next_tab(-1),
            KeyCode::Char(c) => {
                let step = self.step_cmds().into_iter().find(|cmd| matches!(cmd, Cmd::Act { act, .. } if act.key == Some(c)));
                let book = || self.book().and_then(|b| book_acts(b).into_iter().find(|a| a.key == Some(c)).map(|a| book_cmd(&b.slug, a)));
                if let Some(cmd) = step.or_else(book) {
                    self.act(cmd);
                }
            }
            _ => {}
        }
        false
    }

    /// A key while the filter bar is typed in: the list follows each change, `/` widens it to every runbook and back.
    fn filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('/') if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                self.filter.global = !self.filter.global;
                self.refilter();
            }
            KeyCode::Down => self.move_row(1),
            KeyCode::Up => self.move_row(-1),
            _ => match self.filter.input.key(key) {
                Edit::Submit if self.filter.query().is_empty() => self.clear_filter(),
                Edit::Submit => self.filter.editing = false,
                Edit::Cancel => self.clear_filter(),
                Edit::Changed => self.refilter(),
                Edit::Ignored => {}
            },
        }
    }

    fn open_filter(&mut self, global: bool) {
        self.filter.editing = true;
        self.filter.global |= global;
        self.refilter();
    }

    fn clear_filter(&mut self) {
        self.filter = Filter::default();
        self.refilter();
        self.reveal = Some((self.selected, self.selected));
    }

    /// The step under the cursor picked or put back.
    fn pick(&mut self) {
        let Some(book) = self.book() else { return };
        let Some(step) = book.steps.get(self.selected) else { return };
        let k = key(&book.slug, &step.n);
        if !self.picked.remove(&k) {
            self.picked.insert(k);
        }
    }

    /// Every open step shown picked; all of them put back when they were already.
    fn pick_open(&mut self) {
        let Some(book) = self.book() else { return };
        let open: Vec<String> = self.shown().into_iter().filter(|&i| book.steps[i].is_open()).map(|i| key(&book.slug, &book.steps[i].n)).collect();
        if open.iter().all(|k| self.picked.contains(k)) {
            for k in &open {
                self.picked.remove(k);
            }
        } else {
            self.picked.extend(open);
        }
    }

    fn edit_selected(&mut self, column: Column) {
        if let Some((slug, n)) = self.book().and_then(|b| b.steps.get(self.selected).map(|s| (b.slug.clone(), s.n.clone()))) {
            self.edit(&slug, &n, column);
        }
    }

    fn click(&mut self, at: Position) {
        if let Some(&(_, i)) = self.hits.buttons.iter().find(|(r, _)| r.contains(at)) {
            if let Some(cmd) = self.step_cmds().into_iter().nth(i) {
                self.act(cmd);
            }
        } else if let Some(&(_, i)) = self.hits.tabs.iter().find(|(r, _)| r.contains(at)) {
            self.go_tab(i);
        } else if let Some(&(_, i)) = self.hits.rows.iter().find(|(r, _)| r.contains(at)) {
            self.selected = i;
            self.reveal = Some((i, i));
        }
    }

    /// The cursor `by` rows on, among the shown ones.
    fn move_row(&mut self, by: isize) {
        let shown = self.shown();
        if shown.is_empty() {
            return;
        }
        let at = shown.iter().position(|&i| i == self.selected).unwrap_or(0) as isize;
        self.selected = shown[(at + by).clamp(0, shown.len() as isize - 1) as usize];
        self.reveal = Some((self.selected, self.selected));
    }

    fn next_tab(&mut self, by: isize) {
        let count = self.books.len() as isize;
        if count > 0 {
            self.go_tab((self.active as isize + by).rem_euclid(count) as usize);
        }
    }

    fn scroll_by(&mut self, lines: f32) {
        if let Some(book) = self.book() {
            let slug = book.slug.clone();
            let scroll = self.scroll.entry(slug).or_insert_with(|| cockpit_core::anim::Glide::new(0.0, 0.25));
            scroll.go(scroll.target() + lines);
        }
    }
}

/// ctrl+/ as terminals send it: some as ctrl+/, most as the 0x1f byte that reads as ctrl+7 or ctrl+_.
fn is_global_filter_key(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('/' | '7' | '_'))
}

fn step_cmd(slug: &str, n: &str, step: &str, act: Act) -> Cmd {
    let question = format!("{} {} step {n}: {step}", act.label, slug.to_uppercase());
    Cmd::Act { slug: slug.into(), n: Some(n.into()), act, question }
}

fn book_cmd(slug: &str, act: Act) -> Cmd {
    let question = format!("{} the {} runbook", act.label, slug.to_uppercase());
    Cmd::Act { slug: slug.into(), n: None, act, question }
}
