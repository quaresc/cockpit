//! A diff: a patch given whole, or a repo's working tree read again every few seconds in the background. Each file
//! folds; lines carry their syntax colours and changed words, wrap rather than get cut, and show unified or side by side
//! (`s`). Focused, the diff has a line cursor (j/k, `[`/`]` between files) and `c` comments on the line under it.

use std::collections::{BTreeSet, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier, Style};
use serde_json::{Value, json};

use cockpit_core::input::{Edit, Input};
use cockpit_core::text::{fit, width};
use cockpit_core::theme::{BG, DIM, FG, GREEN, ORANGE, RED, SURFACE, bold, dim, fg};

use crate::markdown::split_at_cells;
use crate::patch::{File, Kind, Line};
use crate::question::Key;
use crate::row::{Act, Row, Target};

/// A repo diff is read again this often, so the pane follows the agent's edits.
const EVERY: Duration = Duration::from_secs(3);
/// Side by side needs room for two readable halves.
const SIDE_BY_SIDE: usize = 100;
/// The file list at the top shows this many files, then how many more.
const LISTED: usize = 12;
const ADD_BG: Color = Color::Rgb(18, 44, 38);
const ADD_WORD: Color = Color::Rgb(32, 92, 74);
const DEL_BG: Color = Color::Rgb(52, 22, 26);
const DEL_WORD: Color = Color::Rgb(112, 40, 46);
const CURSOR_BLURRED: Color = Color::Rgb(70, 56, 44);

/// A patch's text hashed, so a repo read that found the same changes paints nothing again.
pub fn hash(patch: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    patch.hash(&mut hasher);
    hasher.finish()
}

pub enum Source {
    Patch(String),
    Repo { repo: String, base: Option<String> },
}

type Pieces = Vec<(String, Style)>;

pub struct Diff {
    pub source: Source,
    shown: Option<u64>,
    error: Option<String>,
    files: Vec<File>,
    folded: HashSet<String>,
    reading: bool,
    read_at: Option<Instant>,
    /// The file and line under the cursor; no line is the file's header.
    cursor: (usize, Option<usize>),
    side_by_side: bool,
    comment: Option<Input>,
    /// Comments sent from here: path, line, whether on the old side, text; shown under their line.
    sent: Vec<(String, usize, bool, String)>,
}

/// A line's number for a comment: the new file's, or the old one's for a removed line.
fn number(line: &Line) -> Option<usize> {
    match line.kind {
        Kind::Add | Kind::Same => line.new,
        Kind::Del => line.old,
        _ => None,
    }
}

fn is_stop(line: &Line) -> bool {
    matches!(line.kind, Kind::Add | Kind::Del | Kind::Same)
}

/// The line's coloured pieces wrapped to `room` cells, on its kind's background, changed words stronger.
fn wrapped(line: &Line, room: usize) -> Vec<Pieces> {
    let (bg, word) = match line.kind {
        Kind::Add => (Some(ADD_BG), ADD_WORD),
        Kind::Del => (Some(DEL_BG), DEL_WORD),
        _ => (None, BG),
    };
    let room = room.max(1);
    let (mut rows, mut row, mut used): (Vec<Pieces>, Pieces, usize) = (Vec::new(), Vec::new(), 0);
    for (text, style, strong) in &line.paint {
        let style = match (bg, strong) {
            (Some(_), true) => style.bg(word),
            (Some(bg), false) => style.bg(bg),
            _ => *style,
        };
        let mut text = text.clone();
        while used + width(&text) > room {
            if used < room {
                let (head, rest) = split_at_cells(&text, room - used);
                row.push((head, style));
                text = rest;
            }
            rows.push(std::mem::take(&mut row));
            used = 0;
        }
        used += width(&text);
        row.push((text, style));
    }
    rows.push(row);
    for row in &mut rows {
        let used: usize = row.iter().map(|(t, _)| width(t)).sum();
        row.push((" ".repeat(room.saturating_sub(used)), bg.map_or(Style::new(), |bg| Style::new().bg(bg))));
    }
    rows
}

fn sign(kind: Kind) -> (&'static str, Style) {
    match kind {
        Kind::Add => ("+", fg(GREEN).bg(ADD_BG)),
        Kind::Del => ("-", fg(RED).bg(DEL_BG)),
        _ => (" ", Style::new()),
    }
}

