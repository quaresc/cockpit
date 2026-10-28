//! Markdown drawn in the pane, parsed with comrak (GitHub Flavored: tables, task lists, strikethrough, autolinks,
//! footnotes, alerts): a centred h1, blank lines around headings, paragraphs joined and wrapped, nested bullets, numbered
//! lists, checkboxes, quotes with a bar, alerts with their colour and icon, boxed tables, code in a tinted panel with
//! syntax colours, footnotes under a rule, and links a click opens.

use std::collections::HashMap;
use std::sync::LazyLock;

use comrak::arena_tree::NodeEdge;
use comrak::nodes::{AlertType, ListType, NodeValue, TableAlignment};
use comrak::{Arena, Options, parse_document};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use regex::Regex;

use cockpit_core::text::width;
use cockpit_core::theme::{BLUE, DIM, FG, GREEN, ORANGE, PANEL, PINK, RED, bold, dim, fg};

use crate::highlight;
use crate::row::{Act, Row};

/// A piece of text, its style, and the web address a click on it opens.
pub type Segment = (String, Style, Option<String>);

const BULLETS: [&str; 3] = ["• ", "▪ ", "‣ "];
const QUOTE_BAR: Color = Color::Rgb(122, 98, 80);
const BORDER: Color = Color::Rgb(70, 70, 76);
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\S+\s*|\s+").unwrap());

/// Splits styled segments into rows of at most `room` cells, breaking between words; a "\n" in a segment forces a break.
pub fn wrap(segments: Vec<Segment>, room: usize) -> Vec<Vec<Segment>> {
    let room = room.max(1);
    let (mut rows, mut row, mut used): (Vec<Vec<Segment>>, Vec<Segment>, usize) = (Vec::new(), Vec::new(), 0);
    let mut wrapped = false;
    for (text, style, url) in segments {
        for (n, part) in text.split('\n').enumerate() {
            if n > 0 {
                rows.push(std::mem::take(&mut row));
                (used, wrapped) = (0, false);
            }
            for word in WORD.find_iter(part) {
                let mut word = word.as_str().to_string();
                if used > 0 && used + width(word.trim_end()) > room {
                    rows.push(std::mem::take(&mut row));
                    (used, wrapped) = (0, true);
                }
                if used == 0 && wrapped {
                    word = word.trim_start().to_string();
                    if word.is_empty() {
                        continue;
                    }
                }
                while used + width(word.trim_end()) > room {
                    let (head, rest) = split_at_cells(&word, room.saturating_sub(used).max(1));
                    row.push((head, style, url.clone()));
                    rows.push(std::mem::take(&mut row));
                    (used, wrapped) = (0, true);
                    word = rest;
                }
                used += width(&word);
                row.push((word, style, url.clone()));
            }
        }
    }
    rows.push(row);
    rows
}

/// The text cut after `cells` terminal cells, at least one character going first so a split always progresses.
pub fn split_at_cells(text: &str, cells: usize) -> (String, String) {
    let mut used = 0;
    let at = text.char_indices().find(|(_, c)| {
        used += width(&c.to_string());
        used > cells
    });
    let at = at.map_or(text.len(), |(i, _)| i).max(text.chars().next().map_or(0, char::len_utf8));
    (text[..at].to_string(), text[at..].to_string())
}

pub fn add_segments(mut row: Row, segments: Vec<Segment>) -> Row {
    for (text, style, url) in segments {
        row = match url {
            Some(url) => row.act(text, style, Act::Open(url)),
            None => row.add(text, style),
        };
    }
    row
}

