//! A page of lines in place of the search: the doctor's report as it runs, or today's brief, each line with its tone
//! and, for the brief, the link Enter opens.

use std::sync::mpsc::{Receiver, channel};

use cockpit_core::keys::Move;
use cockpit_core::term;
use cockpit_core::theme::{DIM, FG, GREEN, ORANGE, PANEL, RED, SURFACE, bold, fg};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub mark: &'static str,
    pub color: Color,
    pub text: String,
    pub url: Option<String>,
}

fn row(mark: &'static str, color: Color, text: impl Into<String>) -> Row {
    Row { mark, color, text: text.into(), url: None }
}

pub struct Report {
    title: String,
    rows: Vec<Row>,
    state: ListState,
    running: Option<Receiver<Vec<Row>>>,
}

impl Report {
    /// `cockpit doctor` run in the background by this same binary, its lines shown once it is done.
    pub fn doctor() -> Report {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let exe = std::env::current_exe().unwrap_or_else(|_| "cockpit".into());
            let rows = match std::process::Command::new(exe).arg("doctor").stdin(std::process::Stdio::null()).output() {
                Ok(out) => doctor_rows(&String::from_utf8_lossy(&out.stdout)),
                Err(error) => vec![row("✗", RED, format!("cockpit doctor did not run: {error}"))],
            };
            let _ = tx.send(rows);
            term::redraw();
        });
        Report { title: "Cockpit doctor".into(), rows: vec![row("…", DIM, "checking every part of cockpit")], state: ListState::default(), running: Some(rx) }
    }

    /// The brief the mod wrote this morning; an older one says its date.
    pub fn brief(brief: &Value, today: &str) -> Report {
        let date = brief["date"].as_str().unwrap_or_default();
        let lines = brief["lines"].as_array().cloned().unwrap_or_default();
        let mut rows: Vec<Row> = lines
            .iter()
            .map(|l| {
                let color = match l["tone"].as_str() {
                    Some("warn") => ORANGE,
                    Some("bad") => RED,
                    Some("good") => GREEN,
                    _ => FG,
                };
                Row { url: l["url"].as_str().filter(|u| !u.is_empty()).map(str::to_string), ..row("•", color, l["text"].as_str().unwrap_or_default()) }
            })
            .collect();
        let title = if date == today || date.is_empty() { "Today's brief".to_string() } else { format!("The last brief, of {date}") };
        if rows.is_empty() {
            rows.push(row("·", DIM, "No brief yet: the first Claude session of the day with cockpit writes it."));
        }
        let mut state = ListState::default();
        state.select(Some(0));
        Report { title, rows, state, running: None }
    }

    /// Takes in the doctor's lines once it is done; true when they came.
    pub fn pump(&mut self) -> bool {
        let Some(rx) = &self.running else { return false };
        let Ok(rows) = rx.try_recv() else { return false };
        self.rows = rows;
        self.running = None;
        self.state.select(Some(0));
        true
    }

    pub fn selected_url(&self) -> Option<String> {
        self.rows.get(self.state.selected()?)?.url.clone()
    }

    pub fn go(&mut self, movement: Move) {
        let last = self.rows.len().saturating_sub(1);
        let at = self.state.selected().unwrap_or(0);
        let to = match movement {
            Move::By(n) | Move::Group(n) => at.saturating_add_signed(n).min(last),
            Move::Pages(n) => at.saturating_add_signed(n * 10).min(last),
            Move::Top => 0,
            Move::Bottom => last,
        };
        self.state.select(Some(to));
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Clear, area);
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(fg(ORANGE))
            .title(Span::styled(format!(" {} ", self.title), bold(ORANGE)))
            .style(Style::new().bg(SURFACE));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let cells = (inner.width as usize).saturating_sub(3).max(10);
        let items: Vec<ListItem> = self
            .rows
            .iter()
            .map(|r| {
                let mut lines: Vec<Line> = textwrap::wrap(&r.text, cells)
                    .into_iter()
                    .enumerate()
                    .map(|(n, part)| {
                        let mark = if n == 0 { format!("{} ", r.mark) } else { "  ".into() };
                        Line::from(vec![Span::styled(mark, fg(r.color)), Span::styled(part.into_owned(), fg(FG))])
                    })
                    .collect();
                if let Some(url) = &r.url {
                    lines.push(Line::from(Span::styled(format!("  {url}"), fg(DIM))));
                }
                ListItem::new(lines)
            })
            .collect();
        let list = List::new(items).highlight_style(Style::new().bg(PANEL).add_modifier(Modifier::BOLD));
        frame.render_stateful_widget(list, inner, &mut self.state);
    }
}

/// The doctor's "ok    …", "warn  …" and "fail  …" lines, failures first, then warnings, then the rest as they came.
pub fn doctor_rows(out: &str) -> Vec<Row> {
    let mut rows: Vec<(u8, Row)> = out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| match l.split_once(' ') {
            Some(("fail", rest)) => (0, row("✗", RED, rest.trim())),
            Some(("warn", rest)) => (1, row("!", ORANGE, rest.trim())),
            Some(("ok", rest)) => (2, row("✓", GREEN, rest.trim())),
            _ => (2, row(" ", DIM, l.trim())),
        })
        .collect();
    rows.sort_by_key(|(rank, _)| *rank);
    if rows.is_empty() {
        return vec![row("✗", RED, "cockpit doctor printed nothing")];
    }
    rows.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_doctor_puts_failures_first() {
        let rows = doctor_rows("ok    herdr 0.9.5\nwarn  no Claude session with cockpit is running\nfail  jq is missing: brew install jq\n");
        let marks: Vec<(&str, &str)> = rows.iter().map(|r| (r.mark, r.text.as_str())).collect();
        assert_eq!(marks, [("✗", "jq is missing: brew install jq"), ("!", "no Claude session with cockpit is running"), ("✓", "herdr 0.9.5")]);
        assert_eq!(doctor_rows("").len(), 1);
    }

    #[test]
    fn the_brief_says_when_it_is_not_todays() {
        let brief = json!({"date": "2026-10-27", "lines": [{"icon": "calendar", "text": "2 meetings, first at 09:45: Team daily", "url": "https://meet.test/x"}, {"icon": "ci", "text": "api#4 fails CI", "tone": "bad"}]});
        let report = Report::brief(&brief, "2026-10-28");
        assert_eq!(report.title, "The last brief, of 2026-10-27");
        assert_eq!((report.rows[1].color, report.selected_url()), (RED, Some("https://meet.test/x".to_string())));
        assert_eq!(Report::brief(&Value::Null, "2026-10-28").rows[0].mark, "·");
    }
}
