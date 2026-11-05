//! The frame: the header, the list grouped Unread then Read, the selected message's preview (its thread once read
//! whole, else its snippet) beside or under it, the actions and keys at the foot, and the toasts over them.

use chrono::{DateTime, Local, Utc};
use cockpit_core::acts;
use cockpit_core::text::{fit, width, wrap};
use cockpit_core::theme::{FG, LINE, ORANGE, PANEL, RED, base, bold, dim, fg};
use cockpit_core::widgets;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::app::App;
use crate::model::{GROUPS, Group, Message, age};
use crate::thread::Thread;

const ICON_MAIL: &str = "\u{f42f}";
const NAME: usize = 18;
const AGE: usize = 4;
/// From this width the preview sits beside the list, under it below.
const SIDE_BY_SIDE: u16 = 110;

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        self.take_replies();
        let area = frame.area();
        frame.render_widget(Block::new().style(base()), area);
        self.hits = Default::default();
        let head = self.head_lines();
        let [head_area, main, foot] = Layout::vertical([Constraint::Length(head.len() as u16 + 1), Constraint::Min(3), Constraint::Length(1)]).areas(area);
        frame.render_widget(Paragraph::new(head), head_area.inner(ratatui::layout::Margin::new(1, 0)));
        let [list, preview] = if area.width >= SIDE_BY_SIDE {
            Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(main)
        } else {
            Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(main)
        };
        self.draw_list(frame, list);
        self.draw_preview(frame, preview, area.width >= SIDE_BY_SIDE);
        self.draw_foot(frame, foot);
        self.toasts.render(frame, area);
    }

    fn head_lines(&self) -> Vec<Line<'static>> {
        let updated = self.mail.at.and_then(DateTime::from_timestamp_millis).map(|at| format!("updated {}", at.with_timezone(&Local).format("%H:%M")));
        let count = match self.mail.unread() {
            0 => "nothing unread".to_string(),
            n => format!("{n} unread"),
        };
        let mut line = vec![Span::styled(format!("{ICON_MAIL}  Mail"), bold(ORANGE)), Span::styled(format!("   {count}"), fg(FG))];
        if let Some(updated) = updated {
            line.push(Span::styled(format!(" · {updated}"), dim()));
        }
        let mut lines = vec![Line::from(line)];
        if let Some(error) = &self.mail.error {
            lines.push(Line::styled(error.clone(), fg(RED)));
        }
        lines
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect) {
        self.hits.list = area;
        let inner = Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area };
        let w = inner.width as usize;
        if self.mail.messages.is_empty() {
            let text =
                if self.mail.at.is_none() { "No mail yet: the hub or a Claude Code session with the mail connector fills it in." } else { "Inbox zero." };
            frame.render_widget(Paragraph::new(Line::styled(text, dim())), inner);
            return;
        }
        let now = Utc::now();
        // Each line: its text, and the message it is the row of.
        let mut rows: Vec<(Line<'static>, Option<String>)> = Vec::new();
        for (kind, _, label) in GROUPS {
            let group: Vec<&Message> = self.mail.messages.iter().filter(|m| m.group() == kind).collect();
            if group.is_empty() {
                continue;
            }
            if !rows.is_empty() {
                rows.push((Line::raw(""), None));
            }
            let style = if kind == Group::Unread { bold(ORANGE) } else { dim() };
            rows.push((Line::from(vec![Span::styled(label, style), Span::styled(format!("  {}", group.len()), dim())]), None));
            for m in group {
                rows.push((row_line(m, self.selected.as_deref() == Some(m.id.as_str()), w, now), Some(m.id.clone())));
            }
        }
        let height = inner.height as usize;
        let at = rows.iter().position(|(_, id)| id.is_some() && *id == self.selected).unwrap_or(0);
        if self.follow {
            if at < self.scroll {
                self.scroll = at.saturating_sub(1);
            } else if at >= self.scroll + height {
                self.scroll = at + 1 - height;
            }
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
        for (i, (line, id)) in rows.iter().skip(self.scroll).take(height).enumerate() {
            let rect = Rect { y: inner.y + i as u16, height: 1, ..inner };
            let selected = id.is_some() && *id == self.selected;
            let style = if selected { Style::new().bg(PANEL) } else { Style::new() };
            frame.render_widget(Paragraph::new(line.clone()).style(style), rect);
            if let Some(id) = id {
                self.hits.rows.push((rect, id.clone()));
            }
        }
        widgets::scrollbar(frame, area, self.scroll, rows.len(), height);
    }

    fn draw_preview(&mut self, frame: &mut Frame, area: Rect, beside: bool) {
        let block = if beside { Block::new().borders(ratatui::widgets::Borders::LEFT) } else { Block::new().borders(ratatui::widgets::Borders::TOP) };
        let block = block.border_style(fg(LINE));
        let inner = block.inner(area).inner(ratatui::layout::Margin::new(1, 0));
        frame.render_widget(block, area);
        self.hits.body = inner;
        let Some(m) = self.selected_message().cloned() else { return };
        let w = inner.width as usize;
        let mut lines = vec![Line::styled(fit(m.title(), w), bold(FG)), Line::styled(fit(&format!("{} <{}>", m.name, m.from), w), dim()), Line::raw("")];
        match &self.thread {
            Thread::Ready(_) => lines.extend(self.body_lines(&m.id, w)),
            Thread::Loading(at) => {
                let secs = (Utc::now().timestamp_millis() as u64).saturating_sub(*at) / 1000;
                let wait = if secs > 30 { format!("Loading… ({secs}s)") } else { "Loading…".to_string() };
                lines.push(Line::styled(wait, fg(ORANGE)));
                lines.extend(wrap(&m.snippet, w).into_iter().map(|l| Line::styled(l, dim())));
            }
            Thread::Failed(error) => {
                lines.push(Line::styled(fit(error, w), fg(RED)));
                lines.push(Line::styled("⏎ tries again", dim()));
            }
            Thread::Unread => {
                lines.extend(wrap(&m.snippet, w).into_iter().map(Line::raw));
                lines.push(Line::raw(""));
                lines.push(Line::styled("⏎ reads it whole · h in the browser beside · o in the mail client", dim()));
            }
        }
        let height = inner.height as usize;
        self.body_scroll = self.body_scroll.min(lines.len().saturating_sub(height));
        let shown: Vec<Line> = lines.iter().skip(self.body_scroll).take(height).cloned().collect();
        frame.render_widget(Paragraph::new(shown), inner);
        widgets::scrollbar(frame, area, self.body_scroll, lines.len(), height);
    }

    /// The thread's messages as lines for this width: each under its sender and date, its text wrapped.
    fn body_lines(&mut self, id: &str, w: usize) -> Vec<Line<'static>> {
        if let Some((for_id, for_w, lines)) = &self.body
            && for_id == id
            && *for_w == w
        {
            return lines.clone();
        }
        let Thread::Ready(messages) = &self.thread else { return Vec::new() };
        let mut lines = Vec::new();
        for (i, m) in messages.iter().enumerate() {
            if i > 0 {
                lines.push(Line::styled("─".repeat(w.min(60)), fg(LINE)));
            }
            let when = DateTime::parse_from_rfc3339(&m.date).map(|d| d.with_timezone(&Local).format("%a %-d %b %H:%M").to_string()).unwrap_or(m.date.clone());
            lines.push(Line::from(vec![Span::styled(m.name.clone(), bold(ORANGE)), Span::styled(format!("  {when}"), dim())]));
            for paragraph in m.body(w.max(20)).lines() {
                if paragraph.is_empty() {
                    lines.push(Line::raw(""));
                }
                lines.extend(wrap(paragraph, w).into_iter().map(Line::raw));
            }
            lines.push(Line::raw(""));
        }
        self.body = Some((id.to_string(), w, lines.clone()));
        lines
    }

    fn draw_foot(&mut self, frame: &mut Frame, area: Rect) {
        let acts = self.acts();
        let mut spans = Vec::new();
        let mut x = area.x + 1;
        for (act, cmd) in &acts {
            let line = acts::buttons(std::slice::from_ref(act));
            let w = line.width() as u16;
            if x + w > area.x + area.width {
                break;
            }
            self.hits.buttons.push((Rect { x, y: area.y, width: w.saturating_sub(1), height: 1 }, cmd.clone()));
            x += w;
            spans.extend(line.spans);
        }
        let hints = " j k move · ] [ group · ctrl+d/u scroll · R refresh · q quit";
        if (x - area.x) as usize + width(hints) <= area.width as usize {
            spans.push(Span::styled(hints, dim()));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), Rect { x: area.x + 1, width: area.width.saturating_sub(1), ..area });
    }
}

