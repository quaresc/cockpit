//! A one-line text field: typing, the cursor, word and line deletion, drawn with a block cursor.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{BG, FG, ORANGE};

#[derive(Clone, Debug, Default)]
pub struct Input {
    pub text: String,
    /// In characters, not bytes.
    cursor: usize,
}

pub enum Edit {
    /// The text or the cursor moved.
    Changed,
    Submit,
    Cancel,
    /// Not a key for the field.
    Ignored,
}

impl Input {
    pub fn new(text: &str) -> Self {
        Self { text: text.into(), cursor: text.chars().count() }
    }

    fn byte(&self, at: usize) -> usize {
        self.text.char_indices().nth(at).map_or(self.text.len(), |(i, _)| i)
    }

    pub fn key(&mut self, key: KeyEvent) -> Edit {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let len = self.text.chars().count();
        match key.code {
            KeyCode::Enter => return Edit::Submit,
            KeyCode::Esc => return Edit::Cancel,
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = len,
            KeyCode::Char('u') if ctrl => {
                self.text.drain(..self.byte(self.cursor));
                self.cursor = 0;
            }
            KeyCode::Char('w') if ctrl => {
                let before: Vec<char> = self.text.chars().take(self.cursor).collect();
                let mut start = before.len();
                while start > 0 && before[start - 1] == ' ' {
                    start -= 1;
                }
                while start > 0 && before[start - 1] != ' ' {
                    start -= 1;
                }
                let (from, to) = (self.byte(start), self.byte(self.cursor));
                self.text.drain(from..to);
                self.cursor = start;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let (from, to) = (self.byte(self.cursor - 1), self.byte(self.cursor));
                self.text.drain(from..to);
                self.cursor -= 1;
            }
            KeyCode::Delete if self.cursor < len => {
                let (from, to) = (self.byte(self.cursor), self.byte(self.cursor + 1));
                self.text.drain(from..to);
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                let at = self.byte(self.cursor);
                self.text.insert(at, c);
                self.cursor += 1;
            }
            _ => return Edit::Ignored,
        }
        Edit::Changed
    }

    /// The text with its cursor, after a prompt.
    pub fn line(&self, prompt: &str) -> Line<'static> {
        let before: String = self.text.chars().take(self.cursor).collect();
        let at: String = self.text.chars().nth(self.cursor).map_or(" ".into(), |c| c.to_string());
        let after: String = self.text.chars().skip(self.cursor + 1).collect();
        Line::from(vec![
            Span::styled(prompt.to_string(), Style::new().fg(ORANGE)),
            Span::styled(before, Style::new().fg(FG)),
            Span::styled(at, Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD)),
            Span::styled(after, Style::new().fg(FG)),
        ])
    }
}