/// A hunk header or a note, across the whole width.
fn across(line: &Line, cols: usize) -> Row {
    let text: String = line.paint.iter().map(|p| p.0.as_str()).collect();
    Row::text(fit(&format!(" {text}"), cols).trim_end().to_string(), line.paint.first().map_or(dim(), |p| p.1)).target(Target::Diff)
}

/// What one row of the side-by-side view holds: a line across both halves, or a removed and an added line facing.
enum Pair {
    Across(usize),
    Facing(Option<usize>, Option<usize>),
}

fn pairs(lines: &[Line]) -> Vec<Pair> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        match lines[i].kind {
            Kind::Hunk | Kind::Note => {
                out.push(Pair::Across(i));
                i += 1;
            }
            Kind::Same => {
                out.push(Pair::Facing(Some(i), Some(i)));
                i += 1;
            }
            Kind::Del | Kind::Add => {
                let dels = lines[i..].iter().take_while(|l| l.kind == Kind::Del).count();
                let adds = lines[i + dels..].iter().take_while(|l| l.kind == Kind::Add).count();
                for k in 0..dels.max(adds) {
                    out.push(Pair::Facing((k < dels).then_some(i + k), (k < adds).then_some(i + dels + k)));
                }
                i += dels + adds;
            }
        }
    }
    out
}

impl Diff {
    pub fn new(block: &Value) -> Self {
        let source = match block["patch"].as_str() {
            Some(patch) => Source::Patch(patch.to_string()),
            None => Source::Repo { repo: block["repo"].as_str().unwrap_or_default().to_string(), base: block["base"].as_str().map(str::to_string) },
        };
        Self {
            source,
            shown: None,
            error: None,
            files: Vec::new(),
            folded: HashSet::new(),
            reading: false,
            read_at: None,
            cursor: (0, None),
            side_by_side: false,
            comment: None,
            sent: Vec::new(),
        }
    }

    /// The repo of a diff read from the working tree, for reviewr.
    pub fn repo(&self) -> Option<&str> {
        if let Source::Repo { repo, .. } = &self.source { Some(repo) } else { None }
    }

    /// True when a background read is due: a given patch once, a repo every few seconds.
    pub fn due(&mut self) -> bool {
        let again = matches!(self.source, Source::Repo { .. }) || (self.shown.is_none() && self.read_at.is_none());
        if self.reading || !again || self.read_at.is_some_and(|at| at.elapsed() < EVERY) {
            return false;
        }
        (self.reading, self.read_at) = (true, Some(Instant::now()));
        true
    }

    /// What the last read showed, for the next one to say "unchanged".
    pub fn shown(&self) -> Option<u64> {
        self.shown
    }

    /// A read came back: new files (None when the patch had not changed) or an error; false when nothing changed.
    pub fn arrived(&mut self, hash: u64, files: Option<Vec<File>>, error: Option<String>) -> bool {
        self.reading = false;
        if let Some(error) = error {
            let changed = self.error.as_ref() != Some(&error);
            (self.error, self.shown) = (Some(error), None);
            return changed;
        }
        let Some(files) = files else { return false };
        (self.files, self.shown, self.error) = (files, Some(hash), None);
        self.clamp();
        true
    }

    fn clamp(&mut self) {
        let Some(file) = self.files.get(self.cursor.0) else {
            self.cursor = (self.files.len().saturating_sub(1), None);
            return;
        };
        if self.cursor.1.is_some_and(|l| file.lines.get(l).is_none_or(|l| !is_stop(l))) {
            self.cursor.1 = None;
        }
    }

    pub fn targets(&self) -> Vec<Target> {
        if self.error.is_none() && !self.files.is_empty() { vec![Target::Diff] } else { vec![] }
    }

    /// True while a comment is typed: every key goes to its field.
    pub fn is_typing(&self) -> bool {
        self.comment.is_some()
    }

    /// Every place the cursor can stop, in order: each file's header, then its lines unless it is folded.
    fn stops(&self) -> Vec<(usize, Option<usize>)> {
        let mut out = Vec::new();
        for (f, file) in self.files.iter().enumerate() {
            out.push((f, None));
            if !self.folded.contains(&file.path) {
                out.extend(file.lines.iter().enumerate().filter(|(_, l)| is_stop(l)).map(|(l, _)| (f, Some(l))));
            }
        }
        out
    }

