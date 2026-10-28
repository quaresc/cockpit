//! Small pieces drawn the same way in every app: the scrollbar, sparklines and the filter bar.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme::{DIM, LINE, ORANGE};

const THUMB: ratatui::style::Color = ratatui::style::Color::Rgb(122, 98, 80);

/// A one-column scrollbar at the area's right edge: an orange-brown thumb on a faint track, nothing when all fits.
pub fn scrollbar(frame: &mut Frame, area: Rect, offset: usize, total: usize, visible: usize) {
    if total <= visible || area.height == 0 || area.width == 0 {
        return;
    }
    let height = area.height as usize;
    let size = (height * visible / total).max(1);
    let top = ((height - size) * offset / (total - visible).max(1)).min(height - size);
    let x = area.x + area.width - 1;
    for y in 0..height {
        let thumb = (top..top + size).contains(&y);
        let style = Style::new().fg(if thumb { THUMB } else { LINE });
        frame.render_widget(Paragraph::new(Span::styled(if thumb { "█" } else { "▕" }, style)), Rect { x, y: area.y + y as u16, width: 1, height: 1 });
    }
}

/// Values as eighth-height blocks, `width` cells wide, the last ones kept when there are more.
pub fn sparkline(values: &[f64], width: usize) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let values = &values[values.len().saturating_sub(width)..];
    let (lo, hi) = values.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    values
        .iter()
        .map(|&v| if hi > lo { BARS[(((v - lo) / (hi - lo)) * 7.0).round() as usize] } else { BARS[3] })
        .collect()
}

/// The "/ filter" bar shown while a filter is typed or kept, with how many rows match.
pub fn filter_line(input: &crate::input::Input, editing: bool, matched: usize) -> Line<'static> {
    let mut line = if editing { input.line("/ ") } else { Line::from(vec![Span::styled("/ ", Style::new().fg(ORANGE)), Span::raw(input.text.clone())]) };
    line.spans.push(Span::styled(format!("   {matched} match{}", if matched == 1 { "" } else { "es" }), Style::new().fg(DIM)));
    if !editing {
        line.spans.push(Span::styled("   esc clears", Style::new().fg(DIM)));
    }
    line
}
