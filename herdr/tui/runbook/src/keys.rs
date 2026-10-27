//! Keys and clicks: rows, tabs, the selected step's actions, the runbook's own, and the palette over all of them.

use cockpit_core::acts::Act;
use cockpit_core::dialog::Answer;
use cockpit_core::palette::{Entry, Outcome};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Position;

use crate::acts::{book_acts, step_acts};
use crate::app::{App, Cmd};

impl App {
    /// The selected step's actions, each as the command it runs.
    pub fn step_cmds(&self) -> Vec<Cmd> {
        let Some(book) = self.book() else { return Vec::new() };
        let Some(step) = book.steps.get(self.selected) else { return Vec::new() };
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
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) if self.dialog.is_none() && !self.palette.is_open => self.click(at),
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
                    if let Some((_, Cmd::Act { slug, n, act, .. })) = self.dialog.take() {
                        self.perform(&slug, n.as_deref(), &act, act.confirm.clone());
                    }
                }
                Some(_) => self.dialog = None,
                None => {}
            }
            return false;
        }
        if self.palette.is_open {
            if let Outcome::Chosen(id) = self.palette.key(key)
                && let Some(cmd) = self.cmds.get(id).cloned()
            {
                self.act(cmd);
            }
            return false;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
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

    fn move_row(&mut self, by: isize) {
        let Some(book) = self.book() else { return };
        let last = book.steps.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + by).clamp(0, last) as usize;
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
            let scroll = self.scroll.entry(slug).or_insert_with(|| crate::anim::Glide::new(0.0, 0.25));
            scroll.go(scroll.target() + lines);
        }
    }
}

fn step_cmd(slug: &str, n: &str, step: &str, act: Act) -> Cmd {
    let question = format!("{} {} step {n}: {step}", act.label, slug.to_uppercase());
    Cmd::Act { slug: slug.into(), n: Some(n.into()), act, question }
}

fn book_cmd(slug: &str, act: Act) -> Cmd {
    let question = format!("{} the {} runbook", act.label, slug.to_uppercase());
    Cmd::Act { slug: slug.into(), n: None, act, question }
}
