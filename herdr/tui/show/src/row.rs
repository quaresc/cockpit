//! One row of the scrolling body: its styled text, what a click on part of it does, the focus stop it belongs to, the
//! image it is a slice of, and whether the block's own cursor sits on it.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use cockpit_core::text::width;

/// What a click on a span does, handled by the block the row belongs to (links by the app).
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Open(String),
    Sort(usize),
    Cursor(usize),
    Fold(usize),
    Pick(usize),
    Answer(bool),
    /// A diff's line cursor to this file and line (None: the file's header).
    Line(usize, Option<usize>),
    /// A diff's file list entry: the cursor to that file.
    Jump(usize),
}

/// A place the keyboard can sit in a block, cycled with tab like Textual's focus chain.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Table,
    Diff,
    Log,
    Chart,
    Compare,
    Options,
    Input,
    Button(bool),
}

#[derive(Clone, Default)]
pub struct Row {
    pub line: Line<'static>,
    pub clicks: Vec<(usize, usize, Act)>,
    pub target: Option<Target>,
    /// Which row of its block's image this row shows.
    pub image: Option<u16>,
    pub block: usize,
    /// The block's cursor (a diff line, a comment field) is on this row: focus scrolls to it rather than the whole block.
    pub here: bool,
}

impl Row {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(text: impl Into<String>, style: Style) -> Self {
        Self::new().add(text, style)
    }

    pub fn add(mut self, text: impl Into<String>, style: Style) -> Self {
        self.line.spans.push(Span::styled(text.into(), style));
        self
    }

    /// Adds a span a click acts on.
    pub fn act(mut self, text: impl Into<String>, style: Style, act: Act) -> Self {
        let start = self.line.width();
        let text = text.into();
        let end = start + width(&text);
        self.clicks.push((start, end, act));
        self.add(text, style)
    }

    pub fn target(mut self, target: Target) -> Self {
        self.target = Some(target);
        self
    }

    pub fn here(mut self, here: bool) -> Self {
        self.here = here;
        self
    }

    pub fn from_line(line: Line<'static>) -> Self {
        Self { line, ..Self::default() }
    }

    pub fn act_at(&self, x: usize) -> Option<&Act> {
        self.clicks.iter().find(|(a, b, _)| x >= *a && x < *b).map(|(_, _, act)| act)
    }
}

/// The cells `[from, from + cells)` of styled pieces laid end to end, padded with `fill` to exactly `cells`.
pub fn slice(pieces: &[(String, Style)], from: usize, cells: usize, fill: Style) -> Line<'static> {
    let mut spans = Vec::new();
    let mut at = 0;
    let end = from + cells;
    for (text, style) in pieces {
        let w = width(text);
        if at + w <= from || at >= end {
            at += w;
            continue;
        }
        let mut out = String::new();
        for c in text.chars() {
            let cw = c.width().unwrap_or(0);
            if at >= from && at + cw <= end {
                out.push(c);
            } else if at < end && at + cw > from {
                out.push(' ');
            }
            at += cw;
        }
        spans.push(Span::styled(out, *style));
    }
    let used: usize = spans.iter().map(|s| s.width()).sum();
    if used < cells {
        spans.push(Span::styled(" ".repeat(cells - used), fill));
    }
    Line::from(spans)
}
