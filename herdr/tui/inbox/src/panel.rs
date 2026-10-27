//! What frames the tree: the header with its error lines, the highlighted row's details and buttons under it, and
//! the footer of key hints.

use cockpit_core::acts::{self, Act};
use cockpit_core::text::{width, wrap};
use cockpit_core::theme::{FG, ORANGE, PANEL, bold, fg};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

const MAX_PANEL: u16 = 12;
const KEYS: [(&str, &str); 5] = [("q", "quit"), ("g", "group by"), ("R", "refresh"), ("space", "fold"), ("enter", "open")];

fn plain(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// How many screen lines these lines wrap to.
pub fn height(lines: &[Line], cells: u16) -> u16 {
    lines.iter().map(|l| wrap(&plain(l), cells as usize).len() as u16).sum()
}

pub fn header(frame: &mut Frame, area: Rect, head: &[Line<'static>]) {
    frame.render_widget(Paragraph::new(head.to_vec()).wrap(Wrap { trim: false }), inset(area));
}

/// The panel's height for these details, the rule and the buttons' line included.
pub fn panel_height(info: &[Line], acts: &[Act], cells: u16) -> u16 {
    (1 + height(info, cells.saturating_sub(2)) + 1 + u16::from(!acts.is_empty())).min(MAX_PANEL)
}

/// The details and the buttons; each button's place, to know what a click pressed.
pub fn panel(frame: &mut Frame, area: Rect, info: &[Line<'static>], acts: &[Act]) -> Vec<(Rect, usize)> {
    frame.render_widget(Paragraph::new("─".repeat(area.width as usize)).style(Style::new().fg(PANEL)), Rect { height: 1, ..area });
    let body = inset(Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area });
    let buttons_y = body.y + body.height.saturating_sub(1);
    let text_area = Rect { height: body.height.saturating_sub(u16::from(!acts.is_empty()) + 1), ..body };
    frame.render_widget(Paragraph::new(info.to_vec()).wrap(Wrap { trim: false }), text_area);
    if acts.is_empty() {
        return Vec::new();
    }
    let bar = Rect { y: buttons_y, height: 1, ..body };
    frame.render_widget(Paragraph::new(acts::buttons(acts)), bar);
    let mut x = bar.x;
    acts.iter()
        .enumerate()
        .map(|(n, act)| {
            let key = act.key.map(|k| format!("  {k}")).unwrap_or_default();
            let cells = width(&format!(" {}{key} ", act.label)) as u16;
            let rect = Rect { x, y: buttons_y, width: cells, height: 1 };
            x += cells + 1;
            (rect, n)
        })
        .collect()
}

pub fn footer(frame: &mut Frame, area: Rect) {
    let palette = vec![Span::styled("^p ", bold(ORANGE)), Span::styled("palette ", fg(FG))];
    let mut spans = vec![Span::raw(" ")];
    // A narrow pane drops the last hints rather than cutting the palette's.
    let mut room = (area.width as usize).saturating_sub(1 + palette.iter().map(|s| width(&s.content)).sum::<usize>());
    for (key, what) in KEYS {
        let hint = [Span::styled(format!("{key} "), bold(ORANGE)), Span::styled(format!("{what}  "), fg(FG))];
        let cells: usize = hint.iter().map(|s| width(&s.content)).sum();
        if cells > room {
            break;
        }
        room -= cells;
        spans.extend(hint);
    }
    let used: usize = spans.iter().chain(&palette).map(|s| width(&s.content)).sum();
    spans.push(Span::raw(" ".repeat((area.width as usize).saturating_sub(used))));
    spans.extend(palette);
    frame.render_widget(Paragraph::new(Line::from(spans)).style(Style::new().bg(PANEL)), area);
}

/// The area less one cell each side, Textual's `padding: 0 1`.
pub fn inset(area: Rect) -> Rect {
    Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area }
}
