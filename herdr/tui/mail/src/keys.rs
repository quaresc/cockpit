//! Keys and clicks: moving through the list, the selected message's actions, the preview's scroll, a double click
//! reading a message whole.

use std::time::{Duration, Instant};

use cockpit_core::keys::{self, Move};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Position;

use crate::app::{App, Cmd};

const DOUBLE_CLICK: Duration = Duration::from_millis(450);

impl App {
    pub fn event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Mouse(mouse) => {
                let at = Position::new(mouse.column, mouse.row);
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => self.click(at),
                    MouseEventKind::ScrollDown if self.hits.body.contains(at) => self.scroll_body(3),
                    MouseEventKind::ScrollUp if self.hits.body.contains(at) => self.scroll_body(-3),
                    MouseEventKind::ScrollDown if self.hits.list.contains(at) => self.move_sel(1),
                    MouseEventKind::ScrollUp if self.hits.list.contains(at) => self.move_sel(-1),
                    _ => {}
                }
                false
            }
            _ => false,
        }
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if keys::is_quit(&key) {
            return true;
        }
        if ctrl {
            match key.code {
                KeyCode::Char('d') => self.scroll_body(10),
                KeyCode::Char('u') => self.scroll_body(-10),
                _ => {}
            }
            return false;
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        if let Some(to) = keys::movement(&key) {
            match to {
                Move::By(n) => self.move_sel(n),
                Move::Pages(n) => self.scroll_body(10 * n),
                Move::Top => self.move_sel(-1_000_000),
                Move::Bottom => self.move_sel(1_000_000),
                Move::Group(n) => self.next_group(n),
            }
            return false;
        }
        let Some(id) = self.selected.clone() else {
            if key.code == KeyCode::Char('R') {
                self.run(Cmd::Refresh);
            }
            return false;
        };
        match key.code {
            KeyCode::Enter => self.run(Cmd::See(id)),
            KeyCode::Char(' ') => self.scroll_body(10),
            KeyCode::Char('R') => self.run(Cmd::Refresh),
            KeyCode::Char(c) => {
                if let Some((_, cmd)) = self.acts().into_iter().find(|(a, _)| a.key == Some(c)) {
                    self.run(cmd);
                }
            }
            _ => {}
        }
        false
    }

    /// The first message of the next group (Read) or the previous one (Unread).
    fn next_group(&mut self, by: isize) {
        let in_unread = self.selected_message().is_some_and(|m| m.unread);
        let target = if by > 0 { in_unread.then_some(false) } else { Some(true) };
        let found = target.and_then(|t| self.mail.messages.iter().find(|m| m.unread == t)).or(if by < 0 { self.mail.messages.first() } else { None });
        if let Some(id) = found.map(|m| m.id.clone()) {
            self.select(&id);
        }
    }

    fn scroll_body(&mut self, lines: isize) {
        self.body_scroll = (self.body_scroll as isize + lines).max(0) as usize;
    }

    fn click(&mut self, at: Position) {
        if let Some((_, cmd)) = self.hits.buttons.iter().find(|(r, _)| r.contains(at)) {
            let cmd = cmd.clone();
            self.run(cmd);
        } else if let Some((_, id)) = self.hits.rows.iter().find(|(r, _)| r.contains(at)) {
            let id = id.clone();
            let double = self.last_click.as_ref().is_some_and(|(when, was)| *was == id && when.elapsed() < DOUBLE_CLICK);
            self.select(&id);
            self.last_click = Some((Instant::now(), id.clone()));
            if double {
                self.run(Cmd::See(id));
            }
        }
    }
}
