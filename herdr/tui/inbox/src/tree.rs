//! The inbox tree: rows known by their path (a stacked PR shows under each ticket its stack belongs to), folds and the
//! cursor kept across rebuilds, guides between levels, and the visible rows scrolled to keep the cursor in view. A
//! filter narrows it to the matching rows and their ancestors.

use std::collections::{HashMap, HashSet};

use cockpit_core::acts::{self, Act};
use cockpit_core::anim::Effects;
use cockpit_core::text::width;
use cockpit_core::theme::{BG, DIM, FG, ORANGE, SURFACE};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::filter;
use crate::requests::Requests;
use crate::row::Row;

const GUIDE: Color = Color::Rgb(64, 64, 70);
const THUMB: Color = Color::Rgb(70, 70, 76);
/// A selected row's background, warm like the cursor but faint.
const PICKED: Color = Color::Rgb(52, 42, 34);
const SCROLLBAR: u16 = 2;
/// The fold glyph's cells before a row's label.
const PREFIX: usize = 2;

pub struct Node {
    pub row: Row,
    pub path: String,
    pub parent: Option<usize>,
    pub kids: Vec<usize>,
}

pub struct Shown {
    pub node: usize,
    pub depth: usize,
    guide: String,
}

/// What a frame of the tree draws beside its rows.
pub struct Paint<'a> {
    pub requests: &'a Requests,
    /// Row keys picked for a batch action.
    pub picked: &'a HashSet<String>,
    pub effects: &'a mut Effects,
    /// The compact layout's buttons, drawn at the end of the cursor row.
    pub buttons: Option<&'a [Act]>,
}

#[derive(Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    tops: Vec<usize>,
    by_path: HashMap<String, usize>,
    opened: HashSet<String>,
    closed: HashSet<String>,
    cursor: String,
    offset: usize,
    /// Set when the cursor moved, so the next draw scrolls it into view; a wheel scroll leaves it off.
    follow: bool,
    pub area: Rect,
    /// Narrower guides, as the compact layout wants.
    pub compact: bool,
    query: String,
    /// The rows the filter matched, with their matched characters; and those plus their ancestors, which show.
    hits: HashMap<usize, Vec<usize>>,
    keep: HashSet<usize>,
    /// Rows unfolded while filtering, which then show all their children.
    unfolded: HashSet<String>,
}

impl Tree {
    /// New rows in place of the old ones, folds, cursor and filter kept by path.
    pub fn replace(&mut self, top: Vec<Row>) {
        self.nodes.clear();
        self.by_path.clear();
        self.tops = top.into_iter().map(|row| self.add(row, "", None)).collect();
        self.refilter();
        if !self.by_path.contains_key(&self.cursor) {
            self.cursor = self.tops.first().map(|&n| self.nodes[n].path.clone()).unwrap_or_default();
            self.follow = true;
        }
    }

    fn add(&mut self, mut row: Row, path: &str, parent: Option<usize>) -> usize {
        let path = format!("{path}/{}", row.key);
        let children = std::mem::take(&mut row.children);
        let at = self.nodes.len();
        self.nodes.push(Node { row, path: path.clone(), parent, kids: Vec::new() });
        self.by_path.insert(path.clone(), at);
        let kids = children.into_iter().map(|kid| self.add(kid, &path, Some(at))).collect();
        self.nodes[at].kids = kids;
        at
    }

    fn guide_depth(&self) -> usize {
        if self.compact { 2 } else { 3 }
    }

