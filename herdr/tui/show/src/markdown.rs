//! Markdown drawn in the pane, parsed with pulldown-cmark (CommonMark, tables, strikethrough, task
//! lists): a centred h1, blank lines around headings, paragraphs joined and wrapped, nested bullets, numbered lists,
//! checkboxes, quotes with a bar, boxed tables, code in a tinted panel with syntax colours, and links a click opens.

use std::sync::LazyLock;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use regex::Regex;

use cockpit_core::text::width;
use cockpit_core::theme::{BLUE, DIM, FG, GREEN, ORANGE, PANEL, bold, dim, fg};

use crate::highlight;
use crate::row::{Act, Row};

/// A piece of text, its style, and the web address a click on it opens.
pub type Segment = (String, Style, Option<String>);

const BULLETS: [&str; 3] = ["• ", "▪ ", "‣ "];
const QUOTE_BAR: Color = Color::Rgb(122, 98, 80);
const BORDER: Color = Color::Rgb(70, 70, 76);
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\S+\s*|\s+").unwrap());
static BARE_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://[^\s<>()]+[^\s<>().,;:!?'\x22]").unwrap());

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

/// What the lines being drawn sit in: a quote's bar, or a list item's marker on its first line and room under it after.
enum Container {
    Quote,
    Item { marker: String, style: Style, used: bool },
}

struct Table {
    aligns: Vec<Alignment>,
    rows: Vec<Vec<Vec<Segment>>>,
}

#[derive(Default)]
struct Writer {
    cols: usize,
    out: Vec<Row>,
    containers: Vec<Container>,
    /// Each open list's next number, None for bullets.
    lists: Vec<Option<u64>>,
    inline: Vec<Segment>,
    styles: Vec<Style>,
    links: Vec<Option<String>>,
    image: Option<(Option<String>, String)>,
    code: Option<(String, String)>,
    table: Option<Table>,
    html: String,
    /// A blank line is due before the next block.
    gap: bool,
    /// The grammars, loaded at the first code block that needs them and held to the end.
    grammars: Option<highlight::Grammars>,
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
                Container::Quote => out.push(("▌ ".to_string(), fg(QUOTE_BAR))),
                Container::Item { marker, style, used } => {
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
                Container::Quote => 2,
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

    /// Starts a block: the blank line the previous one asked for, the quote's bar running through it.
    fn block(&mut self) {
        if std::mem::take(&mut self.gap) && !self.out.is_empty() {
            let bars = self.containers.iter().filter(|c| matches!(c, Container::Quote)).count();
            self.out.push(Row::text("▌ ".repeat(bars).trim_end().to_string(), fg(QUOTE_BAR)));
        }
    }

    /// The text gathered so far as wrapped lines: a paragraph, or a tight list item's text.
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
        if let Some((_, code)) = self.code.as_mut() {
            code.push_str(text);
            return;
        }
        if let Some((_, alt)) = self.image.as_mut() {
            alt.push_str(text);
            return;
        }
        let style = self.style();
        if let Some(link) = self.links.last() {
            self.inline.push((text.to_string(), style, link.clone()));
            return;
        }
        // A bare web address in the text is a link too.
        let mut at = 0;
        for m in BARE_URL.find_iter(text) {
            if m.start() > at {
                self.inline.push((text[at..m.start()].to_string(), style, None));
            }
            self.inline.push((m.as_str().to_string(), style.fg(BLUE).add_modifier(Modifier::UNDERLINED), Some(m.as_str().to_string())));
            at = m.end();
        }
        if at < text.len() {
            self.inline.push((text[at..].to_string(), style, None));
        }
    }

