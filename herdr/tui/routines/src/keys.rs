//! Keys and clicks: the form first, then the dialog and the palette, then rows, the selected row's actions (a delete
//! asked first), folds, and adding a routine.

use cockpit_core::acts::Act;
use cockpit_core::dialog::{Answer, Dialog};
use cockpit_core::keys::{self, Move};
use cockpit_core::palette::{Entry, Outcome};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Position;

use crate::app::{App, Cmd};
use crate::form;
use crate::rows::Item;

impl App {
    /// Adding and refreshing, then every row's actions and a jump to each; the main actions first.
    fn palette_entries(&mut self) -> Vec<Entry> {
        let mut cmds = vec![Cmd::Add, Cmd::Refresh];
        let mut entries = vec![
            Entry { text: "add a routine".into(), help: "a".into(), featured: true, id: 0 },
            Entry { text: "read the routines again".into(), help: "R".into(), featured: true, id: 1 },
        ];
        // Every routine's runs, folded or not, so the palette finds them all.
        let opened = self.opened.clone();
        self.opened.extend(self.doc.iter().flat_map(|d| d.routines.iter().map(|r| r.slug.clone())));
        let rows = self.rows();
        self.opened = opened;
        for row in rows {
            let (name, help) = (self.row_name(&row.item), self.row_help(&row.item));
            for (act, cmd) in self.acts_for(&row.item) {
                entries.push(Entry { text: format!("{}  {name}", act.label), help: help.clone(), featured: act.main && row.depth == 0, id: cmds.len() });
                cmds.push(cmd);
            }
            entries.push(Entry { text: format!("go to  {name}"), help, featured: false, id: cmds.len() });
            cmds.push(Cmd::GoTo(row.key));
        }
        self.cmds = cmds;
        entries
    }

    /// How the palette names a row: "Morning sweep", "Morning sweep 09:00".
    pub fn row_name(&self, item: &Item) -> String {
        let Some(doc) = &self.doc else { return String::new() };
        let name = doc.routine(item.slug()).map(|r| r.name.clone()).unwrap_or_default();
        match item {
            Item::Routine(_) => name,
            Item::Run(slug, id) => format!("{name} {}", doc.run(slug, id).map(|r| self.clock.when(r.started)).unwrap_or_default()),
        }
    }

    /// What the palette says under a row's entries: when the routine runs, or what the run said.
    fn row_help(&self, item: &Item) -> String {
        let Some(doc) = &self.doc else { return String::new() };
        match item {
            Item::Routine(slug) => doc.routine(slug).map(|r| r.when.clone()).unwrap_or_default(),
            Item::Run(slug, id) => doc.run(slug, id).map(|r| r.summary.clone()).unwrap_or_default(),
        }
    }

