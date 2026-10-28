//! Editing in place: a step's command or status in a text field, or a new step's text, place and command; drawn in a box
//! over the bottom of the list, written to the runbook file by the app on enter.

use cockpit_core::input::{Edit, Input};
use cockpit_core::theme::{DIM, FG, ORANGE, SURFACE, dim, fg};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap};

use crate::table::Column;

pub enum Editor {
    /// One cell of step `n`.
    Cell { slug: String, n: String, column: Column, input: Input },
    /// A new step under step `after`, its fields step, where and command, `at` the one typed in.
    Add { slug: String, after: Option<String>, fields: [Input; 3], at: usize },
}

pub enum Outcome {
    Editing,
    Cancel,
    Save,
}

const ADD_LABELS: [&str; 3] = ["step     ", "where    ", "command  "];

impl Editor {
    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        match self {
            Editor::Cell { input, .. } => match input.key(key) {
                Edit::Submit => Outcome::Save,
                Edit::Cancel => Outcome::Cancel,
                _ => Outcome::Editing,
            },
            Editor::Add { fields, at, .. } => match key.code {
                KeyCode::Tab | KeyCode::Down => {
                    *at = (*at + 1) % fields.len();
                    Outcome::Editing
                }
                KeyCode::BackTab | KeyCode::Up => {
                    *at = (*at + fields.len() - 1) % fields.len();
                    Outcome::Editing
                }
                // Enter goes down the fields, and saves from the last one.
                KeyCode::Enter if *at + 1 < fields.len() => {
                    *at += 1;
                    Outcome::Editing
                }
                _ => match fields[*at].key(key) {
                    Edit::Submit => Outcome::Save,
                    Edit::Cancel => Outcome::Cancel,
                    _ => Outcome::Editing,
                },
            },
        }
    }

    /// The box's title, its fields as lines, and the keys under them.
    fn lines(&self) -> (String, Vec<Line<'static>>, &'static str) {
        match self {
            Editor::Cell { slug, n, column, input } => {
                let what = if *column == Column::Command { "command" } else { "status" };
                (format!(" {} step {n}: {what} ", slug.to_uppercase()), vec![input.line(if *column == Column::Command { "$ " } else { "" })], "enter saves to the runbook · esc cancels")
            }
            Editor::Add { slug, after, fields, at } => {
                let place = after.as_ref().map_or("at the end".to_string(), |n| format!("under step {n}"));
                let lines = fields
                    .iter()
                    .zip(ADD_LABELS)
                    .enumerate()
                    .map(|(i, (input, label))| {
                        if i == *at {
                            input.line(label)
                        } else {
                            Line::from(vec![Span::styled(label, dim()), Span::styled(input.text.clone(), fg(FG))])
                        }
                    })
                    .collect();
                (format!(" {}: a new step {place} ", slug.to_uppercase()), lines, "tab or enter next field · enter on the last saves · esc cancels")
            }
        }
    }

    /// Over the bottom of `area`, the full width less a margin.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let (title, mut lines, hint) = self.lines();
        lines.push(Line::from(Span::styled(hint, Style::new().fg(DIM))));
        // A long command wraps, so its cursor stays in sight.
        let width = area.width.saturating_sub(2);
        let inner = width.saturating_sub(4).max(1) as usize;
        let rows: usize = lines.iter().map(|l| l.width().max(1).div_ceil(inner)).sum();
        let height = (rows as u16 + 2).min(area.height);
        let rect = Rect { x: area.x + 1, y: area.y + area.height - height, width, height };
        frame.render_widget(Clear, rect);
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(fg(ORANGE))
            .title(Span::styled(title, fg(ORANGE)))
            .padding(Padding::horizontal(1))
            .style(Style::new().bg(SURFACE));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), rect);
    }
}