    fn step(&mut self, by: isize) {
        let stops = self.stops();
        let at = stops.iter().position(|s| *s == self.cursor).unwrap_or(0);
        let next = at.saturating_add_signed(by).min(stops.len().saturating_sub(1));
        if let Some(stop) = stops.get(next) {
            self.cursor = *stop;
        }
    }

    fn jump_file(&mut self, forward: bool) {
        let f = self.cursor.0;
        self.cursor = match (forward, self.cursor.1) {
            (true, _) => ((f + 1).min(self.files.len().saturating_sub(1)), None),
            (false, Some(_)) => (f, None),
            (false, None) => (f.saturating_sub(1), None),
        };
    }

    fn fold(&mut self, f: usize) {
        let Some(path) = self.files.get(f).map(|file| file.path.clone()) else { return };
        if !self.folded.remove(&path) {
            self.folded.insert(path);
            self.cursor = (f, None);
        }
    }

    fn start_comment(&mut self) {
        let line = self.files.get(self.cursor.0).and_then(|f| self.cursor.1.and_then(|l| f.lines.get(l)));
        if line.is_some_and(is_stop) {
            self.comment = Some(Input::default());
        }
    }

    fn send_comment(&mut self) -> Key {
        let text = self.comment.take().map(|i| i.text.trim().to_string()).unwrap_or_default();
        let file = &self.files[self.cursor.0];
        let line = self.cursor.1.and_then(|l| file.lines.get(l)).and_then(number);
        let Some(line) = line.filter(|_| !text.is_empty()) else { return Key::Used };
        let old = self.cursor.1.and_then(|l| file.lines.get(l)).is_some_and(|l| l.kind == Kind::Del);
        self.sent.push((file.path.clone(), line, old, text.clone()));
        Key::Comment(json!({"path": file.path, "line": line, "text": text}), format!("{}:{line}", file.path))
    }