    pub fn event(&mut self, event: Event) -> bool {
        let selected = self.selected.clone();
        let quit = match event {
            Event::FocusGained | Event::FocusLost => {
                self.looked.focus(event == Event::FocusGained);
                false
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Mouse(mouse) => {
                let at = Position::new(mouse.column, mouse.row);
                let modal = self.palette.is_open || self.dialog.is_some() || self.form.is_some();
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) if !modal => self.click(at),
                    MouseEventKind::ScrollDown if self.hits.list.contains(at) => self.scroll_by(3),
                    MouseEventKind::ScrollUp if self.hits.list.contains(at) => self.scroll_by(-3),
                    _ => {}
                }
                false
            }
            _ => false,
        };
        // The selection moved onto a row: its marker clears.
        if let Some(row) = self.selected_row().filter(|_| self.selected != selected) {
            let keys = self.marker_keys(&row);
            self.looked.select(keys.iter().map(String::as_str));
        }
        quit
    }

    pub fn key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && keys::is_quit(&key) {
            return true;
        }
        if let Some(form) = &mut self.form {
            match form.key(key) {
                form::Outcome::Cancel => self.form = None,
                form::Outcome::Submit(values) => self.submit(values),
                form::Outcome::Stay => {}
            }
            return false;
        }
        if let Some((dialog, _, _)) = &mut self.dialog {
            if let Some(answer) = dialog.key(key) {
                let (_, cmd, armed) = self.dialog.take().expect("a dialog is open");
                if matches!(answer, Answer::Yes) {
                    self.run(cmd, Some(armed));
                }
            }
            return false;
        }
        if self.palette.is_open {
            match self.palette.key(key) {
                Outcome::Chosen(id) => {
                    if let Some(cmd) = self.cmds.get(id).cloned() {
                        self.press(None, cmd);
                    }
                }
                Outcome::Elsewhere(target) => cockpit_core::hub::go(&target),
                _ => {}
            }
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
        if keys::is_quit(&key) {
            return true;
        }
        if let Some(to) = keys::movement(&key) {
            match to {
                Move::By(n) => self.move_sel(n),
                Move::Pages(n) => self.move_sel(10 * n),
                Move::Top => self.select_edge(false),
                Move::Bottom => self.select_edge(true),
                Move::Group(n) => self.move_group(n),
            }
            return false;
        }
        let acts = self.acts();
        match key.code {
            KeyCode::Char(c) if acts.iter().any(|(a, _)| a.key == Some(c)) => {
                let (act, cmd) = acts.into_iter().find(|(a, _)| a.key == Some(c)).expect("found above");
                self.press(Some(&act), cmd);
            }
            KeyCode::Char('a') => self.run(Cmd::Add, None),
            KeyCode::Char('R') => self.run(Cmd::Refresh, None),
            KeyCode::Char(' ') => self.fold(),
            KeyCode::Enter => match acts.iter().find(|(a, _)| a.main) {
                Some((act, cmd)) => self.press(Some(&act.clone()), cmd.clone()),
                None => self.fold(),
            },
            _ => {}
        }
        false
    }

    /// Runs an action, or asks first when it deletes.
    fn press(&mut self, act: Option<&Act>, cmd: Cmd) {
        let confirm = act.and_then(|a| a.confirm.clone()).or_else(|| self.confirm_of(&cmd));
        match confirm {
            Some(armed) => {
                let question = self.question(&cmd);
                self.dialog = Some((Dialog::Confirm { question }, cmd, armed));
            }
            None => self.run(cmd, None),
        }
    }

    /// The confirmation a command found in the palette carries: the one its button has.
    fn confirm_of(&self, cmd: &Cmd) -> Option<String> {
        let Cmd::Delete(slug) = cmd else { return None };
        self.acts_for(&Item::Routine(slug.clone())).into_iter().find_map(|(a, c)| (c == *cmd).then_some(a.confirm).flatten())
    }

    fn question(&self, cmd: &Cmd) -> String {
        let Cmd::Delete(slug) = cmd else { return "sure?".into() };
        format!("Delete the routine \u{201c}{}\u{201d}, its file and its runs?", self.row_name(&Item::Routine(slug.clone())))
    }

    /// Folds or unfolds the selected routine, or the routine of the selected run, which then takes the selection.
    fn fold(&mut self) {
        let Some(row) = self.selected_row() else { return };
        let slug = row.item.slug().to_string();
        if row.depth > 0 {
            self.selected = Some(slug.clone());
        }
        if row.fold.is_some() || row.depth > 0 {
            self.toggle(&slug);
        }
    }

    fn click(&mut self, at: Position) {
        if let Some((_, cmd)) = self.hits.buttons.iter().find(|(r, _)| r.contains(at)) {
            let cmd = cmd.clone();
            let act = self.acts().into_iter().find(|(_, c)| *c == cmd).map(|(a, _)| a);
            self.press(act.as_ref(), cmd);
        } else if let Some((_, key, glyph)) = self.hits.rows.iter().find(|(r, _, _)| r.contains(at)) {
            let (key, on_glyph) = (key.clone(), glyph.contains(at));
            self.selected = Some(key.clone());
            if on_glyph && !key.contains('/') {
                self.toggle(&key);
            }
        }
    }

    fn scroll_by(&mut self, lines: isize) {
        self.scroll = (self.scroll as isize + lines).max(0) as usize;
        self.follow = false;
    }
}
