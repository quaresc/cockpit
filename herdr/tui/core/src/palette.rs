//! The command palette: every action of every row, found by a fuzzy match on what you type (ctrl+p).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::theme::{DIM, FG, ORANGE, PANEL, SURFACE};

#[derive(Clone, Debug)]
pub struct Entry {
    pub text: String,
    pub help: String,
    /// Shown before anything is typed.
    pub featured: bool,
    pub id: usize,
}

pub enum Outcome {
    Open,
    Closed,
    Chosen(usize),
}

#[derive(Default)]
pub struct Palette {
    pub is_open: bool,
    query: String,
    entries: Vec<Entry>,
    shown: Vec<usize>,
    state: ListState,
}

impl Palette {
    pub fn open(&mut self, entries: Vec<Entry>) {
        self.is_open = true;
        self.query.clear();
        self.entries = entries;
        self.filter();
    }

    fn filter(&mut self) {
        if self.query.is_empty() {
            self.shown = (0..self.entries.len()).filter(|&i| self.entries[i].featured).collect();
        } else {
            let pattern = Pattern::parse(&self.query, CaseMatching::Ignore, Normalization::Smart);
            let mut matcher = Matcher::new(Config::DEFAULT);
            let texts: Vec<(usize, &str)> = self.entries.iter().enumerate().map(|(i, e)| (i, e.text.as_str())).collect();
            let mut scored: Vec<(usize, u32)> = texts
                .iter()
                .filter_map(|(i, text)| {
                    let mut buf = Vec::new();
                    pattern.score(nucleo_matcher::Utf32Str::new(text, &mut buf), &mut matcher).map(|s| (*i, s))
                })
                .collect();
            scored.sort_by_key(|s| std::cmp::Reverse(s.1));
            self.shown = scored.into_iter().map(|(i, _)| i).collect();
        }
        self.state.select(if self.shown.is_empty() { None } else { Some(0) });
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Esc => {
                self.is_open = false;
                return Outcome::Closed;
            }
            KeyCode::Enter => {
                self.is_open = false;
                return match self.state.selected().and_then(|i| self.shown.get(i)) {
                    Some(&n) => Outcome::Chosen(self.entries[n].id),
                    None => Outcome::Closed,
                };
            }
            KeyCode::Down => self.state.select_next(),
            KeyCode::Up => self.state.select_previous(),
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => self.state.select_next(),
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => self.state.select_previous(),
            KeyCode::Backspace => {
                self.query.pop();
                self.filter();
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.filter();
            }
            _ => {}
        }
        Outcome::Open
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let width = area.width.saturating_sub(4).min(90);
        // Each entry takes two lines: its text and its help.
        let height = (self.shown.len() as u16 * 2 + 4).min(area.height.saturating_sub(2)).max(6);
        let box_ = Rect { x: area.x + (area.width - width) / 2, y: area.y + 1, width, height };
        frame.render_widget(Clear, box_);
        let block = Block::new().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::new().fg(ORANGE)).style(Style::new().bg(SURFACE));
        let inner = block.inner(box_);
        frame.render_widget(block, box_);
        let [input, list] = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);
        let prompt = Line::from(vec![Span::styled("❯ ", Style::new().fg(ORANGE)), Span::styled(self.query.clone(), Style::new().fg(FG)), Span::styled("▏", Style::new().fg(ORANGE))]);
        frame.render_widget(Paragraph::new(prompt), input);
        let items: Vec<ListItem> = self
            .shown
            .iter()
            .map(|&i| {
                let e = &self.entries[i];
                ListItem::new(vec![Line::from(Span::styled(e.text.clone(), Style::new().fg(FG))), Line::from(Span::styled(format!("  {}", e.help), Style::new().fg(DIM)))])
            })
            .collect();
        let list_widget = List::new(items).highlight_style(Style::new().bg(PANEL).add_modifier(Modifier::BOLD)).style(Style::new().bg(SURFACE));
        frame.render_stateful_widget(list_widget, list, &mut self.state);
    }
}