    pub fn filtering(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// Narrows the tree to the rows matching `query`, and puts the cursor on the first of them when it sat elsewhere.
    pub fn set_filter(&mut self, query: &str) {
        if query != self.query {
            self.unfolded.clear();
        }
        self.query = query.to_string();
        self.refilter();
        let on_match = self.node(&self.cursor.clone()).is_some_and(|n| self.hits.contains_key(&n));
        if self.filtering() && !on_match {
            let first = self.shown().into_iter().find(|s| self.hits.contains_key(&s.node)).map(|s| s.node);
            if let Some(node) = first {
                self.select(node);
            }
        }
    }

    pub fn matched(&self) -> usize {
        self.hits.len()
    }

    fn refilter(&mut self) {
        let haystacks: Vec<(usize, String)> = self.nodes.iter().enumerate().map(|(n, node)| (n, node.row.haystack())).collect();
        self.hits = filter::matches(&self.query, haystacks.iter().map(|(n, h)| (*n, h.as_str())));
        self.keep.clear();
        for &n in self.hits.keys() {
            let mut node = Some(n);
            while let Some(at) = node {
                if !self.keep.insert(at) {
                    break;
                }
                node = self.nodes[at].parent;
            }
        }
    }

    pub fn is_open(&self, node: usize) -> bool {
        let n = &self.nodes[node];
        !n.kids.is_empty() && (self.opened.contains(&n.path) || (n.row.expand && !self.closed.contains(&n.path)))
    }

    pub fn toggle(&mut self, node: usize) {
        if self.nodes[node].kids.is_empty() {
            return;
        }
        let path = self.nodes[node].path.clone();
        if self.filtering() {
            if !self.unfolded.remove(&path) {
                self.unfolded.insert(path);
            }
        } else if self.is_open(node) {
            self.opened.remove(&path);
            self.closed.insert(path);
        } else {
            self.closed.remove(&path);
            self.opened.insert(path);
        }
    }

    /// A row's children that show: while filtering, those kept and all of an unfolded row's; else all of an open row's.
    fn visible_kids(&self, node: usize) -> Vec<usize> {
        let n = &self.nodes[node];
        if self.filtering() {
            let all = self.unfolded.contains(&n.path);
            n.kids.iter().copied().filter(|k| all || self.keep.contains(k)).collect()
        } else if self.is_open(node) {
            n.kids.clone()
        } else {
            Vec::new()
        }
    }

    /// The visible rows, top to bottom, each with the guides drawn before it.
    pub fn shown(&self) -> Vec<Shown> {
        let mut out = Vec::new();
        for &top in self.tops.iter().filter(|t| !self.filtering() || self.keep.contains(t)) {
            self.walk(top, 0, "", true, &mut out);
        }
        out
    }

    fn walk(&self, node: usize, depth: usize, above: &str, last: bool, out: &mut Vec<Shown>) {
        let (branch, rest) = match (self.compact, last) {
            (false, true) => ("└─ ", "   "),
            (false, false) => ("├─ ", "│  "),
            (true, true) => ("└ ", "  "),
            (true, false) => ("├ ", "│ "),
        };
        let guide = if depth == 0 { String::new() } else { format!("{above}{branch}") };
        out.push(Shown { node, depth, guide });
        let kids = self.visible_kids(node);
        let below = if depth == 0 { String::new() } else { format!("{above}{rest}") };
        for (n, &kid) in kids.iter().enumerate() {
            self.walk(kid, depth + 1, &below, n + 1 == kids.len(), out);
        }
    }

    /// Where the cursor sits among the visible rows: its row, else its nearest visible ancestor, else the top.
    fn index(&self, shown: &[Shown]) -> usize {
        let mut node = self.by_path.get(&self.cursor).copied();
        while let Some(n) = node {
            if let Some(i) = shown.iter().position(|s| s.node == n) {
                return i;
            }
            node = self.nodes[n].parent;
        }
        0
    }

    pub fn current(&self) -> Option<usize> {
        let shown = self.shown();
        shown.get(self.index(&shown)).map(|s| s.node)
    }

    pub fn node(&self, path: &str) -> Option<usize> {
        self.by_path.get(path).copied()
    }

    /// The first row standing for this key (a PR URL, a ticket id).
    pub fn by_key(&self, key: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.row.key == key)
    }

    pub fn select(&mut self, node: usize) {
        self.cursor = self.nodes[node].path.clone();
        self.follow = true;
    }

    pub fn step(&mut self, delta: isize) {
        let shown = self.shown();
        if shown.is_empty() {
            return;
        }
        let at = (self.index(&shown) as isize + delta).clamp(0, shown.len() as isize - 1) as usize;
        self.select(shown[at].node);
    }

    pub fn page(&self) -> isize {
        self.area.height.max(2) as isize - 1
    }

    pub fn scroll(&mut self, lines: isize) {
        self.offset = (self.offset as isize + lines).max(0) as usize;
        self.follow = false;
    }

    /// Opens every row above this one and puts the cursor on it.
    pub fn go_to(&mut self, node: usize) {
        let mut parent = self.nodes[node].parent;
        while let Some(p) = parent {
            let path = self.nodes[p].path.clone();
            self.closed.remove(&path);
            self.opened.insert(path);
            parent = self.nodes[p].parent;
        }
        self.select(node);
    }

    /// The visible row at a screen position, and whether the position is on its fold glyph.
    pub fn at(&self, x: u16, y: u16) -> Option<(usize, bool)> {
        let area = self.area;
        if x < area.x || x >= area.x + area.width || y < area.y || y >= area.y + area.height {
            return None;
        }
        let shown = self.shown();
        let row = shown.get(self.offset + (y - area.y) as usize)?;
        let glyph = area.x as usize + row.depth * self.guide_depth();
        Some((row.node, (glyph..glyph + PREFIX).contains(&(x as usize))))
    }