fn web(url: &str) -> Option<String> {
    (url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:")).then(|| url.to_string())
}

/// An alert's colour and its Nerd Font octicon, as GitHub draws them.
fn alert_look(kind: AlertType) -> (Color, &'static str) {
    match kind {
        AlertType::Note => (BLUE, "\u{f449}"),
        AlertType::Tip => (GREEN, "\u{f400}"),
        AlertType::Important => (PINK, "\u{f50a}"),
        AlertType::Warning => (ORANGE, "\u{f421}"),
        AlertType::Caution => (RED, "\u{f46e}"),
    }
}

/// What the lines being drawn sit in: a quote's or an alert's bar, or a list item's marker on its first line and room
/// under it after; a tight item's paragraphs have no blank line between them.
enum Container {
    Quote(Color),
    Item { marker: String, style: Style, used: bool, tight: bool },
}

struct Table {
    aligns: Vec<TableAlignment>,
    rows: Vec<Vec<Vec<Segment>>>,
}

#[derive(Default)]
struct Writer {
    cols: usize,
    out: Vec<Row>,
    containers: Vec<Container>,
    /// Each open list's next number (None for bullets) and whether it is tight.
    lists: Vec<(Option<usize>, bool)>,
    inline: Vec<Segment>,
    styles: Vec<Style>,
    /// How many styles were open when each item started: an item's styles end with it, a done task's struck one too.
    item_styles: Vec<usize>,
    links: Vec<Option<String>>,
    image: Option<(Option<String>, String)>,
    table: Option<Table>,
    /// Each footnote's number by its name, as its references show it.
    footnotes: HashMap<String, u32>,
    footnotes_shown: bool,
    /// A blank line is due before the next block.
    gap: bool,
    /// The grammars, each loaded at the first code block that needs it and held to the end.
    grammars: highlight::Grammars,
}

impl Writer {
    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, f: impl Fn(Style) -> Style) {
        self.styles.push(f(self.style()));
    }

    /// The prefix of the next line: quote bars and list markers, a marker shown once then turned to room.
    fn prefix(&mut self) -> Vec<(String, Style)> {
        let mut out = Vec::new();
        for c in &mut self.containers {
            match c {
                Container::Quote(color) => out.push(("▌ ".to_string(), fg(*color))),
                Container::Item { marker, style, used, .. } => {
                    if *used {
                        out.push((" ".repeat(width(marker)), Style::new()));
                    } else {
                        out.push((marker.clone(), *style));
                        *used = true;
                    }
                }
            }
        }
        out
    }

    fn room(&self) -> usize {
        let taken: usize = self
            .containers
            .iter()
            .map(|c| match c {
                Container::Quote(_) => 2,
                Container::Item { marker, .. } => width(marker),
            })
            .sum();
        self.cols.saturating_sub(taken).max(10)
    }

    fn line(&mut self, segments: Vec<Segment>) {
        let mut row = Row::new();
        for (text, style) in self.prefix() {
            row = row.add(text, style);
        }
        self.out.push(add_segments(row, segments));
    }

    /// Starts a block: the blank line the previous one asked for, the quotes' bars running through it.
    fn block(&mut self) {
        if std::mem::take(&mut self.gap) && !self.out.is_empty() {
            let bars: Vec<Color> = self.containers.iter().filter_map(|c| if let Container::Quote(color) = c { Some(*color) } else { None }).collect();
            let mut row = Row::new();
            for (k, color) in bars.iter().enumerate() {
                row = row.add(if k + 1 == bars.len() { "▌" } else { "▌ " }, fg(*color));
            }
            self.out.push(row);
        }
    }

    /// The text gathered so far as wrapped lines: a paragraph, or a heading's or cell's text.
    fn flush(&mut self) {
        let segments = std::mem::take(&mut self.inline);
        if segments.iter().all(|(t, _, _)| t.trim().is_empty()) {
            return;
        }
        self.block();
        for row in wrap(segments, self.room()) {
            self.line(row);
        }
    }

    fn text(&mut self, text: &str) {
        if let Some((_, alt)) = self.image.as_mut() {
            alt.push_str(text);
            return;
        }
        let link = self.links.last().cloned().flatten();
        self.inline.push((text.to_string(), self.style(), link));
    }

    fn heading(&mut self, level: u8) {
        let segments = std::mem::take(&mut self.inline);
        self.gap = true;
        self.block();
        let room = self.room();
        for row in wrap(segments, room) {
            if level == 1 {
                let used: usize = row.iter().map(|(t, _, _)| width(t)).sum();
                let mut centred = vec![(" ".repeat(room.saturating_sub(used) / 2), Style::new(), None)];
                centred.extend(row);
                self.line(centred);
            } else {
                self.line(row);
            }
        }
        self.gap = true;
    }

    fn code_block(&mut self, info: &str, text: &str) {
        self.flush();
        self.block();
        let room = self.room();
        let panel = Style::new().bg(PANEL);
        let lang = info.split_whitespace().next().unwrap_or_default().to_string();
        for (n, pieces) in highlight::code(&mut self.grammars, &lang, text).into_iter().enumerate() {
            let segments: Vec<Segment> = pieces.into_iter().map(|(t, s)| (t, s.bg(PANEL), None)).collect();
            for (k, mut row) in hard_wrap(segments, room.saturating_sub(2)).into_iter().enumerate() {
                let used: usize = row.iter().map(|(t, _, _)| width(t)).sum();
                let mut pad = room.saturating_sub(used + 1);
                // The language at the first line's right end, when it fits.
                let tag = (n == 0 && k == 0 && !lang.is_empty() && pad > width(&lang) + 2).then(|| lang.clone());
                if let Some(tag) = &tag {
                    pad -= width(tag) + 1;
                }
                row.insert(0, (" ".into(), panel, None));
                row.push((" ".repeat(pad), panel, None));
                if let Some(tag) = tag {
                    row.push((format!("{tag} "), panel.fg(DIM).add_modifier(Modifier::ITALIC), None));
                }
                self.line(row);
            }
        }
        self.gap = true;
    }

    fn rule(&mut self) {
        self.flush();
        self.block();
        let room = self.room();
        self.line(vec![("─".repeat(room), fg(BORDER), None)]);
        self.gap = true;
    }

    fn html(&mut self, text: &str) {
        self.flush();
        self.block();
        for line in text.trim_end().lines() {
            self.line(vec![(line.to_string(), dim(), None)]);
        }
        self.gap = true;
    }

    fn table(&mut self, table: Table) {
        self.block();
        let n = table.aligns.len().max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
        if n == 0 {
            return;
        }
        let cell_width = |cell: &Vec<Segment>| cell.iter().map(|(t, _, _)| width(t)).sum::<usize>();
        let mut sizes: Vec<usize> = (0..n).map(|i| table.rows.iter().filter_map(|r| r.get(i)).map(cell_width).max().unwrap_or(0).max(1)).collect();
        let room = self.room().saturating_sub(3 * n + 1);
        while sizes.iter().sum::<usize>() > room && sizes.iter().any(|s| *s > 3) {
            let widest = sizes.iter().enumerate().max_by_key(|(i, s)| (**s, usize::MAX - i)).map_or(0, |(i, _)| i);
            sizes[widest] -= 1;
        }
        let border = fg(BORDER);
        let rule = |l: &str, m: &str, r: &str| {
            let middle: Vec<String> = sizes.iter().map(|s| "─".repeat(s + 2)).collect();
            vec![(format!("{l}{}{r}", middle.join(m)), border, None)]
        };
        self.line(rule("┌", "┬", "┐"));
        for (k, cells) in table.rows.iter().enumerate() {
            let wrapped: Vec<Vec<Vec<Segment>>> = (0..n)
                .map(|i| {
                    let cell = cells.get(i).cloned().unwrap_or_default();
                    let cell = if k == 0 { cell.into_iter().map(|(t, s, u)| (t, s.add_modifier(Modifier::BOLD), u)).collect() } else { cell };
                    wrap(cell, sizes[i])
                })
                .collect();
            let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
            for line in 0..height {
                let mut row: Vec<Segment> = vec![("│".into(), border, None)];
                for (i, cell) in wrapped.iter().enumerate() {
                    let mut piece = cell.get(line).cloned().unwrap_or_default();
                    if let Some(last) = piece.last_mut() {
                        last.0 = last.0.trim_end().to_string();
                    }
                    let used: usize = piece.iter().map(|(t, _, _)| width(t)).sum();
                    let free = sizes[i].saturating_sub(used);
                    let (before, after) = match table.aligns.get(i) {
                        Some(TableAlignment::Right) => (free, 0),
                        Some(TableAlignment::Center) => (free / 2, free - free / 2),
                        _ => (0, free),
                    };
                    row.push((" ".repeat(before + 1), Style::new(), None));
                    row.extend(piece);
                    row.push((" ".repeat(after + 1), Style::new(), None));
                    row.push(("│".into(), border, None));
                }
                self.line(row);
            }
            if k == 0 && table.rows.len() > 1 {
                self.line(rule("├", "┼", "┤"));
            }
        }
        self.line(rule("└", "┴", "┘"));
        self.gap = true;
    }

    /// A list item or a footnote: its marker on its first line, its styles ending with it.
    fn item(&mut self, marker: String, style: Style, tight: bool) {
        self.flush();
        self.item_styles.push(self.styles.len());
        self.containers.push(Container::Item { marker, style, used: false, tight });
    }

    fn end_item(&mut self) {
        self.flush();
        if let Some(n) = self.item_styles.pop() {
            self.styles.truncate(n);
        }
        // An item with nothing in it still shows its marker.
        if let Some(Container::Item { used: false, .. }) = self.containers.last() {
            self.line(vec![]);
        }
        self.containers.pop();
    }

    /// The next item's marker: its number in an ordered list, else the bullet of its depth.
    fn marker(&mut self) -> String {
        let depth = self.lists.len().saturating_sub(1);
        match self.lists.last_mut().and_then(|(next, _)| next.as_mut()) {
            Some(n) => {
                *n += 1;
                format!("{}. ", *n - 1)
            }
            None => BULLETS[depth % BULLETS.len()].to_string(),
        }
    }

    fn enter(&mut self, value: &NodeValue) {
        match value {
            NodeValue::Heading(heading) => {
                self.flush();
                let style = match heading.level {
                    1 => bold(ORANGE),
                    2 => bold(FG).add_modifier(Modifier::UNDERLINED),
                    3 => bold(FG),
                    4 => bold(DIM).add_modifier(Modifier::ITALIC),
                    _ => dim().add_modifier(Modifier::ITALIC),
                };
                self.styles.push(style);
            }
            NodeValue::BlockQuote => {
                self.flush();
                self.block();
                self.containers.push(Container::Quote(QUOTE_BAR));
                self.push_style(|s| s.add_modifier(Modifier::ITALIC));
            }
            NodeValue::Alert(alert) => {
                self.flush();
                self.block();
                let (color, icon) = alert_look(alert.alert_type);
                self.containers.push(Container::Quote(color));
                let title = alert.title.clone().unwrap_or_else(|| alert.alert_type.default_title().to_string());
                self.line(vec![(format!("{icon} {title}"), bold(color), None)]);
                self.styles.push(self.style());
            }
            NodeValue::CodeBlock(block) => self.code_block(&block.info, &block.literal),
            NodeValue::HtmlBlock(block) => self.html(&block.literal),
            NodeValue::List(list) => {
                self.flush();
                if self.lists.is_empty() {
                    self.block();
                }
                self.lists.push(((list.list_type == ListType::Ordered).then_some(list.start), list.tight));
            }
            NodeValue::Item(_) => {
                let (marker, tight) = (self.marker(), self.lists.last().is_some_and(|l| l.1));
                self.item(marker, fg(ORANGE), tight);
            }
            NodeValue::TaskItem(task) => {
                let tight = self.lists.last().is_some_and(|l| l.1);
                self.marker();
                let done = task.symbol.is_some_and(|c| !c.is_whitespace());
                let (marker, style) = if done { ("☑ ", fg(GREEN)) } else { ("☐ ", dim()) };
                self.item(marker.into(), style, tight);
                if done {
                    self.push_style(|s| s.fg(DIM).add_modifier(Modifier::CROSSED_OUT));
                }
            }
            NodeValue::FootnoteDefinition(note) => {
                if !std::mem::replace(&mut self.footnotes_shown, true) {
                    self.rule();
                }
                let number = self.footnotes.get(&note.name).copied().unwrap_or(self.footnotes.len() as u32 + 1);
                self.item(format!("[{number}] "), fg(BLUE), true);
            }
            NodeValue::Emph => self.push_style(|s| s.add_modifier(Modifier::ITALIC)),
            NodeValue::Strong => self.push_style(|s| s.add_modifier(Modifier::BOLD)),
            NodeValue::Strikethrough => self.push_style(|s| s.add_modifier(Modifier::CROSSED_OUT)),
            NodeValue::Link(link) => {
                self.links.push(web(&link.url));
                self.push_style(|s| s.fg(BLUE).add_modifier(Modifier::UNDERLINED));
            }
            NodeValue::Image(link) => self.image = Some((web(&link.url), String::new())),
            NodeValue::Table(table) => {
                self.flush();
                self.table = Some(Table { aligns: table.alignments.clone(), rows: Vec::new() });
            }
            NodeValue::TableRow(_) => {
                if let Some(t) = self.table.as_mut() {
                    t.rows.push(Vec::new());
                }
            }
            NodeValue::TableCell => self.inline.clear(),
            NodeValue::Text(text) => self.text(text),
            NodeValue::Code(code) => match self.image.as_mut() {
                Some((_, alt)) => alt.push_str(&code.literal),
                None => {
                    let link = self.links.last().cloned().flatten();
                    self.inline.push((code.literal.clone(), self.style().fg(ORANGE).bg(PANEL), link));
                }
            },
            NodeValue::HtmlInline(html) => self.inline.push((html.clone(), dim(), None)),
            NodeValue::SoftBreak => self.text(" "),
            NodeValue::LineBreak => self.inline.push(("\n".into(), Style::new(), None)),
            NodeValue::ThematicBreak => self.rule(),
            NodeValue::FootnoteReference(note) => {
                self.footnotes.insert(note.name.clone(), note.ix);
                self.inline.push((format!("[{}]", note.ix), fg(BLUE), None));
            }
            _ => {}
        }
    }

    fn leave(&mut self, value: &NodeValue) {
        match value {
            NodeValue::Paragraph => {
                self.flush();
                self.gap = !matches!(self.containers.last(), Some(Container::Item { tight: true, .. }));
            }
            NodeValue::Heading(heading) => {
                self.styles.pop();
                self.heading(heading.level);
            }
            NodeValue::BlockQuote | NodeValue::Alert(_) => {
                self.flush();
                self.containers.pop();
                self.styles.pop();
                self.gap = true;
            }
            NodeValue::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.gap = true;
                }
            }
            NodeValue::Item(_) | NodeValue::TaskItem(_) => self.end_item(),
            NodeValue::FootnoteDefinition(_) => {
                self.end_item();
                self.gap = false;
            }
            NodeValue::Emph | NodeValue::Strong | NodeValue::Strikethrough => {
                self.styles.pop();
            }
            NodeValue::Link(_) => {
                self.links.pop();
                self.styles.pop();
            }
            NodeValue::Image(_) => {
                if let Some((url, alt)) = self.image.take() {
                    let label = if alt.trim().is_empty() { "picture".to_string() } else { alt };
                    self.inline.push((format!("▣ {label}"), self.style().fg(DIM).add_modifier(Modifier::UNDERLINED), url));
                }
            }
            NodeValue::Table(_) => {
                if let Some(table) = self.table.take() {
                    self.table(table);
                }
            }
            NodeValue::TableCell => {
                let cell = std::mem::take(&mut self.inline);
                if let Some(row) = self.table.as_mut().and_then(|t| t.rows.last_mut()) {
                    row.push(cell);
                }
            }
            _ => {}
        }
    }
}

