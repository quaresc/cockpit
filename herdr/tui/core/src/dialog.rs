//! A question in a box over the app: yes or no before an action that changes something, or one choice among a few.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use crate::theme::{BG, DIM, FG, LINE, ORANGE, PANEL, SURFACE};

pub enum Dialog {
    Confirm { question: String },
    Pick { question: String, options: Vec<String>, selected: usize },
}

pub enum Answer {
    Yes,
    Picked(usize),
    Cancel,
}

impl Dialog {
    pub fn key(&mut self, key: KeyEvent) -> Option<Answer> {
        match self {
            Dialog::Confirm { .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => Some(Answer::Yes),
                KeyCode::Char('n') | KeyCode::Esc => Some(Answer::Cancel),
                _ => None,
            },
            Dialog::Pick { options, selected, .. } => match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    *selected = (*selected + 1).min(options.len().saturating_sub(1));
                    None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    *selected = selected.saturating_sub(1);
                    None
                }
                KeyCode::Enter => Some(Answer::Picked(*selected)),
                KeyCode::Esc => Some(Answer::Cancel),
                _ => None,
            },
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let width = 64.min(area.width.saturating_sub(4));
        let (question, body): (&str, Vec<Line>) = match self {
            Dialog::Confirm { question } => (question, vec![Line::from(vec![
                Span::styled(" No  n ", Style::new().fg(FG).bg(LINE)),
                Span::raw("  "),
                Span::styled(" Yes  y ", Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD)),
            ])]),
            Dialog::Pick { question, options, selected } => (question, options
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let style = if i == *selected { Style::new().fg(FG).bg(PANEL).add_modifier(Modifier::BOLD) } else { Style::new().fg(FG) };
                    Line::from(Span::styled(format!(" {} {o} ", if i == *selected { "›" } else { " " }), style))
                })
                .collect()),
        };
        let lines = crate::text::wrap(question, (width as usize).saturating_sub(6)).len() as u16;
        // Borders, a row of padding above and below, the question, a blank row, the body, and Pick's hint.
        let hint = u16::from(matches!(self, Dialog::Pick { .. }));
        let height = 2 + 2 + lines + 1 + body.len() as u16 + hint;
        let box_ = Rect { x: area.x + (area.width - width) / 2, y: area.y + area.height.saturating_sub(height) / 2, width, height };
        frame.render_widget(Clear, box_);
        let block = Block::new().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::new().fg(ORANGE)).style(Style::new().bg(SURFACE));
        let mut text = vec![Line::from(Span::styled(question.to_string(), Style::new().fg(FG))), Line::raw("")];
        text.extend(body);
        if matches!(self, Dialog::Pick { .. }) {
            text.push(Line::from(Span::styled("j/k move · enter picks · esc cancels", Style::new().fg(DIM))));
        }
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).block(block.padding(ratatui::widgets::Padding::new(2, 2, 1, 1))), box_);
    }
}
