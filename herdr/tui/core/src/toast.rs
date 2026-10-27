//! Short notes stacked in the bottom right corner, gone after a few seconds.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::theme::{FG, GREEN, ORANGE, PANEL, RED};

const SHOWN: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
pub enum Kind {
    Note,
    Good,
    Bad,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<(String, Kind, Instant)>,
}

impl Toasts {
    pub fn push(&mut self, text: impl Into<String>, kind: Kind) {
        let text = text.into();
        if !text.is_empty() {
            self.items.push((text, kind, Instant::now()));
        }
    }

    /// True while a toast shows, for the loop to redraw when it goes.
    pub fn any(&mut self) -> bool {
        self.items.retain(|(_, _, at)| at.elapsed() < SHOWN);
        !self.items.is_empty()
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.any();
        let width = 48.min(area.width.saturating_sub(2));
        let mut bottom = area.y + area.height.saturating_sub(1);
        for (text, kind, _) in self.items.iter().rev() {
            let lines = crate::text::wrap(text, width as usize - 4);
            let height = lines.len() as u16;
            if bottom < area.y + height + 1 {
                break;
            }
            let rect = Rect { x: area.x + area.width - width - 1, y: bottom - height - 1, width, height };
            let edge: Color = match kind {
                Kind::Note => ORANGE,
                Kind::Good => GREEN,
                Kind::Bad => RED,
            };
            frame.render_widget(Clear, rect);
            let body: Vec<Line> = lines.into_iter().map(|l| Line::from(Span::styled(l, Style::new().fg(FG)))).collect();
            let block = Block::new().borders(Borders::LEFT).border_style(Style::new().fg(edge)).style(Style::new().bg(PANEL));
            frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }).block(block.padding(ratatui::widgets::Padding::horizontal(1))), rect);
            bottom -= height + 1;
        }
    }
}