/// Code cut into rows of `room` cells, not between words: code keeps its spaces.
fn hard_wrap(segments: Vec<Segment>, room: usize) -> Vec<Vec<Segment>> {
    let room = room.max(1);
    let (mut rows, mut row, mut used) = (Vec::new(), Vec::new(), 0);
    for (mut text, style, url) in segments {
        while used + width(&text) > room {
            if used < room {
                let (head, rest) = split_at_cells(&text, room - used);
                row.push((head, style, url.clone()));
                text = rest;
            }
            rows.push(std::mem::take(&mut row));
            used = 0;
        }
        used += width(&text);
        row.push((text, style, url));
    }
    rows.push(row);
    rows
}

/// GitHub Flavored Markdown with its footnotes and alerts.
fn options() -> Options<'static> {
    let mut options = Options::default();
    let extension = &mut options.extension;
    (extension.table, extension.strikethrough, extension.tasklist, extension.autolink) = (true, true, true, true);
    (extension.footnotes, extension.alerts) = (true, true);
    options
}

pub fn markdown(text: &str, cols: usize) -> Vec<Row> {
    let arena = Arena::new();
    let root = parse_document(&arena, text, &options());
    let mut writer = Writer { cols: cols.max(20), ..Writer::default() };
    for edge in root.traverse() {
        match edge {
            NodeEdge::Start(node) => writer.enter(&node.data().value),
            NodeEdge::End(node) => writer.leave(&node.data().value),
        }
    }
    writer.flush();
    while writer.out.last().is_some_and(|r| r.line.width() == 0) {
        writer.out.pop();
    }
    writer.out
}

