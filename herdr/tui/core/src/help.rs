//! The keys overlay every space opens with `?`: the actions the row under the cursor offers, how to move, and the
//! chords that jump between spaces. Built from the app's own actions and herdr's config, never from a hand-written
//! list, so it cannot drift from what the keys actually do — and it lists no verb the app does not have.

use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::actions;
use crate::acts::Act;
use crate::text::width;
use crate::theme::{DIM, FG, LINE, ORANGE, SURFACE, bold, dim, fg};

const MOVING: [(&str, &str); 6] =
    [("j k", "move"), ("↑ ↓", "move"), ("g G", "first, last"), ("[ ]", "previous, next group"), ("^d ^u", "page"), ("q", "leave")];

// One space runs per process, so the overlay is one flag rather than a field threaded through every app.
static OPEN: AtomicBool = AtomicBool::new(false);

pub fn is_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

/// `?` opens the overlay; in it, `m` hands the mouse to the terminal and back, and any other key closes it. True
/// when the key was the overlay's, so the app leaves it alone. The mouse lives here because every letter an app
/// could bind is already taken — `M` merges a pull request in the inbox.
pub fn key(key: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::{KeyCode, KeyModifiers};
    if is_open() {
        if key.code == KeyCode::Char('m') {
            crate::term::toggle_mouse();
        } else {
            OPEN.store(false, Ordering::Relaxed);
        }
        return true;
    }
    if key.code == KeyCode::Char('?') && !key.modifiers.contains(KeyModifiers::CONTROL) {
        OPEN.store(true, Ordering::Relaxed);
        return true;
    }
    false
}

/// A heading, then its rows, as the overlay lays them out.
fn section(title: &str, rows: &[(String, String)], key_col: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::raw(""), Line::from(Span::styled(title.to_string(), bold(ORANGE)))];
    for (key, what) in rows {
        lines.push(Line::from(vec![Span::raw("  "), Span::styled(format!("{key:<key_col$}"), fg(FG)), Span::raw("  "), Span::styled(what.clone(), dim())]));
    }
    lines
}

/// The overlay's lines for this app: its own actions first, then what every app shares.
pub fn lines(app: &str, acts: &[Act]) -> Vec<Line<'static>> {
    let own: Vec<(String, String)> = acts.iter().filter_map(|a| a.key.map(|k| (k.to_string(), a.label.clone()))).collect();

    let moving: Vec<(String, String)> = MOVING.iter().map(|(k, w)| (k.to_string(), w.to_string())).collect();
    let mut spaces: Vec<(String, String)> = actions::space_keys().into_iter().map(|(space, key)| (key, format!("the {space} space"))).collect();
    spaces.sort_by(|a, b| a.1.cmp(&b.1));
    spaces.push(("^p".into(), "everything this app can do".into()));

    let col = [&own, &moving, &spaces].iter().flat_map(|rows| rows.iter()).map(|(k, _)| width(k)).max().unwrap_or(3);
    let mut lines = vec![Line::from(Span::styled(format!("Keys in {app}"), bold(FG)))];
    if !own.is_empty() {
        lines.extend(section("This row", &own, col));
    }
    lines.extend(section("Moving", &moving, col));
    let mouse =
        if crate::term::mouse_is_off() { "take the mouse back (it is the terminal's)" } else { "give the mouse to the terminal, to select and copy text" };
    lines.extend(section("While this is open", &[("m".to_string(), mouse.to_string())], col));
    if !spaces.is_empty() {
        lines.extend(section(if spaces.len() > 1 { "Spaces" } else { "Find" }, &spaces, col));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled("  any key closes this", Style::new().fg(DIM).add_modifier(Modifier::ITALIC))));
    lines
}

/// The lines laid out in two columns, for a pane too short for one. Sections are kept whole, split at the boundary
/// that leaves the two halves most even.
fn two_columns(text: Vec<Line<'static>>, gap: usize) -> Vec<Line<'static>> {
    let heads: Vec<usize> = text
        .iter()
        .enumerate()
        .filter(|(i, l)| *i > 0 && l.spans.len() == 1 && !l.spans[0].content.trim().is_empty() && !l.spans[0].content.starts_with("  "))
        .map(|(i, _)| i)
        .collect();
    let half = text.len() / 2;
    // Each section opens with a blank line, then its heading: cut on the blank, so the heading leads the column.
    let Some(at) = heads.iter().map(|h| h.saturating_sub(1)).min_by_key(|i| i.abs_diff(half)) else { return text };
    let (left, right) = text.split_at(at);
    let right = right.iter().skip(1).cloned().collect::<Vec<_>>();
    let wide = left.iter().map(|l| l.spans.iter().map(|s| width(&s.content)).sum::<usize>()).max().unwrap_or(0);
    (0..left.len().max(right.len()))
        .map(|i| {
            let mut spans: Vec<Span<'static>> = left.get(i).map(|l| l.spans.clone()).unwrap_or_default();
            let used: usize = spans.iter().map(|s| width(&s.content)).sum();
            spans.push(Span::raw(" ".repeat(wide + gap - used.min(wide + gap))));
            spans.extend(right.get(i).map(|l| l.spans.clone()).unwrap_or_default());
            Line::from(spans)
        })
        .collect()
}

/// Draws the overlay over the pane, as tall as it needs and no taller: too tall for one column, it folds into two.
pub fn render(frame: &mut Frame, area: Rect, app: &str, acts: &[Act]) {
    let mut text = lines(app, acts);
    if text.len() as u16 + 2 > area.height {
        text = two_columns(text, 4);
    }
    let text = text;
    let wide = text.iter().map(|l| l.spans.iter().map(|s| width(&s.content)).sum::<usize>()).max().unwrap_or(20);
    let width = ((wide + 6) as u16).min(area.width);
    let height = (text.len() as u16 + 2).min(area.height);
    let box_ = Rect { x: area.x + (area.width.saturating_sub(width)) / 2, y: area.y + (area.height.saturating_sub(height)) / 2, width, height };
    frame.render_widget(Clear, box_);
    let block = Block::new()
        .borders(if height >= 3 { Borders::ALL } else { Borders::NONE })
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(LINE))
        .style(Style::new().bg(SURFACE))
        .padding(ratatui::widgets::Padding::new(2, 2, 0, 0));
    frame.render_widget(Paragraph::new(text).block(block), box_);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_is_built_from_the_keys_the_app_really_has() {
        let acts = vec![Act::new("join", 'm', "join"), Act::new("brief", 'b', "brief")];
        let text: Vec<String> = lines("meetings", &acts).iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect();
        let all = text.join("\n");
        assert!(all.contains("Keys in meetings"));
        assert!(all.contains("join") && all.contains("brief"), "the row's own actions:\n{all}");
        assert!(all.contains("move") && all.contains("leave"), "how to move:\n{all}");
        assert!(all.contains("While this is open") && all.contains("select and copy text"), "the mouse:\n{all}");
        // Only what this app really has: a verb it does not implement would be a lie. `/` filters in three of the
        // seven apps, so it is listed for those and for no other.
        assert!(!all.contains("filter"), "no verb the app does not have:\n{all}");
    }
}
