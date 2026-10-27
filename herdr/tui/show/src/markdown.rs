//! show.py's Markdown: headings, bullets, quotes, code blocks, rules, pipe tables and inline **bold**, *italic*,
//! ~~struck~~, `code`, [links](url) and ![pictures](url), wrapped to the pane.

use std::sync::LazyLock;

use ratatui::style::{Modifier, Style};
use regex::Regex;

use cockpit_core::text::{fit, width};
use cockpit_core::theme::{DIM, FG, ORANGE, bold, dim, fg};

use crate::row::{Act, Row};

static INLINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\*\*(?<bold>.+?)\*\*|__(?<bold2>.+?)__|`(?<code>[^`]+)`|~~(?<struck>.+?)~~",
        r"|!\[(?<alt>[^\]]*)\]\(<?(?<src>[^)\s>]*)>?\)|\[(?<text>[^\]]+)\]\(<?(?<url>[^)\s>]*)>?\)",
        r"|\*(?<em>[^*\s](?:[^*]*[^*\s])?)\*|\b_(?<em2>[^_\s](?:[^_]*[^_\s])?)_\b",
    ))
    .unwrap()
});
static ESCAPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\([\\`*_{}\[\]()#+\-.!~|>])").unwrap());
static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.*?)(?:\s+#+)?\s*$").unwrap());
static BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)([-*•]|\d+[.)])\s+(.*)").unwrap());
static RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(-{3,}|\*{3,}|_{3,})\s*$").unwrap());
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\S+\s*|\s+").unwrap());
static MARKERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*|`([^`]+)`|!?\[([^\]]*)\]\([^)]*\)|~~(.+?)~~").unwrap());

pub type Segment = (String, Style, Option<String>);

/// Where an escaped character waits while the line is parsed, so `\*` is never italic nor `\#` a heading.
const HIDDEN: u32 = 0xF0000;

fn hide(raw: &str) -> String {
    ESCAPE.replace_all(raw, |m: &regex::Captures| char::from_u32(HIDDEN + m[1].chars().next().map_or(0, u32::from)).unwrap_or(' ').to_string()).into_owned()
}

/// The escaped characters back as themselves.
fn unhide(text: &str) -> String {
    text.chars().map(|c| if (HIDDEN..HIDDEN + 128).contains(&u32::from(c)) { char::from_u32(u32::from(c) - HIDDEN).unwrap_or(c) } else { c }).collect()
}

/// Segments of one line: **bold**, *italic*, ~~struck~~, `code`, [links](url) and ![pictures](url), each with the
/// web address a click opens.
fn inline(text: &str, style: Style) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut position = 0;
    let web = |url: &str| (url.starts_with("http://") || url.starts_with("https://")).then(|| url.to_string());
    for m in INLINE.captures_iter(text) {
        let whole = m.get(0).unwrap();
        if whole.start() > position {
            out.push((text[position..whole.start()].to_string(), style, None));
        }
        let group = |name: &str| m.name(name).map(|g| g.as_str().to_string());
        if let Some(b) = group("bold").or_else(|| group("bold2")) {
            out.push((b, style.add_modifier(Modifier::BOLD), None));
        } else if let Some(c) = group("code") {
            out.push((c, fg(ORANGE), None));
        } else if let Some(s) = group("struck") {
            out.push((s, style.add_modifier(Modifier::CROSSED_OUT), None));
        } else if let Some(alt) = group("alt") {
            let src = group("src").unwrap_or_default();
            let label = if alt.is_empty() { "picture".to_string() } else { alt };
            out.push((format!("▣ {label}"), style.fg(DIM).add_modifier(Modifier::UNDERLINED), web(&src)));
        } else if let Some(link) = group("text") {
            out.push((link, style.add_modifier(Modifier::UNDERLINED), web(&group("url").unwrap_or_default())));
        } else if let Some(em) = group("em").or_else(|| group("em2")) {
            out.push((em, style.add_modifier(Modifier::ITALIC), None));
        }
        position = whole.end();
    }
    if position < text.len() {
        out.push((text[position..].to_string(), style, None));
    }
    out.into_iter().map(|(text, style, url)| (unhide(&text), style, url)).collect()
}

/// Splits styled segments into rows of at most `room` cells, breaking between words.
pub fn wrap(segments: Vec<Segment>, room: usize) -> Vec<Vec<Segment>> {
    let room = room.max(1);
    let (mut rows, mut row, mut used) = (Vec::new(), Vec::new(), 0);
    for (text, style, url) in segments {
        for word in WORD.find_iter(&text) {
            let mut word = word.as_str().to_string();
            if used > 0 && used + width(word.trim_end()) > room {
                rows.push(std::mem::take(&mut row));
                used = 0;
                word = word.trim_start().to_string();
            }
            while width(&word) > room {
                let (head, rest) = split_at_cells(&word, room);
                row.push((head, style, url.clone()));
                rows.push(std::mem::take(&mut row));
                used = 0;
                word = rest;
            }
            used += width(&word);
            row.push((word, style, url.clone()));
        }
    }
    rows.push(row);
    rows
}