/// Markdown as lines for another space's panel: the same look, without the clicks.
pub fn lines(text: &str, cols: usize) -> Vec<Line<'static>> {
    markdown(text, cols).into_iter().map(|row| row.line).collect()
}

/// A line's inline marks only (code, emphasis, links), its blocks' marks dropped: a review comment's first line in a
/// panel row. The text as it is when nothing in it is inline (a code fence's opening line).
pub fn inline(text: &str) -> Vec<Span<'static>> {
    let arena = Arena::new();
    let root = parse_document(&arena, text, &options());
    let mut writer = Writer::default();
    for edge in root.traverse() {
        match edge {
            NodeEdge::Start(node) => match &node.data().value {
                NodeValue::Paragraph | NodeValue::Heading(_) | NodeValue::TableCell if !writer.inline.is_empty() => writer.text(" "),
                value @ (NodeValue::Text(_)
                | NodeValue::Code(_)
                | NodeValue::Emph
                | NodeValue::Strong
                | NodeValue::Strikethrough
                | NodeValue::Link(_)
                | NodeValue::Image(_)
                | NodeValue::SoftBreak
                | NodeValue::HtmlInline(_)) => writer.enter(value),
                _ => {}
            },
            NodeEdge::End(node) => {
                if let value @ (NodeValue::Emph | NodeValue::Strong | NodeValue::Strikethrough | NodeValue::Link(_) | NodeValue::Image(_)) = &node.data().value {
                    writer.leave(value);
                }
            }
        }
    }
    if writer.inline.iter().all(|(t, _, _)| t.trim().is_empty()) {
        return vec![Span::raw(text.to_string())];
    }
    writer.inline.into_iter().map(|(t, s, _)| Span::styled(t, s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Whatever the text and the width, drawing it ends without a panic.
        #[test]
        fn any_text_draws(text in r"([-*#>|`_~\\\[\]()!:^ \n0-9a-zé✓.x]|https://a\.b/c|www\.d\.e|```rust\n|```tsx\n|- \[ \] |\|---\||\[\^1\]|> \[!NOTE\]\n){0,80}", cols in 0usize..160) {
            let _ = markdown(&text, cols);
        }

        #[test]
        fn any_unicode_draws(text in "\\PC{0,200}", cols in 0usize..100) {
            let _ = markdown(&text, cols);
        }
    }

    fn text(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.line.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim_end().to_string()).collect()
    }

    #[test]
    fn escapes_stay_literal() {
        let shown = text(&markdown(r"\*not italic\* and \# not a heading, 2 \* 3", 60));
        assert_eq!(shown, vec!["*not italic* and # not a heading, 2 * 3"]);
    }

    #[test]
    fn paragraphs_join_their_lines() {
        assert_eq!(text(&markdown("one\ntwo\n\nthree", 40)), vec!["one two", "", "three"]);
    }

    #[test]
    fn a_word_that_just_fits_leaves_no_space_on_the_next_line() {
        assert_eq!(text(&markdown(&format!("{} as next", "x".repeat(17)), 20)), vec![format!("{} as", "x".repeat(17)), "next".into()]);
    }

    #[test]
    fn nested_bullets_and_tasks() {
        let shown = text(&markdown("- a\n  - b\n    - c\n- [x] done\n- [ ] todo\n\n1. first\n2. second", 40));
        assert_eq!(shown, vec!["• a", "  ▪ b", "    ‣ c", "☑ done", "☐ todo", "", "1. first", "2. second"]);
    }

    #[test]
    fn links_open_on_click() {
        let rows = markdown("see [the docs](https://example.com) or https://x.dev/a.", 60);
        let mut acts: Vec<&Act> = rows[0].clicks.iter().map(|(_, _, a)| a).collect();
        acts.dedup();
        assert_eq!(acts, vec![&Act::Open("https://example.com".into()), &Act::Open("https://x.dev/a".into())]);
    }

    #[test]
    fn alerts_have_their_icon_title_and_colour() {
        let rows = markdown("> [!WARNING]\n> Mind the quota.\n\n> [!TIP] Shortcut\n> Press s.", 40);
        assert_eq!(text(&rows), vec!["▌ \u{f421} Warning", "▌ Mind the quota.", "", "▌ \u{f400} Shortcut", "▌ Press s."]);
        assert_eq!(rows[0].line.spans[0].style.fg, Some(ORANGE));
        assert_eq!(rows[3].line.spans[1].style.fg, Some(GREEN));
        assert!(!rows[1].line.spans[1].style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn footnotes_go_under_a_rule() {
        let shown = text(&markdown("Counted[^n] here.\n\n[^n]: Per organisation.", 30));
        assert_eq!(shown, vec!["Counted[1] here.", "", &"─".repeat(30), "", "[1] Per organisation."]);
    }

    #[test]
    fn inline_keeps_only_the_inline_marks() {
        let spans = inline("## Use `quota()` **here**");
        let shown: Vec<(&str, Option<Color>, bool)> = spans.iter().map(|s| (s.content.as_ref(), s.style.fg, s.style.add_modifier.contains(Modifier::BOLD))).collect();
        assert_eq!(shown, vec![("Use ", None, false), ("quota()", Some(ORANGE), false), (" ", None, false), ("here", None, true)]);
        assert_eq!(inline("```suggestion")[0].content, "```suggestion");
    }

    #[test]
    fn www_addresses_link_too() {
        let rows = markdown("see www.example.org now", 40);
        assert!(rows[0].clicks.iter().any(|(_, _, a)| *a == Act::Open("http://www.example.org".into())));
    }
}
