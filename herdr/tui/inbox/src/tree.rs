//! The inbox tree: rows known by their path (a stacked PR shows under each ticket its stack belongs to), folds and the
//! cursor kept across rebuilds, guides between levels, and the visible rows scrolled to keep the cursor in view.

use std::collections::{HashMap, HashSet};

use cockpit_core::theme::{BG, FG, ORANGE, SURFACE};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::requests::Requests;
use crate::row::Row;

const GUIDE: Color = Color::Rgb(64, 64, 70);
const THUMB: Color = Color::Rgb(70, 70, 76);
const SCROLLBAR: u16 = 2;
const GUIDE_DEPTH: usize = 3;
/// The fold glyph's cells before a row's label.
const PREFIX: usize = 2;

pub struct Node {
    pub row: Row,
    pub path: String,
    pub parent: Option<usize>,
    pub kids: Vec<usize>,
    last: bool,
}

pub struct Shown {
    pub node: usize,
    pub depth: usize,
    guide: String,
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
}

impl Tree {
    /// New rows in place of the old ones, folds and cursor kept by path.
    pub fn replace(&mut self, top: Vec<Row>) {
        self.nodes.clear();
        self.by_path.clear();
        let count = top.len();
        self.tops = top.into_iter().enumerate().map(|(n, row)| self.add(row, "", None, n + 1 == count)).collect();
        if !self.by_path.contains_key(&self.cursor) {
            self.cursor = self.tops.first().map(|&n| self.nodes[n].path.clone()).unwrap_or_default();
            self.follow = true;
        }
    }

    fn add(&mut self, mut row: Row, path: &str, parent: Option<usize>, last: bool) -> usize {
        let path = format!("{path}/{}", row.key);
        let children = std::mem::take(&mut row.children);
        let at = self.nodes.len();
        self.nodes.push(Node { row, path: path.clone(), parent, kids: Vec::new(), last });
        self.by_path.insert(path.clone(), at);
        let count = children.len();
        let kids = children.into_iter().enumerate().map(|(n, kid)| self.add(kid, &path, Some(at), n + 1 == count)).collect();
        self.nodes[at].kids = kids;
        at
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
        if self.is_open(node) {
            self.opened.remove(&path);
            self.closed.insert(path);
        } else {
            self.closed.remove(&path);
            self.opened.insert(path);
        }
    }

    /// The visible rows, top to bottom, each with the guides drawn before it.
    pub fn shown(&self) -> Vec<Shown> {
        let mut out = Vec::new();
        for &top in &self.tops {
            self.walk(top, 0, "", &mut out);
        }
        out
    }

    fn walk(&self, node: usize, depth: usize, above: &str, out: &mut Vec<Shown>) {
        let last = self.nodes[node].last;
        let guide = if depth == 0 { String::new() } else { format!("{above}{}", if last { "└─ " } else { "├─ " }) };
        out.push(Shown { node, depth, guide });
        if self.is_open(node) {
            let below = if depth == 0 { String::new() } else { format!("{above}{}", if last { "   " } else { "│  " }) };
            for &kid in &self.nodes[node].kids {
                self.walk(kid, depth + 1, &below, out);
            }
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
        let glyph = area.x as usize + row.depth * GUIDE_DEPTH;
        Some((row.node, (glyph..glyph + PREFIX).contains(&(x as usize))))
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, requests: &Requests) {
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
        let lines: Vec<Line> = shown
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(height)
            .map(|(i, s)| {
                let node = &self.nodes[s.node];
                let prefix = if node.kids.is_empty() { "  " } else if self.is_open(s.node) { "▼ " } else { "▶ " };
                let room = (area.width as usize).saturating_sub(s.depth * GUIDE_DEPTH + PREFIX);
                let mut spans = vec![Span::styled(s.guide.clone(), Style::new().fg(GUIDE)), Span::styled(prefix, Style::new().fg(FG))];
                let label = node.row.line(room, requests);
                if i == at {
                    spans.extend(label.spans.into_iter().map(|span| {
                        let style = span.style.patch(cursor);
                        span.style(style)
                    }));
                    spans[1].style = cursor;
                } else {
                    spans.extend(label.spans);
                }
                Line::from(spans)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
        if bar {
            scrollbar(frame, Rect { x: area.x + area.width, width: SCROLLBAR, ..area }, self.offset, shown.len());
        }
    }
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