    /// Draws the visible rows, the picked ones marked, effects over theirs; each compact button's place, for clicks.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, paint: Paint) -> Vec<(Rect, usize)> {
        let shown = self.shown();
        let height = area.height as usize;
        let bar = shown.len() > height && area.width > SCROLLBAR;
        let area = if bar { Rect { width: area.width - SCROLLBAR, ..area } } else { area };
        self.area = area;
        let at = self.index(&shown);
        if self.follow {
            if at < self.offset {
                self.offset = at;
            } else if at >= self.offset + height {
                self.offset = at + 1 - height;
            }
            self.follow = false;
        }
        self.offset = self.offset.min(shown.len().saturating_sub(height));
        let cursor = Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD);
        let mut clicks = Vec::new();
        let mut lines = Vec::new();
        for (i, s) in shown.iter().enumerate().skip(self.offset).take(height) {
            let node = &self.nodes[s.node];
            let has_kids = !node.kids.is_empty();
            let prefix = if !has_kids { "  " } else if self.visible_kids(s.node).is_empty() { "▶ " } else { "▼ " };
            let room = (area.width as usize).saturating_sub(s.depth * self.guide_depth() + PREFIX);
            let on_cursor = i == at;
            let buttons = paint.buttons.filter(|_| on_cursor).map(|acts| fitting(acts, room)).unwrap_or_default();
            let buttons_width: usize = buttons.iter().map(pill_width).sum();
            let mut spans = vec![Span::styled(s.guide.clone(), Style::new().fg(GUIDE)), Span::styled(prefix, Style::new().fg(FG))];
            let hits = self.hits.get(&s.node).map(Vec::as_slice).unwrap_or(&[]);
            let mut label = node.row.line(room, paint.requests, hits);
            // The compact layout's buttons cover the row's end, as the light app's do, so its cells stay in line.
            if !buttons.is_empty() {
                label.spans = cut(label.spans, room.saturating_sub(buttons_width + 1));
            }
            let is_context = self.filtering() && !self.hits.contains_key(&s.node);
            let is_picked = paint.picked.contains(&node.row.key);
            // A text row's colour sits on its line, which the spans moved into this one would drop.
            let base = label.style;
            spans.extend(label.spans.into_iter().map(|span| {
                let mut style = base.patch(span.style);
                if is_context {
                    style = style.fg(DIM);
                }
                if is_picked {
                    style = style.bg(PICKED);
                }
                span.style(if on_cursor { style.patch(cursor) } else { style })
            }));
            if on_cursor {
                spans[1].style = cursor;
            }
            let y = area.y + (i - self.offset) as u16;
            if !buttons.is_empty() {
                let used: usize = spans.iter().map(|s| width(&s.content)).sum();
                let start = area.x as usize + area.width as usize - buttons_width;
                spans.push(Span::raw(" ".repeat(start.saturating_sub(area.x as usize + used))));
                let mut x = start as u16;
                for act in &buttons {
                    let n = paint.buttons.unwrap_or_default().iter().position(|a| a.label == act.label && a.key == act.key).unwrap_or(0);
                    clicks.push((Rect { x, y, width: pill_width(act) as u16 - 1, height: 1 }, n));
                    x += pill_width(act) as u16;
                }
                spans.extend(acts::buttons(&buttons).spans);
            }
            lines.push(Line::from(spans));
        }
        frame.render_widget(Paragraph::new(lines), area);
        for (row, s) in shown.iter().skip(self.offset).take(height).enumerate() {
            let key = &self.nodes[s.node].row.key;
            let y = area.y + row as u16;
            if paint.picked.contains(key) && area.x > 0 {
                frame.render_widget(Paragraph::new(Span::styled("▌", Style::new().fg(ORANGE))), Rect { x: area.x - 1, y, width: 1, height: 1 });
            }
            paint.effects.paint(key, frame.buffer_mut(), Rect { y, height: 1, ..area });
        }
        if bar {
            scrollbar(frame, Rect { x: area.x + area.width, width: SCROLLBAR, ..area }, self.offset, shown.len());
        }
        clicks
    }
}

/// Spans cut to `cells` terminal cells.
fn cut(spans: Vec<Span<'static>>, cells: usize) -> Vec<Span<'static>> {
    let mut used = 0;
    let mut out = Vec::new();
    for span in spans {
        let w = width(&span.content);
        if used + w <= cells {
            used += w;
            out.push(span);
            continue;
        }
        let mut text = String::new();
        for c in span.content.chars() {
            let w = width(c.encode_utf8(&mut [0; 4]));
            if used + w > cells {
                break;
            }
            used += w;
            text.push(c);
        }
        out.push(Span::styled(text, span.style));
        break;
    }
    out
}

/// A button's cells, the gap after it included, as `acts::buttons` draws it.
pub fn pill_width(act: &Act) -> usize {
    let key = act.key.map(|k| format!("  {k}")).unwrap_or_default();
    width(&format!("  {}{key}  ", act.label)) + 1
}

/// The buttons that fit at a row's end, leaving the row at least half its room, the first ones kept.
fn fitting(acts: &[Act], room: usize) -> Vec<Act> {
    let mut used = 0;
    acts.iter()
        .take_while(|act| {
            used += pill_width(act);
            used <= room / 2
        })
        .cloned()
        .collect()
}

/// Textual's scrollbar: a thumb as tall as the share of rows in view, on a dark track.
fn scrollbar(frame: &mut Frame, area: Rect, offset: usize, total: usize) {
    let height = area.height as usize;
    let thumb = (height * height / total).max(1);
    let top = (offset * height).div_ceil(total).min(height - thumb);
    for y in 0..height {
        let color = if (top..top + thumb).contains(&y) { THUMB } else { SURFACE };
        let cell = Rect { y: area.y + y as u16, height: 1, ..area };
        frame.render_widget(Block::new().style(Style::new().bg(color)), cell);
    }
}