    fn heading(&mut self, level: HeadingLevel) {
        let segments = std::mem::take(&mut self.inline);
        self.gap = true;
        self.block();
        let room = self.room();
        for row in wrap(segments, room) {
            if level == HeadingLevel::H1 {
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
                        Some(Alignment::Right) => (free, 0),
                        Some(Alignment::Center) => (free / 2, free - free / 2),
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

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush();
                let style = match level {
                    HeadingLevel::H1 => bold(ORANGE),
                    HeadingLevel::H2 => bold(FG).add_modifier(Modifier::UNDERLINED),
                    HeadingLevel::H3 => bold(FG),
                    HeadingLevel::H4 => bold(DIM).add_modifier(Modifier::ITALIC),
                    _ => dim().add_modifier(Modifier::ITALIC),
                };
                self.styles.push(style);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.block();
                self.containers.push(Container::Quote);
                self.push_style(|s| s.add_modifier(Modifier::ITALIC));
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let info = match kind {
                    CodeBlockKind::Fenced(info) => info.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((info, String::new()));
            }
            Tag::HtmlBlock => self.html.clear(),
            Tag::List(start) => {
                self.flush();
                if self.lists.is_empty() {
                    self.block();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let depth = self.lists.len().saturating_sub(1);
                let marker = match self.lists.last_mut().and_then(Option::as_mut) {
                    Some(n) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    None => BULLETS[depth % BULLETS.len()].to_string(),
                };
                self.containers.push(Container::Item { marker, style: fg(ORANGE), used: false });
            }
            Tag::Emphasis => self.push_style(|s| s.add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(|s| s.add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(|s| s.add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => {
                self.links.push(web(&dest_url));
                self.push_style(|s| s.fg(BLUE).add_modifier(Modifier::UNDERLINED));
            }
            Tag::Image { dest_url, .. } => self.image = Some((web(&dest_url), String::new())),
            Tag::Table(aligns) => {
                self.flush();
                self.table = Some(Table { aligns, rows: Vec::new() });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    t.rows.push(Vec::new());
                }
            }
            Tag::TableCell => self.inline.clear(),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                self.gap = true;
            }
            TagEnd::Heading(level) => {
                self.styles.pop();
                self.heading(level);
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
                self.styles.pop();
                self.gap = true;
            }
            TagEnd::CodeBlock => {
                if let Some((info, text)) = self.code.take() {
                    self.code_block(&info, &text);
                }
            }
            TagEnd::HtmlBlock => {
                let html = std::mem::take(&mut self.html);
                self.html(&html);
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.gap = true;
                }
            }
            TagEnd::Item => {
                self.flush();
                // An item with nothing in it still shows its marker.
                if let Some(Container::Item { used: false, .. }) = self.containers.last() {
                    self.line(vec![]);
                }
                self.containers.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.links.pop();
                self.styles.pop();
            }
            TagEnd::Image => {
                if let Some((url, alt)) = self.image.take() {
                    let label = if alt.trim().is_empty() { "picture".to_string() } else { alt };
                    self.inline.push((format!("▣ {label}"), self.style().fg(DIM).add_modifier(Modifier::UNDERLINED), url));
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.table(table);
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inline);
                if let Some(row) = self.table.as_mut().and_then(|t| t.rows.last_mut()) {
                    row.push(cell);
                }
            }
            _ => {}
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => {
                let style = self.style().fg(ORANGE).bg(PANEL);
                let link = self.links.last().cloned().flatten();
                self.inline.push((code.to_string(), style, link));
            }
            Event::Html(html) => self.html.push_str(&html),
            Event::InlineHtml(html) => self.inline.push((html.to_string(), dim(), None)),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.inline.push(("\n".into(), Style::new(), None)),
            Event::Rule => self.rule(),
            Event::TaskListMarker(done) => {
                if let Some(Container::Item { marker, style, .. }) = self.containers.last_mut() {
                    (*marker, *style) = if done { ("☑ ".into(), fg(GREEN)) } else { ("☐ ".into(), dim()) };
                }
                if done {
                    self.push_style(|s| s.fg(DIM).add_modifier(Modifier::CROSSED_OUT));
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

pub fn markdown(text: &str, cols: usize) -> Vec<Row> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut writer = Writer { cols: cols.max(20), ..Writer::default() };
    // Each item's styles end with it, a checked task's struck style included.
    let mut item_styles = Vec::new();
    for event in Parser::new_ext(text, options) {
        match &event {
            Event::Start(Tag::Item) => item_styles.push(writer.styles.len()),
            Event::End(TagEnd::Item) => {
                if let Some(n) = item_styles.pop() {
                    writer.flush();
                    writer.styles.truncate(n);
                }
            }
            _ => {}
        }
        writer.event(event);
    }
    writer.flush();
    while writer.out.last().is_some_and(|r| r.line.width() == 0) {
        writer.out.pop();
    }
    writer.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Whatever the text and the width, drawing it ends without a panic.
        #[test]
        fn any_text_draws(text in r"([-*#>|`_~\\\[\]()!: \n0-9a-zé✓.x]|https://a\.b/c|```rust\n|- \[ \] |\|---\|){0,80}", cols in 0usize..160) {
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
}
