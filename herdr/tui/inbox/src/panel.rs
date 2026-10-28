//! What frames the tree: the header with its error lines, the filter bar, the highlighted row's details and buttons
//! (or the picked rows' shared ones) under it, and the footer of key hints.

use cockpit_core::acts::{self, Act};
use cockpit_core::text::{width, wrap};
use cockpit_core::theme::{DIM, FG, ORANGE, PANEL, bold, fg};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::tree::pill_width;

const MAX_PANEL: u16 = 24;
const KEYS: [(&str, &str); 7] = [("q", "quit"), ("/", "filter"), ("v", "pick"), ("w", "group by"), ("R", "refresh"), ("space", "fold"), ("enter", "main")];
const PICKING: [(&str, &str); 3] = [("v", "pick"), ("V", "all alike"), ("esc", "drop picks")];

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
    places(acts, bar.x, buttons_y)
}

/// Each button's cells on a line of buttons starting at `x`.
fn places(acts: &[Act], mut x: u16, y: u16) -> Vec<(Rect, usize)> {
    acts.iter()
        .enumerate()
        .map(|(n, act)| {
            let cells = pill_width(act) as u16;
            let rect = Rect { x, y, width: cells - 1, height: 1 };
            x += cells;
            (rect, n)
        })
        .collect()
}

/// The picked rows' summary for the panel: how many of what, then their names.
pub fn picked_info(summary: &str, names: &[String], shared: bool) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![Span::styled(format!("{summary} picked"), bold(ORANGE)), Span::styled("   esc drops them", fg(DIM))])];
    const NAMES: usize = 5;
    lines.extend(names.iter().take(NAMES).map(|n| Line::styled(format!("  {n}"), fg(DIM))));
    if names.len() > NAMES {
        lines.push(Line::styled(format!("  and {} more", names.len() - NAMES), fg(DIM)));
    }
    if !shared {
        lines.push(Line::styled("no action is shared by all of them", fg(DIM)));
    }
    lines
}

/// Key hints, the palette's at the right; a narrow pane drops the last hints rather than cutting the palette's.
pub fn footer(frame: &mut Frame, area: Rect, picking: bool) {
    let palette = vec![Span::styled("^p ", bold(ORANGE)), Span::styled("palette ", fg(FG))];
    let mut spans = vec![Span::raw(" ")];
    let mut room = (area.width as usize).saturating_sub(1 + palette.iter().map(|s| width(&s.content)).sum::<usize>());
    let keys: &[(&str, &str)] = if picking { &PICKING } else { &KEYS };
    for (key, what) in keys {
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

/// The compact layout's footer while rows are picked: how many, then the buttons they share.
pub fn picked_footer(frame: &mut Frame, area: Rect, summary: &str, acts: &[Act]) -> Vec<(Rect, usize)> {
    let lead = format!(" {summary} picked  ");
    let mut line = acts::buttons(acts);
    line.spans.insert(0, Span::styled(lead.clone(), bold(ORANGE)));
    frame.render_widget(Paragraph::new(line).style(Style::new().bg(PANEL)), area);
    places(acts, area.x + width(&lead) as u16, area.y)
}

/// The area less one cell each side.
pub fn inset(area: Rect) -> Rect {
    Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area }
}
