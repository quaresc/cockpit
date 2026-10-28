//! Keys and clicks: rows, the selected row's actions (a roll back asked first), folds, the view switch, and the
//! palette over all of them.

use cockpit_core::acts::Act;
use cockpit_core::dialog::{Answer, Dialog};
use cockpit_core::keys::{self, Move};
use cockpit_core::palette::{Entry, Outcome};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Position;

use crate::app::{App, Cmd};
use crate::rows::Item;

impl App {
    /// The view's switch and a refresh, then every row's actions and a jump to each; the main actions first.
    fn palette_entries(&mut self) -> Vec<Entry> {
        let mut cmds = vec![Cmd::View, Cmd::Refresh];
        let view = if self.timeline { "apps view" } else { "releases view" };
        let mut entries = vec![
            Entry { text: view.into(), help: "w".into(), featured: true, id: 0 },
            Entry { text: "refresh the monitor".into(), help: "R".into(), featured: true, id: 1 },
        ];
        let was = self.timeline;
        // Every app's rows, folded or not, so the palette finds them all.
        let (opened, closed) = (self.opened.clone(), self.closed.clone());
        self.timeline = false;
        self.opened.extend(self.doc.iter().flat_map(|d| d.apps.iter().map(|a| a.id.clone())));
        self.closed.clear();
        let rows = self.rows();
        (self.timeline, self.opened, self.closed) = (was, opened, closed);
        for row in rows {
            let (name, help) = (self.row_name(&row.item), self.row_help(&row.item));
            for (act, cmd) in self.acts_for(&row.item) {
                entries.push(Entry { text: format!("{}  {name}", act.label), help: help.clone(), featured: act.main && row.depth > 0, id: cmds.len() });
                cmds.push(cmd);
            }
            entries.push(Entry { text: format!("go to  {name}"), help, featured: false, id: cmds.len() });
            cmds.push(Cmd::GoTo(row.key));
        }
        self.cmds = cmds;
        entries
    }

    /// How the palette names a row: "api", "api v2026.10.28.1", "api Req.TransportError".
    pub fn row_name(&self, item: &Item) -> String {
        let Some(app) = self.doc.as_ref().and_then(|d| d.app(item.app())) else { return String::new() };
        match item {
            Item::App(_) => app.name.clone(),
            Item::Monitor(_, id) => {
                let name = app.alerting().iter().find(|m| &crate::data::id_text(&m.id) == id).map(|m| m.name.clone()).unwrap_or_default();
                format!("{} {name}", app.name)
            }
            Item::Issue(_, id) => format!("{} {}", app.name, app.issues.iter().find(|i| &i.id == id).map(|i| i.kind.clone()).unwrap_or_default()),
            Item::Release(_, id) => format!("{} {}", app.name, app.release(id).map(|r| r.tag.clone()).unwrap_or_default()),
        }
    }

    /// What the palette says under a row's entries: the app's tag, the monitor's state, the error's message or the
    /// release's verdict.
    fn row_help(&self, item: &Item) -> String {
        let Some(app) = self.doc.as_ref().and_then(|d| d.app(item.app())) else { return String::new() };
        match item {
            Item::App(_) => format!("{} · {}", app.tag, app.health),
            Item::Monitor(_, id) => app.alerting().iter().find(|m| &crate::data::id_text(&m.id) == id).map(|m| m.status.clone()).unwrap_or_default(),
            Item::Issue(_, id) => app.issues.iter().find(|i| &i.id == id).map(|i| i.message.clone()).unwrap_or_default(),
            Item::Release(_, id) => app.release(id).map(|r| r.verdict.summary.clone()).unwrap_or_default(),
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
                let modal = self.palette.is_open || self.dialog.is_some();
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
        // The selection moved onto a row: its marker, and those of the rows it hides folded, clear.
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
            KeyCode::Char('w') => self.run(Cmd::View, None),
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

    /// Runs an action, or asks first when it changes something.
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
        let Cmd::RollBack(app, release, _) = cmd else { return None };
        self.acts_for(&Item::Release(app.clone(), release.clone())).into_iter().find_map(|(a, c)| (c == *cmd).then_some(a.confirm).flatten())
    }

    fn question(&self, cmd: &Cmd) -> String {
        let Cmd::RollBack(app, release, previous) = cmd else { return "sure?".into() };
        let name = self.row_name(&Item::Release(app.clone(), release.clone()));
        format!("roll back {name} to {previous}? A Claude session checks {previous} and asks before any deploy.")
    }

    /// Folds or unfolds the selected app, or the app of the selected row, which then takes the selection.
    fn fold(&mut self) {
        let Some(row) = self.selected_row() else { return };
        if self.timeline {
            return;
        }
        let app = row.item.app().to_string();
        if row.depth > 0 {
            self.selected = Some(app.clone());
        }
        self.toggle(&app);
    }

    fn click(&mut self, at: Position) {
        if let Some((_, cmd)) = self.hits.buttons.iter().find(|(r, _)| r.contains(at)) {
            let cmd = cmd.clone();
            let act = self.acts().into_iter().find(|(_, c)| *c == cmd).map(|(a, _)| a);
            self.press(act.as_ref(), cmd);
        } else if let Some((_, key, glyph)) = self.hits.rows.iter().find(|(r, _, _)| r.contains(at)) {
            let (key, on_glyph) = (key.clone(), glyph.contains(at));
            self.selected = Some(key.clone());
            if on_glyph && !self.timeline {
                self.toggle(&key);
            }
        }
    }

    fn scroll_by(&mut self, lines: isize) {
        self.scroll = (self.scroll as isize + lines).max(0) as usize;
        self.follow = false;
    }
}