fn split_at_cells(text: &str, cells: usize) -> (String, String) {
    let mut used = 0;
    let at = text.char_indices().find(|(_, c)| {
        used += width(&c.to_string());
        used > cells
    });
    let at = at.map_or(text.len(), |(i, _)| i).max(text.chars().next().map_or(0, char::len_utf8));
    (text[..at].to_string(), text[at..].to_string())
}

fn row_of(prefix: &str, prefix_style: Style, segments: Vec<Segment>) -> Row {
    let mut row = Row::text(prefix, prefix_style);
    for (text, style, url) in segments {
        row = match url {
            Some(url) => row.act(text, style, Act::Open(url)),
            None => row.add(text, style),
        };
    }
    row
}

/// Inline Markdown markers dropped, for table cells.
fn plain(text: &str) -> String {
    MARKERS.replace_all(text, |m: &regex::Captures| m.get(1).or(m.get(2)).or(m.get(3)).or(m.get(4)).map_or("", |g| g.as_str()).to_string()).into_owned()
}

/// A pipe table, its widest columns giving way first when the pane is narrow.
fn pipe_table(lines: &[String], cols: usize, out: &mut Vec<Row>) {
    let cells: Vec<Vec<String>> = lines
        .iter()
        .map(|r| r.trim().trim_matches('|').split('|').map(|c| unhide(&plain(c.trim()))).collect::<Vec<_>>())
        .filter(|r: &Vec<String>| !r.concat().chars().all(|c| "-: ".contains(c)))
        .collect();
    let Some(wide) = cells.iter().map(Vec::len).max() else { return };
    let cell = |r: &Vec<String>, i: usize| r.get(i).cloned().unwrap_or_default();
    let mut sizes: Vec<usize> = (0..wide).map(|i| cells.iter().map(|r| width(&cell(r, i))).max().unwrap_or(0)).collect();
    let room = cols.saturating_sub(2 * (wide - 1));
    while sizes.iter().sum::<usize>() > room && sizes.iter().max().copied().unwrap_or(0) > 6 {
        let i = sizes.iter().enumerate().max_by_key(|(i, s)| (**s, usize::MAX - i)).map(|(i, _)| i).unwrap_or(0);
        sizes[i] -= 1;
    }
    let line = |r: &Vec<String>, style: Style| {
        let text: Vec<String> = (0..wide).map(|i| fit(&cell(r, i), sizes[i])).collect();
        Row::text(text.join("  "), style)
    };
    out.push(line(&cells[0], bold(DIM)));
    out.push(Row::text("─".repeat(cols.min(sizes.iter().sum::<usize>() + 2 * (wide - 1))), dim()));
    for r in &cells[1..] {
        out.push(line(r, fg(FG)));
    }
}

pub fn markdown(text: &str, cols: usize) -> Vec<Row> {
    let room = cols.max(20);
    let mut out = Vec::new();
    let (mut in_code, mut pending) = (false, Vec::new());
    for raw in text.lines() {
        let raw = if in_code { raw.to_string() } else { hide(raw) };
        if raw.trim().starts_with('|') && !in_code {
            pending.push(raw);
            continue;
        }
        if !pending.is_empty() {
            pipe_table(&std::mem::take(&mut pending), cols, &mut out);
        }
        if raw.trim().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push(Row::text("│ ", dim()).add(fit(&raw.replace('\t', "    "), room.saturating_sub(2)), dim()));
        } else if RULE.is_match(&raw) {
            out.push(Row::text("─".repeat(room.min(40)), dim()));
        } else if let Some(h) = HEADING.captures(&raw) {
            let style = match h[1].len() {
                1 => bold(ORANGE),
                2 | 3 => Style::new().add_modifier(Modifier::BOLD),
                _ => Style::new().add_modifier(Modifier::BOLD | Modifier::ITALIC),
            };
            out.push(row_of("", Style::new(), inline(&h[2], style)));
        } else if let Some(b) = BULLET.captures(&raw) {
            let mark = if "-*•".contains(&b[2]) { "• ".to_string() } else { format!("{} ", &b[2]) };
            let indent = format!("{}{mark}", &b[1]);
            for (n, segments) in wrap(inline(&b[3], Style::new()), room.saturating_sub(width(&indent))).into_iter().enumerate() {
                let prefix = if n == 0 { indent.clone() } else { " ".repeat(width(&indent)) };
                out.push(row_of(&prefix, dim(), segments));
            }
        } else if let Some(quote) = raw.strip_prefix('>') {
            for segments in wrap(inline(quote.trim_start_matches(['>', ' ']), dim()), room.saturating_sub(2)) {
                out.push(row_of("▎ ", dim(), segments));
            }
        } else if raw.trim().is_empty() {
            out.push(Row::new());
        } else {
            for segments in wrap(inline(&raw, Style::new()), room) {
                out.push(row_of("", Style::new(), segments));
            }
        }
    }
    if !pending.is_empty() {
        pipe_table(&pending, cols, &mut out);
    }
    out
}
