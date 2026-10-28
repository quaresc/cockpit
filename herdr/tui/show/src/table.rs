//! A table: every column at its full width, scrolled sideways when the pane is narrow, numbers
//! right-aligned, zebra rows, a row cursor, and a header click (or 1-9) sorting by that column.

use std::cmp::Ordering;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier, Style};
use serde_json::Value;

use cockpit_core::text::width;
use cockpit_core::theme::{BG, DIM, FG, LINE, ORANGE, PANEL, SURFACE, fg};

use crate::row::{Act, Row, Target, slice};
use crate::value::{at, cell, csv, number, rows, text};

/// The rows in view at once, so the table and its header stand at most eighteen rows tall.
const SHOWN: usize = 17;
const CURSOR_BLURRED: Color = Color::Rgb(70, 56, 44);

pub struct Table {
    columns: Vec<Value>,
    rows: Vec<Vec<Value>>,
    order: Vec<usize>,
    sorted_by: Option<(usize, bool)>,
    numeric: Vec<bool>,
    widths: Vec<usize>,
    x: usize,
    cursor: usize,
    top: usize,
}

fn key_of(value: &Value) -> (bool, Option<f64>, String) {
    (value.is_null(), number(value), if value.is_string() { text(value).to_lowercase() } else { String::new() })
}

fn compare(a: &Value, b: &Value) -> Ordering {
    let ((an, af, at), (bn, bf, bt)) = (key_of(a), key_of(b));
    an.cmp(&bn).then_with(|| match (af, bf) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => at.cmp(&bt),
    })
}

impl Table {
    pub fn new(block: &Value) -> Self {
        let columns: Vec<Value> = block["columns"].as_array().cloned().unwrap_or_default();
        let rows = rows(block);
        let numeric: Vec<bool> = (0..columns.len()).map(|i| rows.iter().any(|r| number(at(r, i)).is_some())).collect();
        let widths = (0..columns.len()).map(|i| rows.iter().map(|r| width(&cell(at(r, i)))).chain([width(&text(&columns[i]))]).max().unwrap_or(0)).collect();
        Self { order: (0..rows.len()).collect(), columns, rows, sorted_by: None, numeric, widths, x: 0, cursor: 0, top: 0 }
    }

    pub fn csv(&self) -> String {
        csv(&self.columns, &self.rows)
    }

    fn sort(&mut self, i: usize) {
        let reverse = self.sorted_by == Some((i, false));
        self.sorted_by = Some((i, reverse));
        let rows = &self.rows;
        self.order.sort_by(|&a, &b| {
            let order = compare(at(&rows[a], i), at(&rows[b], i));
            if reverse { order.reverse() } else { order }
        });
    }

    fn piece(&self, i: usize, text: String, style: Style) -> (String, Style) {
        let pad = " ".repeat(self.widths[i].saturating_sub(width(&text)));
        (if self.numeric[i] { format!(" {pad}{text} ") } else { format!(" {text}{pad} ") }, style)
    }

    pub fn rows(&mut self, cols: usize, focused: bool) -> Vec<Row> {
        let total: usize = self.widths.iter().map(|w| w + 2).sum();
        let tall = self.rows.len() > SHOWN;
        let view = if tall { cols.saturating_sub(1) } else { cols };
        self.x = self.x.min(total.saturating_sub(view));
        let shown = self.rows.len().min(SHOWN);
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.top = self.top.min(self.cursor).max((self.cursor + 1).saturating_sub(shown));
        let bar = |n: usize| -> Option<(String, Style)> {
            tall.then(|| {
                let (len, start) = ((shown * shown / self.rows.len()).max(1), self.top * shown / self.rows.len());
                let thumb = n >= start && n < start + len;
                (if thumb { "┃" } else { "│" }.into(), fg(if thumb && focused { ORANGE } else { LINE }).bg(BG))
            })
        };
        let header = Style::new().fg(FG).bg(PANEL).add_modifier(Modifier::BOLD);
        let pieces: Vec<(String, Style)> = (0..self.columns.len()).map(|i| self.piece(i, text(&self.columns[i]), header)).collect();
        let mut head = Row::from_line(slice(&pieces, self.x, view, header)).target(Target::Table);
        let mut start = 0;
        for i in 0..self.columns.len() {
            let end = start + self.widths[i] + 2;
            let (a, b) = (start.saturating_sub(self.x).min(view), end.saturating_sub(self.x).min(view));
            if a < b {
                head.clicks.push((a, b, Act::Sort(i)));
            }
            start = end;
        }
        if let Some(piece) = bar(usize::MAX) {
            head.line.spans.push(ratatui::text::Span::styled(" ", piece.1));
        }
        let mut out = vec![head];
        for (n, k) in (self.top..self.top + shown).enumerate() {
            let r = &self.rows[self.order[k]];
            let zebra = if k % 2 == 1 { SURFACE } else { BG };
            let base = match (k == self.cursor, focused) {
                (true, true) => Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD),
                (true, false) => Style::new().fg(FG).bg(CURSOR_BLURRED),
                _ => Style::new().fg(FG).bg(zebra),
            };
            let pieces: Vec<(String, Style)> =
                (0..self.columns.len()).map(|i| self.piece(i, cell(at(r, i)), if at(r, i).is_null() && k != self.cursor { base.fg(DIM) } else { base })).collect();
            let mut row = Row::from_line(slice(&pieces, self.x, view, base)).target(Target::Table);
            row.clicks.push((0, view, Act::Cursor(k)));
            if let Some((text, style)) = bar(n) {
                row.line.spans.push(ratatui::text::Span::styled(text, style));
            }
            out.push(row);
        }
        if total > view {
            let len = (view * view / total).max(1);
            // Scrolled to the right edge, the thumb touches it rather than stopping a cell short.
            let start = if self.x + view >= total { view - len } else { self.x * view / total };
            let mut row = Row::new().target(Target::Table);
            row = row.add("─".repeat(start), fg(LINE));
            row = row.add("━".repeat(len), fg(if focused { ORANGE } else { DIM }));
            out.push(row.add("─".repeat(view.saturating_sub(start + len)), fg(LINE)));
        }
        out
    }

    pub fn act(&mut self, act: &Act) {
        match act {
            Act::Sort(i) => self.sort(*i),
            Act::Cursor(k) => self.cursor = *k,
            _ => {}
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => self.x = self.x.saturating_sub(4),
            KeyCode::Right | KeyCode::Char('l') => self.x += 4,
            KeyCode::Home => self.x = 0,
            KeyCode::End => self.x = usize::MAX,
            KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down => self.cursor += 1,
            KeyCode::Char(c @ '1'..='9') if (c as usize - '1' as usize) < self.columns.len() => self.sort(c as usize - '1' as usize),
            _ => return false,
        }
        true
    }
}
