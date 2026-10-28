//! The steps table: `#`, status, step and where columns whose cells wrap instead of being cut, the characters a filter
//! matched underlined in the accent colour and a picked step marked by its number.

use std::collections::HashMap;

use cockpit_core::text::{width, wrap};
use cockpit_core::theme::{DIM, FG, GREEN, ORANGE, bold, dim, fg};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::acts::{handoff_id, pr_ref};
use crate::anim::Phase;
use crate::book::{Book, Step};
use crate::filter::Hit;
use crate::handoff;

/// Each column's width, its one cell of right padding included.
pub struct Widths {
    pub n: usize,
    pub status: usize,
    pub what: usize,
    pub where_: usize,
}

impl Widths {
    pub fn new(book: &Book, total: usize) -> Self {
        let longest = book.steps.iter().map(|s| s.where_.chars().count()).chain([8]).max().unwrap_or(8);
        let (n, status, where_) = (4, total * 32 / 100, (longest + 2).min(24));
        Self { n, status, where_, what: total.saturating_sub(n + status + where_).max(2) }
    }

    fn all(&self) -> [usize; 4] {
        [self.n, self.status, self.what, self.where_]
    }
}

type Cell = Vec<Vec<Span<'static>>>;

/// How the filter's matched characters stand out, over whatever style their cell has.
const MATCHED: Style = Style::new().fg(ORANGE).add_modifier(Modifier::BOLD.union(Modifier::UNDERLINED));

/// What a row shows beyond its step: where its done animation is, whether it is picked, what the filter matched.
pub struct Marks<'a> {
    pub phase: Phase,
    pub picked: bool,
    pub hit: Option<&'a Hit>,
}

/// Text wrapped to the cell, each paragraph of it on lines of its own; the characters at `matched` (char indexes into
/// `value`) drawn as matches.
fn text(cell: &mut Cell, value: &str, style: Style, cells: usize, matched: &[usize]) {
    let chars: Vec<char> = value.chars().collect();
    let mut at = 0;
    for line in wrap(value, cells.max(1)) {
        if matched.is_empty() {
            cell.push(vec![Span::styled(line, style)]);
            continue;
        }
        let mut spans: Vec<Span<'static>> = Vec::new();
        for c in line.chars() {
            // wrap only drops the spaces where it breaks a line.
            while chars.get(at).is_some_and(|&p| p != c && p == ' ') {
                at += 1;
            }
            let style = if matched.contains(&at) { style.patch(MATCHED) } else { style };
            at += 1;
            match spans.last_mut() {
                Some(last) if last.style == style => last.content.to_mut().push(c),
                _ => spans.push(Span::styled(c.to_string(), style)),
            }
        }
        cell.push(spans);
    }
}

/// Match indexes moved by what is drawn before the value.
fn shifted(matched: &[usize], by: usize) -> Vec<usize> {
    matched.iter().map(|i| i + by).collect()
}

/// The cells side by side, each line padded to its column.
fn join(cells: [Cell; 4], widths: [usize; 4]) -> Vec<Line<'static>> {
    let height = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
    (0..height)
        .map(|i| {
            let mut spans = Vec::new();
            for (cell, cells) in cells.iter().zip(widths) {
                let line = cell.get(i).cloned().unwrap_or_default();
                let used: usize = line.iter().map(|s| width(&s.content)).sum();
                spans.extend(line);
                spans.push(Span::raw(" ".repeat(cells.saturating_sub(used))));
            }
            Line::from(spans)
        })
        .collect()
}

pub fn header(widths: &Widths) -> Line<'static> {
    let cells = ["#", "status", "step", "where"].map(|label| vec![vec![Span::styled(label, dim())]]);
    join(cells, widths.all()).remove(0)
}

pub fn row(book: &Book, step: &Step, widths: &Widths, marks: Marks, handoffs: &HashMap<String, Value>, inbox: &Value) -> Vec<Line<'static>> {
    let is_done = !step.is_open();
    let plain = if is_done { dim() } else { fg(FG) };
    let none = Hit::default();
    let hit = marks.hit.unwrap_or(&none);
    let mut n = Cell::new();
    if marks.picked {
        text(&mut n, &format!("●{}", step.n), bold(ORANGE), widths.n.saturating_sub(1), &shifted(&hit.fields[0], 1));
    } else {
        text(&mut n, &step.n, dim(), widths.n.saturating_sub(1), &hit.fields[0]);
    }
    let mut status = Cell::new();
    let rest = step.status.split_once(' ').map_or(step.status.as_str(), |(_, r)| r);
    match marks.phase {
        Phase::Waiting => text(&mut status, &format!("{} {rest}", crate::anim::SPINNER[0]), dim(), widths.status.saturating_sub(1), &[]),
        Phase::Spin(frame) => text(&mut status, &format!("{frame} {rest}"), fg(ORANGE), widths.status.saturating_sub(1), &[]),
        Phase::Spark => text(&mut status, &format!("{} ✦", step.status), bold(GREEN), widths.status.saturating_sub(1), &[]),
        Phase::Still => text(&mut status, &step.status, plain, widths.status.saturating_sub(1), &hit.fields[3]),
    }
    let mut what = Cell::new();
    text(&mut what, &step.step, plain, widths.what.saturating_sub(1), &hit.fields[1]);
    if !step.command.is_empty() {
        text(&mut what, &format!("$ {}", step.command), if is_done { dim() } else { fg(ORANGE) }, widths.what.saturating_sub(1), &shifted(&hit.fields[4], 2));
    }
    if let Some((said, style)) = handoffs.get(&handoff_id(book, step)).and_then(handoff::line) {
        text(&mut what, &said, style, widths.what.saturating_sub(1), &[]);
    }
    let mut where_ = Cell::new();
    if let Some((repo, number)) = pr_ref(step) {
        text(&mut where_, &step.where_, Style::new().fg(DIM).add_modifier(Modifier::UNDERLINED), widths.where_.saturating_sub(1), &hit.fields[2]);
        if let Some((badge, style)) = handoff::live_badge(&repo, number, inbox) {
            text(&mut where_, &badge, style, widths.where_.saturating_sub(1), &[]);
        }
    } else {
        text(&mut where_, &step.where_, dim(), widths.where_.saturating_sub(1), &hit.fields[2]);
    }
    join([n, status, what, where_], widths.all())
}