    pub fn act(&mut self, act: &Act) {
        match act {
            Act::Fold(f) => self.fold(*f),
            Act::Line(f, l) => self.cursor = (*f, *l),
            Act::Jump(f) => self.cursor = (*f, None),
            _ => {}
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Key {
        if let Some(input) = self.comment.as_mut() {
            return match input.key(key) {
                Edit::Submit => self.send_comment(),
                Edit::Cancel => {
                    self.comment = None;
                    Key::Used
                }
                Edit::Changed | Edit::Ignored => Key::Used,
            };
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.step(1),
            KeyCode::Char('k') | KeyCode::Up => self.step(-1),
            KeyCode::Char('g') | KeyCode::Home => self.cursor = (0, None),
            KeyCode::Char('G') | KeyCode::End => self.step(isize::MAX),
            KeyCode::Char(']') => self.jump_file(true),
            KeyCode::Char('[') => self.jump_file(false),
            KeyCode::Char('s') => self.side_by_side = !self.side_by_side,
            KeyCode::Enter if self.cursor.1.is_none() => self.fold(self.cursor.0),
            KeyCode::Char('c') | KeyCode::Enter => self.start_comment(),
            _ => return Key::Unused,
        }
        Key::Used
    }

    pub fn rows(&self, cols: usize, focus: Option<&Target>) -> Vec<Row> {
        if let Some(error) = &self.error {
            return crate::blocks::error_rows(error, cols);
        }
        if self.shown.is_none() {
            let what = self.repo().map_or_else(|| "the patch".to_string(), str::to_string);
            return vec![Row::text(format!("reading {what}…"), dim())];
        }
        let focused = focus == Some(&Target::Diff);
        let wide = self.side_by_side && cols >= SIDE_BY_SIDE;
        let (added, removed): (usize, usize) = (self.files.iter().map(|f| f.added).sum(), self.files.iter().map(|f| f.removed).sum());
        let count = self.files.len();
        let mode = match (self.side_by_side, wide) {
            (true, true) => "side by side · s unified",
            (true, false) => "side by side needs a wider pane",
            _ => "unified · s side by side",
        };
        let mut out = vec![
            Row::text(format!("{count} file{}  ", if count == 1 { "" } else { "s" }), dim())
                .add(format!("+{added}"), fg(GREEN))
                .add(" ", Style::new())
                .add(format!("−{removed}"), fg(RED))
                .add(format!("   {mode}"), dim())
                .target(Target::Diff),
        ];
        if count > 1 {
            for (f, file) in self.files.iter().enumerate().take(LISTED) {
                let here = focused && self.cursor.0 == f;
                out.push(
                    Row::text(if here { "  ▸ " } else { "    " }, fg(ORANGE))
                        .act(file.path.clone(), fg(if here { ORANGE } else { FG }), Act::Jump(f))
                        .add(format!("  +{}", file.added), fg(GREEN))
                        .add(format!(" −{}", file.removed), fg(RED))
                        .target(Target::Diff),
                );
            }
            if count > LISTED {
                out.push(Row::text(format!("    … and {} more, ] goes through them", count - LISTED), dim()).target(Target::Diff));
            }
        }
        for (f, file) in self.files.iter().enumerate() {
            out.push(Row::new().target(Target::Diff));
            let folded = self.folded.contains(&file.path);
            let on_header = self.cursor == (f, None);
            let style = match (on_header, focused) {
                (true, true) => Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD),
                (true, false) => Style::new().fg(FG).bg(CURSOR_BLURRED),
                _ => bold(FG),
            };
            let title = format!(" {} {}   +{} −{} ", if folded { "▶" } else { "▼" }, file.path, file.added, file.removed);
            out.push(Row::new().act(title, style, Act::Fold(f)).target(Target::Diff).here(on_header));
            if folded {
                continue;
            }
            let digits = file.lines.iter().filter_map(|l| l.old.max(l.new)).max().unwrap_or(0).to_string().len().max(3);
            if wide {
                self.side_by_side_rows(f, file, cols, digits, focused, &mut out);
            } else {
                self.unified_rows(f, file, cols, digits, focused, &mut out);
            }
        }
        out
    }

    /// The cursor's mark in the gutter: orange when the diff has the keyboard.
    fn gutter(&self, f: usize, l: usize, first: bool, focused: bool) -> (String, Style) {
        match (self.cursor == (f, Some(l)) && first, focused) {
            (true, true) => ("▌".into(), fg(ORANGE)),
            (true, false) => ("▌".into(), fg(CURSOR_BLURRED)),
            _ => (" ".into(), Style::new()),
        }
    }

    fn unified_rows(&self, f: usize, file: &File, cols: usize, digits: usize, focused: bool, out: &mut Vec<Row>) {
        let room = cols.saturating_sub(2 * digits + 5);
        let num = |n: Option<usize>| n.map_or(" ".repeat(digits), |n| format!("{n:>digits$}"));
        for (l, line) in file.lines.iter().enumerate() {
            if !is_stop(line) {
                out.push(across(line, cols));
                continue;
            }
            let (mark, mark_style) = sign(line.kind);
            for (k, pieces) in wrapped(line, room).into_iter().enumerate() {
                let (gutter, gutter_style) = self.gutter(f, l, k == 0, focused);
                let numbers = if k == 0 { format!("{} {} ", num(line.old), num(line.new)) } else { " ".repeat(2 * digits + 2) };
                let mut row = Row::text(gutter, gutter_style)
                    .act(numbers, dim(), Act::Line(f, Some(l)))
                    .add(if k == 0 { mark } else { " " }, mark_style)
                    .add(" ", mark_style)
                    .target(Target::Diff)
                    .here(k == 0 && self.cursor == (f, Some(l)));
                let start = row.line.width();
                for (text, style) in pieces {
                    row = row.add(text, style);
                }
                row.clicks.push((start, cols, Act::Line(f, Some(l))));
                out.push(row);
            }
            self.after_line(file, line, l, f, cols, out);
        }
    }

    /// One half of a side-by-side row: a line's wrapped rows with their gutter, or blank room when the side has none.
    fn side(&self, (f, file): (usize, &File), l: Option<usize>, old: bool, half: usize, digits: usize, focused: bool) -> Vec<Pieces> {
        let Some(l) = l else { return vec![vec![(" ".repeat(half), Style::new().bg(SURFACE))]] };
        let line = &file.lines[l];
        let (mark, mark_style) = sign(line.kind);
        wrapped(line, half.saturating_sub(digits + 4))
            .into_iter()
            .enumerate()
            .map(|(k, pieces)| {
                let (gutter, gutter_style) = self.gutter(f, l, k == 0, focused);
                let n = match (k, if old { line.old } else { line.new }) {
                    (0, Some(n)) => format!("{n:>digits$} "),
                    _ => " ".repeat(digits + 1),
                };
                let mut row = vec![(gutter, gutter_style), (n, dim()), (if k == 0 { mark } else { " " }.to_string(), mark_style), (" ".into(), mark_style)];
                row.extend(pieces);
                row
            })
            .collect()
    }

    fn side_by_side_rows(&self, f: usize, file: &File, cols: usize, digits: usize, focused: bool, out: &mut Vec<Row>) {
        let half = (cols - 1) / 2;
        for pair in pairs(&file.lines) {
            let (left, right) = match pair {
                Pair::Across(l) => {
                    out.push(across(&file.lines[l], cols));
                    continue;
                }
                Pair::Facing(left, right) => (left, right),
            };
            let (a, b) = (self.side((f, file), left, true, half, digits, focused), self.side((f, file), right, false, half, digits, focused));
            let filler = |rows: &Vec<Pieces>| vec![(" ".repeat(half), rows.first().and_then(|r| r.last()).map_or(Style::new(), |p| p.1))];
            for k in 0..a.len().max(b.len()) {
                let here = k == 0 && [left, right].iter().any(|s| s.is_some_and(|l| self.cursor == (f, Some(l))));
                let mut row = Row::new().target(Target::Diff).here(here);
                for (side, rows, l) in [(0, &a, left.or(right)), (1, &b, right.or(left))] {
                    if side == 1 {
                        row = row.add("│", fg(DIM));
                    }
                    let start = row.line.width();
                    for (text, style) in rows.get(k).cloned().unwrap_or_else(|| filler(rows)) {
                        row = row.add(text, style);
                    }
                    row.clicks.push((start, start + half, Act::Line(f, l)));
                }
                out.push(row);
            }
            for l in [left, right].into_iter().flatten().collect::<BTreeSet<_>>() {
                self.after_line(file, &file.lines[l], l, f, cols, out);
            }
        }
    }

    /// Under a line: the comments sent on it, and the comment being typed when the cursor is on it.
    fn after_line(&self, file: &File, line: &Line, l: usize, f: usize, cols: usize, out: &mut Vec<Row>) {
        let Some(n) = number(line) else { return };
        let old = line.kind == Kind::Del;
        for (_, _, _, text) in self.sent.iter().filter(|(p, at, side, _)| *p == file.path && *at == n && *side == old) {
            for (k, piece) in cockpit_core::text::wrap(text, cols.saturating_sub(8)).into_iter().enumerate() {
                out.push(Row::text(if k == 0 { "    ✓ " } else { "      " }, fg(GREEN)).add(piece, dim().add_modifier(Modifier::ITALIC)).target(Target::Diff));
            }
        }
        if let (Some(input), true) = (&self.comment, self.cursor == (f, Some(l))) {
            let mut row = input.line("    ✎ ");
            row.spans.push(ratatui::text::Span::styled("   enter sends to the session · esc cancels", dim()));
            out.push(Row::from_line(row).target(Target::Diff).here(true));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn press(diff: &mut Diff, code: KeyCode) -> Key {
        diff.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn a_comment_names_the_line_under_the_cursor() {
        let patch = "diff --git a/a.py b/a.py\n--- a/a.py\n+++ b/a.py\n@@ -1,2 +1,2 @@\n x = 1\n-y = 2\n+y = 3\n";
        let mut diff = Diff::new(&json!({"patch": patch}));
        diff.arrived(1, Some(crate::patch::prepare(patch)), None);
        for code in [KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('c'), KeyCode::Char('o'), KeyCode::Char('k')] {
            press(&mut diff, code);
        }
        match press(&mut diff, KeyCode::Enter) {
            Key::Comment(value, label) => {
                assert_eq!(value, json!({"path": "a.py", "line": 2, "text": "ok"}));
                assert_eq!(label, "a.py:2");
            }
            _ => panic!("no comment sent"),
        }
    }
}