/// A message's row: the unread mark, the sender, the subject and its snippet, the age.
fn row_line(m: &Message, selected: bool, w: usize, now: DateTime<Utc>) -> Line<'static> {
    let mark = if m.unread { Span::styled("● ", fg(ORANGE)) } else { Span::raw("  ") };
    let name_style = if m.unread { Style::new().fg(FG).add_modifier(Modifier::BOLD) } else { dim() };
    let rest = w.saturating_sub(2 + NAME + 2 + AGE + 1);
    let subject = fit(m.title(), rest).trim_end().to_string();
    let snippet_room = rest.saturating_sub(width(&subject) + 3);
    let snippet = if snippet_room > 4 && !m.snippet.is_empty() { format!(" — {}", fit(&m.snippet, snippet_room).trim_end()) } else { String::new() };
    let used = 2 + NAME + 2 + width(&subject) + width(&snippet);
    let pad = w.saturating_sub(used + AGE);
    let subject_style = if m.unread || selected { fg(FG) } else { dim() };
    Line::from(vec![
        mark,
        Span::styled(format!("{:<NAME$}", fit(&m.name, NAME)), name_style),
        Span::raw("  "),
        Span::styled(subject, subject_style),
        Span::styled(snippet, dim()),
        Span::raw(" ".repeat(pad)),
        Span::styled(format!("{:>AGE$}", age(m.date, now)), dim()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    fn screen(app: &mut App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..h).map(|y| (0..w).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn the_list_groups_and_the_preview_reads_the_thread() {
        let mut app = App::with(json!({"mail": {"tools": {"archive": {"tool": "archive_thread"}}}}), false);
        app.set_mail(&json!({"messages": [
            {"id": "a", "from": "noreply@ci.example.com", "fromName": "CI", "subject": "Build passed", "snippet": "All green", "date": "2026-11-04T10:00:00Z", "unread": true},
            {"id": "b", "from": "maya@example.com", "fromName": "Maya", "subject": "Lunch?", "snippet": "Noon at the usual place", "date": "2026-11-04T08:00:00Z", "unread": true}
        ]}));
        let text = screen(&mut app, 120, 16);
        let unread = text.find("Unread").unwrap();
        assert!(unread < text.find("Build passed").unwrap() && text.find("Build passed").unwrap() < text.find("Maya").unwrap(), "{text}");
        assert!(text.contains("⏎ reads it whole"), "{text}");
        assert!(text.contains("archive  b"), "{text}");
        app.thread = crate::thread::parse(
            &json!({"messages": [{"from": "maya@example.com", "fromName": "Maya", "date": "2026-11-04T08:00:00Z", "html": "<p>Noon at <b>the usual place</b>?</p>"}]}),
        );
        let text = screen(&mut app, 120, 16);
        assert!(text.contains("Noon at the usual place?"), "{text}");
        let narrow = screen(&mut app, 70, 24);
        assert!(narrow.find("Build passed").unwrap() < narrow.find("Noon at the usual place?").unwrap(), "the preview goes under the list:\n{narrow}");
    }
}
