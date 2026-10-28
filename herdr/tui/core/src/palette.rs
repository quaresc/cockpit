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
    /// Something found in another app: hand its target to `hub::go`.
    Elsewhere(serde_json::Value),
}

#[derive(Default)]
pub struct Palette {
    pub is_open: bool,
    /// The app's own name: what the hub finds in it is left out, the app's entries already cover it.
    pub app: String,
    query: String,
    entries: Vec<Entry>,
    shown: Vec<usize>,
    elsewhere: Vec<crate::hub::Found>,
    asked: Option<(String, std::sync::mpsc::Receiver<Vec<crate::hub::Found>>)>,
    state: ListState,
}

impl Palette {
    pub fn open(&mut self, entries: Vec<Entry>) {
        self.is_open = true;
        self.query.clear();
        self.entries = entries;
        self.filter();
    }

    /// Asks the hub in the background, so typing never waits on it; the answer shows when it lands (see `arrived`).
    fn ask_hub(&mut self) {
        if self.query.chars().count() < 2 {
            self.elsewhere.clear();
            self.asked = None;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let query = self.query.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::hub::search(&query));
            crate::term::redraw();
        });
        self.asked = Some((self.query.clone(), rx));
    }

    /// The hub's answer for the query typed now, once it is there; an answer to an older query is dropped.
    fn arrived(&mut self) {
        let Some((query, rx)) = &self.asked else { return };
        let Ok(found) = rx.try_recv() else { return };
        if *query == self.query {
            // The hub lists past sessions after the apps' entries: they stay, whatever the apps found.
            let (sessions, apps): (Vec<_>, Vec<_>) = found.into_iter().filter(|f| f.app != self.app).partition(|f| f.app == "session");
            self.elsewhere = apps.into_iter().take(30).chain(sessions).collect();
            if self.state.selected().is_none() && !self.elsewhere.is_empty() {
                self.state.select(Some(0));
            }
        }
        self.asked = None;
    }

    fn filter(&mut self) {
        self.ask_hub();
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
        self.state.select(if self.shown.is_empty() && self.elsewhere.is_empty() { None } else { Some(0) });
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Esc => {
                self.is_open = false;
                return Outcome::Closed;
            }
            KeyCode::Enter => {
                self.is_open = false;
                let selected = self.state.selected().unwrap_or(usize::MAX);
                return match self.shown.get(selected) {
                    Some(&n) => Outcome::Chosen(self.entries[n].id),
                    None => match self.elsewhere.get(selected.wrapping_sub(self.shown.len())) {
                        Some(found) => Outcome::Elsewhere(found.target.clone()),
                        None => Outcome::Closed,
                    },
                };
            }
            KeyCode::Down => self.state.select_next(),
            KeyCode::Up => self.state.select_previous(),
            KeyCode::Tab => self.state.select_next(),
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => self.state.select_next(),
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => self.state.select_previous(),
            KeyCode::Backspace => {
                self.query.pop();
                self.filter();
            }
            // A quick esc then a key arrives as alt+key: the esc closes the palette.
            KeyCode::Char(_) if key.modifiers.contains(KeyModifiers::ALT) => {
                self.is_open = false;
                return Outcome::Closed;
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.push(c);
                self.filter();
            }
            _ => {}
        }
        Outcome::Open
    }

    /// The found item under the selection; None on one of the app's own entries.
    pub fn selected_found(&self) -> Option<&crate::hub::Found> {
        self.state.selected()?.checked_sub(self.shown.len()).and_then(|i| self.elsewhere.get(i))
    }

    /// Over the app, as tall as its entries need.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.arrived();
        let width = area.width.saturating_sub(4).min(90);
        // Each entry takes two lines: its text and its help.
        let height = ((self.shown.len() + self.elsewhere.len()) as u16 * 2 + 4).min(area.height.saturating_sub(2)).max(6);
        self.render_in(frame, Rect { x: area.x + (area.width - width) / 2, y: area.y + 1, width, height });
    }

    /// Filling this box, as the palette app draws it.
    pub fn render_in(&mut self, frame: &mut Frame, box_: Rect) {
        self.arrived();
        frame.render_widget(Clear, box_);
        let block = Block::new().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::new().fg(ORANGE)).style(Style::new().bg(SURFACE));
        let inner = block.inner(box_);
        frame.render_widget(block, box_);
        let [input, list] = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);
        let prompt = Line::from(vec![Span::styled("❯ ", Style::new().fg(ORANGE)), Span::styled(self.query.clone(), Style::new().fg(FG)), Span::styled("▏", Style::new().fg(ORANGE))]);
        frame.render_widget(Paragraph::new(prompt), input);
        let mut items: Vec<ListItem> = self
            .shown
            .iter()
            .map(|&i| {
                let e = &self.entries[i];
                ListItem::new(vec![Line::from(Span::styled(e.text.clone(), Style::new().fg(FG))), Line::from(Span::styled(format!("  {}", e.help), Style::new().fg(DIM)))])
            })
            .collect();
        items.extend(self.elsewhere.iter().map(|f| {
            // A kind that only repeats the app's name (a past session) is said once: "session · <title>".
            let (app, help) = if f.kind == f.app { (format!("{} · ", f.app), f.help.clone()) } else { (format!("{} ", f.app), format!("{} · {}", f.kind, f.help)) };
            ListItem::new(vec![
                Line::from(vec![Span::styled(app, Style::new().fg(ORANGE)), Span::styled(f.title.clone(), Style::new().fg(FG))]),
                Line::from(Span::styled(format!("  {help}"), Style::new().fg(DIM))),
            ])
        }));
        let list_widget = List::new(items).highlight_style(Style::new().bg(PANEL).add_modifier(Modifier::BOLD)).style(Style::new().bg(SURFACE));
        frame.render_stateful_widget(list_widget, list, &mut self.state);
    }
}
