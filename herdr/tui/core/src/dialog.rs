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
            Dialog::Confirm { question } => (
                question,
                vec![Line::from(vec![
                    Span::styled(" No  n ", Style::new().fg(FG).bg(LINE)),
                    Span::raw("  "),
                    Span::styled(" Yes  y ", Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD)),
                ])],
            ),
            Dialog::Pick { question, options, selected } => (
                question,
                options
                    .iter()
                    .enumerate()
                    .map(|(i, o)| {
                        let style = if i == *selected { Style::new().fg(FG).bg(PANEL).add_modifier(Modifier::BOLD) } else { Style::new().fg(FG) };
                        Line::from(Span::styled(format!(" {} {o} ", if i == *selected { "›" } else { " " }), style))
                    })
                    .collect(),
            ),
        };
        let lines = crate::text::wrap(question, (width as usize).saturating_sub(6)).len() as u16;
        // Borders, a row of padding above and below, the question, a blank row, the body, and Pick's hint.
        let hint = u16::from(matches!(self, Dialog::Pick { .. }));
        // Never taller than the pane: a dialog whose answer falls off the bottom cannot be answered, and a confirm
        // that cannot be answered blocks the app. A pane too short for the padding, then the border, gives them up.
        let wanted = 2 + 2 + lines + 1 + body.len() as u16 + hint;
        let height = wanted.min(area.height);
        let pad = u16::from(height >= wanted.min(6));
        let bordered = height >= 3;
        let box_ = Rect { x: area.x + (area.width - width) / 2, y: area.y + area.height.saturating_sub(height) / 2, width, height };
        frame.render_widget(Clear, box_);
        let borders = if bordered { Borders::ALL } else { Borders::NONE };
        let block = Block::new().borders(borders).border_type(BorderType::Rounded).border_style(Style::new().fg(ORANGE)).style(Style::new().bg(SURFACE));
        let mut text = vec![Line::from(Span::styled(question.to_string(), Style::new().fg(FG))), Line::raw("")];
        text.extend(body);
        if matches!(self, Dialog::Pick { .. }) {
            text.push(Line::from(Span::styled("j/k move · enter picks · esc cancels", Style::new().fg(DIM))));
        }
        // What a shorter box cut off is scrolled past, so the last rows — the answer — are the ones kept.
        let chrome = u16::from(bordered) * 2 + pad * 2;
        let room = height.saturating_sub(chrome);
        let inner = width.saturating_sub(chrome + 4);
        let scroll = (crate::text::wrapped(&text, inner) as u16).saturating_sub(room);
        let padding = ratatui::widgets::Padding::new(2, 2, pad, pad);
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).scroll((scroll, 0)).block(block.padding(padding)), box_);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn drawn(dialog: &Dialog, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal.draw(|frame| dialog.render(frame, frame.area())).expect("a frame");
        let buffer = terminal.backend().buffer();
        (0..height).map(|y| (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>()).collect()
    }

    #[test]
    fn a_confirm_keeps_its_answer_on_screen_however_short_the_pane() {
        let question = "Stop the preview pv-quotas, which three people are looking at, and free its environment?";
        let dialog = Dialog::Confirm { question: question.to_string() };
        for height in [3, 5, 8, 20] {
            let screen = drawn(&dialog, 60, height);
            assert!(screen.iter().any(|l| l.contains("Yes")), "the answer is drawn at {height} rows:\n{}", screen.join("\n"));
            assert_eq!(screen.len(), height as usize, "and nothing is drawn past the pane");
        }
    }
}
