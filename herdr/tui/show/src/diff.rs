//! A diff: a patch given whole, or a repo's working tree read again every few seconds in the background. Each file
//! folds; lines carry their syntax colours and changed words, wrap rather than get cut, and show unified or side by side
//! (`s`). Focused, the diff has a line cursor (j/k, `[`/`]` between files, `n`/`p` between hunks), `c` comments on the
//! line under it and `S` sends the comments written; it can show only some hunks, a walkthrough step's.

use std::collections::{BTreeSet, HashMap, HashSet};
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

/// A size as people read it: "980 B", "12 KB", "1.4 MB".
fn bytes(n: u64) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{} KB", n / 1024),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

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

/// A comment on a line: its file, its number (the old side's for a removed line), the line's code and what it says.
/// `id` tells two comments apart whatever they say; it is the diff's own and never sent.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: u64,
    pub path: String,
    pub line: usize,
    pub old: bool,
    pub code: String,
    pub text: String,
}

/// The line a comment being typed is about, taken when the field opens, so a read or a click meanwhile cannot move it.
struct Anchor {
    f: usize,
    l: usize,
}

/// The marks drawn under each line: a sent comment (✓) or one waiting (•), by file and line.
type Marks<'a> = HashMap<(usize, usize), Vec<(&'static str, Color, &'a str)>>;

impl Note {
    /// The comment as the review action takes it.
    pub fn value(&self) -> Value {
        json!({"path": self.path, "line": self.line, "old": self.old, "code": self.code, "text": self.text})
    }
}

pub struct Diff {
    pub source: Source,
    shown: Option<u64>,
    error: Option<String>,
    files: Vec<File>,
    /// Each file's hunks: where each starts and ends among its lines, and its id.
    hunks: Vec<Vec<(usize, usize, String)>>,
    /// Each file's lines' hunk, as an index into its `hunks`.
    hunk_of: Vec<Vec<Option<u32>>>,
    /// Only these hunks show (a walkthrough step's), else every one.
    only: Option<HashSet<String>>,
    /// Under `only`: each hunk shown or not, and each file's counts over the lines shown (None: the file is hidden).
    shown_hunks: Vec<Vec<bool>>,
    shown_counts: Vec<Option<(usize, usize)>>,
    /// A read that came while a comment was typed, taken in once the field closes.
    held: Option<(u64, Vec<File>)>,
    folded: HashSet<String>,
    /// The large files opened once: they stay open on the next reads.
    loaded: HashSet<String>,
    reading: bool,
    read_at: Option<Instant>,
    /// The file and line under the cursor; no line is the file's header.
    cursor: (usize, Option<usize>),
    side_by_side: bool,
    comment: Option<Input>,
    anchor: Option<Anchor>,
    /// The comment the field edits, by id, else the field writes a new one.
    editing: Option<u64>,
    /// Comments written and not sent yet (`S` sends them), then those sent; both shown under their line.
    drafts: Vec<Note>,
    sent: Vec<Note>,
    /// Where each comment sits now, by id: None when its line is no longer in the diff.
    at: HashMap<u64, (usize, usize)>,
    /// The comments last deleted or cleared, for undo.
    stash: Vec<Note>,
    next_id: u64,
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
            hunks: Vec::new(),
            hunk_of: Vec::new(),
            only: None,
            shown_hunks: Vec::new(),
            shown_counts: Vec::new(),
            held: None,
            folded: HashSet::new(),
            loaded: HashSet::new(),
            reading: false,
            read_at: None,
            cursor: (0, None),
            side_by_side: false,
            comment: None,
            anchor: None,
            editing: None,
            drafts: Vec::new(),
            sent: Vec::new(),
            at: HashMap::new(),
            stash: Vec::new(),
            next_id: 0,
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

    /// What the last read showed (or the one held while a comment is typed), for the next one to say "unchanged".
    pub fn shown(&self) -> Option<u64> {
        self.held.as_ref().map(|(hash, _)| *hash).or(self.shown)
    }

    /// A read came back: new files (None when the patch had not changed) or an error; false when nothing changed. New
    /// files wait while a comment is typed, so the line it is about stays where it is.
    pub fn arrived(&mut self, hash: u64, files: Option<Vec<File>>, error: Option<String>) -> bool {
        self.reading = false;
        if let Some(error) = error {
            let changed = self.error.as_ref() != Some(&error);
            (self.error, self.shown) = (Some(error), None);
            return changed;
        }
        let Some(files) = files else { return false };
        if self.comment.is_some() {
            self.held = Some((hash, files));
            return false;
        }
        self.take_in(hash, files);
        true
    }

    /// New files in: each line's hunk indexed, the filter applied again, the comments placed on their lines again.
    fn take_in(&mut self, hash: u64, files: Vec<File>) {
        self.hunks = files.iter().map(crate::gui::hunk_ids).collect();
        self.hunk_of = files
            .iter()
            .zip(&self.hunks)
            .map(|(file, hunks)| {
                let mut of = vec![None; file.lines.len()];
                for (h, (start, end, _)) in hunks.iter().enumerate() {
                    of[*start..*end].fill(Some(h as u32));
                }
                of
            })
            .collect();
        let cursor_path = self.files.get(self.cursor.0).map(|f| f.path.clone());
        // A large file starts folded, so a big diff reads at once; opened, it stays open.
        for file in files.iter().filter(|f| f.lines.len() > crate::gui::LARGE_LINES && !self.loaded.contains(&f.path)) {
            self.folded.insert(file.path.clone());
        }
        (self.files, self.shown, self.error) = (files, Some(hash), None);
        if let Some(f) = cursor_path.and_then(|p| self.files.iter().position(|file| file.path == p)) {
            self.cursor.0 = f;
        }
        self.refilter();
        self.place_all();
        self.clamp();
    }

    /// Each hunk shown or not under the filter, and the counts of the lines shown.
    fn refilter(&mut self) {
        let Some(only) = &self.only else {
            (self.shown_hunks, self.shown_counts) = (Vec::new(), Vec::new());
            return;
        };
        self.shown_hunks = self.hunks.iter().map(|hs| hs.iter().map(|(_, _, id)| only.contains(id)).collect()).collect();
        self.shown_counts = self
            .files
            .iter()
            .enumerate()
            .map(|(f, file)| {
                self.shown_hunks[f].iter().any(|s| *s).then(|| {
                    let kept = file.lines.iter().enumerate().filter(|(l, _)| self.hunk_of[f][*l].is_some_and(|h| self.shown_hunks[f][h as usize]));
                    kept.fold((0, 0), |(a, r), (_, line)| (a + usize::from(line.kind == Kind::Add), r + usize::from(line.kind == Kind::Del)))
                })
            })
            .collect();
    }

    /// Where a comment's line is now: the line of its file with its code on its side, the nearest its number; None
    /// when no such line is left.
    fn place(&self, note: &Note) -> Option<(usize, usize)> {
        let f = self.files.iter().position(|file| file.path == note.path)?;
        let lines = &self.files[f].lines;
        let same = |line: &Line| is_stop(line) && (line.kind == Kind::Del) == note.old && line.text == note.code;
        lines.iter().enumerate().filter(|(_, line)| same(line)).min_by_key(|(_, line)| number(line).unwrap_or(0).abs_diff(note.line)).map(|(l, _)| (f, l))
    }

    /// Every comment placed again on the lines as they are now, its number following its line.
    fn place_all(&mut self) {
        self.at.clear();
        for i in 0..self.drafts.len() + self.sent.len() {
            let note = if i < self.drafts.len() { &self.drafts[i] } else { &self.sent[i - self.drafts.len()] };
            let Some((f, l)) = self.place(note) else { continue };
            let n = number(&self.files[f].lines[l]).unwrap_or(note.line);
            self.at.insert(note.id, (f, l));
            let note = if i < self.drafts.len() { &mut self.drafts[i] } else { &mut self.sent[i - self.drafts.len()] };
            note.line = n;
        }
    }

    /// The files read last.
    pub fn files(&self) -> &[File] {
        &self.files
    }

    /// Whether a read has come back, with files or with an error.
    pub fn has_read(&self) -> bool {
        self.shown.is_some() || self.error.is_some()
    }

    /// Shows only these hunks (None: all of them), the cursor moved onto the first line left when its own is gone.
    pub fn set_only(&mut self, only: Option<HashSet<String>>) {
        if self.only == only {
            return;
        }
        self.only = only;
        self.refilter();
        if !self.stops().contains(&self.cursor) {
            self.cursor = self.stops().into_iter().find(|(_, l)| l.is_some()).unwrap_or((0, None));
        }
    }

    fn file_shown(&self, f: usize) -> bool {
        self.only.is_none() || self.shown_counts.get(f).is_some_and(Option::is_some)
    }

    fn line_shown(&self, f: usize, l: usize) -> bool {
        self.only.is_none() || self.hunk_of.get(f).and_then(|of| of.get(l).copied().flatten()).is_some_and(|h| self.shown_hunks[f][h as usize])
    }

    /// A file's counts over the lines shown.
    fn counts(&self, f: usize) -> (usize, usize) {
        match self.shown_counts.get(f).copied().flatten() {
            Some(counts) if self.only.is_some() => counts,
            _ => (self.files[f].added, self.files[f].removed),
        }
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
        for (f, file) in self.files.iter().enumerate().filter(|(f, _)| self.file_shown(*f)) {
            out.push((f, None));
            if !self.folded.contains(&file.path) {
                out.extend(file.lines.iter().enumerate().filter(|(l, line)| is_stop(line) && self.line_shown(f, *l)).map(|(l, _)| (f, Some(l))));
            }
        }
        out
    }

    /// Moves the cursor `by` places, files' headers included.
    pub fn step(&mut self, by: isize) {
        let stops = self.stops();
        let at = stops.iter().position(|s| *s == self.cursor).unwrap_or(0);
        let next = at.saturating_add_signed(by).min(stops.len().saturating_sub(1));
        if let Some(stop) = stops.get(next) {
            self.cursor = *stop;
        }
    }

    fn jump_file(&mut self, forward: bool) {
        let files: Vec<usize> = (0..self.files.len()).filter(|f| self.file_shown(*f)).collect();
        let at = files.iter().position(|f| *f == self.cursor.0).unwrap_or(0);
        let f = |i: usize| files.get(i).copied().unwrap_or(0);
        self.cursor = match (forward, self.cursor.1) {
            (true, _) => (f((at + 1).min(files.len().saturating_sub(1))), None),
            (false, Some(_)) => (f(at), None),
            (false, None) => (f(at.saturating_sub(1)), None),
        };
    }

    /// Moves the cursor to the first line of the next hunk, or back to the start of this one (then the one before);
    /// hunks of folded files are passed over. False when there is none that way.
    pub fn hunk(&mut self, forward: bool) -> bool {
        let stops = self.stops();
        let at = stops.iter().position(|s| *s == self.cursor).unwrap_or(0);
        let hunk = |(f, l): (usize, Option<usize>)| l.and_then(|l| self.hunk_of[f][l]).map(|h| (f, h));
        // A hunk starts at its first stop: the stop before it is a file's header or in another hunk.
        let starts: Vec<usize> = (0..stops.len()).filter(|i| stops[*i].1.is_some() && (*i == 0 || hunk(stops[i - 1]) != hunk(stops[*i]))).collect();
        let next = if forward { starts.iter().find(|i| **i > at) } else { starts.iter().rev().find(|i| **i < at) };
        let Some(&next) = next else { return false };
        self.cursor = stops[next];
        true
    }

    fn fold(&mut self, f: usize) {
        let Some(path) = self.files.get(f).map(|file| file.path.clone()) else { return };
        if self.folded.remove(&path) {
            self.loaded.insert(path);
        } else {
            self.folded.insert(path);
            self.cursor = (f, None);
        }
    }

    /// Folds every file, or unfolds them all when every one is folded already: unfolded, a large file is loaded and stays.
    pub fn fold_all(&mut self) {
        if self.files.iter().all(|f| self.folded.contains(&f.path)) {
            let opened = std::mem::take(&mut self.folded);
            self.loaded.extend(opened);
        } else {
            self.folded = self.files.iter().map(|f| f.path.clone()).collect();
            self.cursor.1 = None;
        }
    }

    /// The line under the cursor, when it is one a comment can go on.
    fn cursor_line(&self) -> Option<&Line> {
        self.files.get(self.cursor.0).and_then(|f| self.cursor.1.and_then(|l| f.lines.get(l))).filter(|l| is_stop(l))
    }

    fn start_comment(&mut self) {
        if let (Some(_), (f, Some(l))) = (self.cursor_line(), self.cursor) {
            (self.comment, self.anchor, self.editing) = (Some(Input::default()), Some(Anchor { f, l }), None);
        }
    }

    /// The field closed: a read that waited for it is taken in.
    fn close_field(&mut self) {
        (self.comment, self.anchor, self.editing) = (None, None, None);
        if let Some((hash, files)) = self.held.take() {
            self.take_in(hash, files);
        }
    }

    fn add_comment(&mut self) -> Key {
        let text = self.comment.as_ref().map(|i| i.text.trim().to_string()).unwrap_or_default();
        if let Some(i) = self.editing.and_then(|id| self.drafts.iter().position(|d| d.id == id)) {
            if text.is_empty() {
                self.stash = vec![self.drafts.remove(i)];
            } else {
                self.drafts[i].text = text;
            }
        } else if let Some(Anchor { f, l }) = self.anchor.take().filter(|_| !text.is_empty()) {
            let line = &self.files[f].lines[l];
            if let Some(n) = number(line) {
                self.next_id += 1;
                let note = Note { id: self.next_id, path: self.files[f].path.clone(), line: n, old: line.kind == Kind::Del, code: line.text.clone(), text };
                self.at.insert(note.id, (f, l));
                self.drafts.push(note);
            }
        }
        self.close_field();
        Key::Used
    }

    /// The comment written on the line under the cursor, not sent yet.
    pub fn draft_here(&self) -> Option<usize> {
        let (f, Some(l)) = self.cursor else { return None };
        self.drafts.iter().position(|d| self.at.get(&d.id) == Some(&(f, l)))
    }

    /// Opens the field on the comment under the cursor; true when there was one.
    pub fn edit_here(&mut self) -> bool {
        let (Some(i), (f, Some(l))) = (self.draft_here(), self.cursor) else { return false };
        (self.comment, self.anchor, self.editing) = (Some(Input::new(&self.drafts[i].text)), Some(Anchor { f, l }), Some(self.drafts[i].id));
        true
    }

    /// Deletes the comment under the cursor, kept for undo; true when there was one.
    pub fn delete_here(&mut self) -> bool {
        let Some(i) = self.draft_here() else { return false };
        self.stash = vec![self.drafts.remove(i)];
        true
    }

    /// Deletes every comment not sent yet, kept for undo; how many there were.
    pub fn clear(&mut self) -> usize {
        if !self.drafts.is_empty() {
            self.stash = std::mem::take(&mut self.drafts);
        }
        self.stash.len()
    }

    /// Brings back the comments last deleted, on their lines as they are now; how many.
    pub fn undo(&mut self) -> usize {
        let back = std::mem::take(&mut self.stash);
        let n = back.len();
        self.drafts.extend(back);
        self.place_all();
        n
    }

    /// Whether an undo would bring comments back.
    pub fn can_undo(&self) -> bool {
        !self.stash.is_empty()
    }

    /// The comments written and not sent yet whose line is still in the diff.
    pub fn notes(&self) -> Vec<&Note> {
        self.drafts.iter().filter(|d| self.at.contains_key(&d.id)).collect()
    }

    /// The comments not sent yet whose line is no longer in the diff: kept and listed, never sent.
    pub fn orphans(&self) -> Vec<&Note> {
        self.drafts.iter().filter(|d| !self.at.contains_key(&d.id)).collect()
    }

    /// Whether any comment waits to be sent, its line in the diff or not.
    pub fn has_drafts(&self) -> bool {
        !self.drafts.is_empty()
    }

    /// These comments were sent: marked as such under their line, no longer waiting.
    pub fn mark_sent(&mut self, sent: &[Note]) {
        let ids: HashSet<u64> = sent.iter().map(|n| n.id).collect();
        let (went, kept): (Vec<Note>, Vec<Note>) = std::mem::take(&mut self.drafts).into_iter().partition(|d| ids.contains(&d.id));
        self.drafts = kept;
        self.sent.extend(went);
    }

    /// Opens the field for a comment on the line under the cursor.
    pub fn comment_here(&mut self) {
        self.start_comment();
    }

    /// The cursor on the first line shown, its file's header when every file is folded.
    pub fn to_first_line(&mut self) {
        let stops = self.stops();
        self.cursor = stops.iter().find(|(_, l)| l.is_some()).or(stops.first()).copied().unwrap_or((0, None));
    }

    /// Whether the side-by-side view is on.
    pub fn side_by_side(&self) -> bool {
        self.side_by_side
    }

    pub fn toggle_side_by_side(&mut self) {
        self.side_by_side = !self.side_by_side;
    }

    #[cfg(test)]
    fn drafts(&self) -> usize {
        self.drafts.len()
    }

    /// The comments written so far, to the session: one as a comment, several as one review.
    fn send_comments(&mut self) -> Key {
        let drafts = std::mem::take(&mut self.drafts);
        let key = match drafts.as_slice() {
            [] => Key::Used,
            [n] => Key::Comment(json!({"path": n.path, "line": n.line, "text": n.text}), format!("{}:{}", n.path, n.line)),
            many => {
                let comments: Vec<Value> = many.iter().map(|n| json!({"path": n.path, "line": n.line, "old": n.old, "text": n.text})).collect();
                let label = many
                    .iter()
                    .enumerate()
                    .map(|(i, n)| format!("{}) {}:{}: {}", i + 1, n.path, n.line, n.text.split_whitespace().collect::<Vec<_>>().join(" ")))
                    .collect::<Vec<_>>()
                    .join(" ");
                Key::Review(json!({"comments": comments}), label)
            }
        };
        self.sent.extend(drafts);
        key
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
                Edit::Submit => self.add_comment(),
                Edit::Cancel => {
                    self.close_field();
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
            KeyCode::Char('n') => _ = self.hunk(true),
            KeyCode::Char('p') => _ = self.hunk(false),
            KeyCode::Char('z') => self.fold_all(),
            KeyCode::Char('s') => self.side_by_side = !self.side_by_side,
            KeyCode::Char('S') => return self.send_comments(),
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
        let shown: Vec<(usize, &File)> = self.files.iter().enumerate().filter(|(f, _)| self.file_shown(*f)).collect();
        let (added, removed) = shown.iter().map(|(f, _)| self.counts(*f)).fold((0, 0), |(a, r), (x, y)| (a + x, r + y));
        let count = shown.len();
        let marks = self.marks();
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
        let orphans = self.orphans();
        if !orphans.is_empty() {
            let n = orphans.len();
            out.push(
                Row::text(format!("{n} comment{} on lines no longer in the diff, not sent:", if n == 1 { "" } else { "s" }), fg(ORANGE)).target(Target::Diff),
            );
            for note in orphans {
                let place = format!("    • {}:{} ", note.path, note.line);
                out.push(
                    Row::text(place, fg(ORANGE))
                        .add(fit(&note.text, cols.saturating_sub(30)).trim_end().to_string(), dim().add_modifier(Modifier::ITALIC))
                        .target(Target::Diff),
                );
            }
        }
        if count > 1 {
            for &(f, file) in shown.iter().take(LISTED) {
                let here = focused && self.cursor.0 == f;
                let (added, removed) = self.counts(f);
                out.push(
                    Row::text(if here { "  ▸ " } else { "    " }, fg(ORANGE))
                        .act(file.path.clone(), fg(if here { ORANGE } else { FG }), Act::Jump(f))
                        .add(format!("  +{added}"), fg(GREEN))
                        .add(format!(" −{removed}"), fg(RED))
                        .target(Target::Diff),
                );
            }
            if count > LISTED {
                out.push(Row::text(format!("    … and {} more, ] goes through them", count - LISTED), dim()).target(Target::Diff));
            }
        }
        for &(f, file) in &shown {
            out.push(Row::new().target(Target::Diff));
            let folded = self.folded.contains(&file.path);
            let on_header = self.cursor == (f, None);
            let style = match (on_header, focused) {
                (true, true) => Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD),
                (true, false) => Style::new().fg(FG).bg(CURSOR_BLURRED),
                _ => bold(FG),
            };
            let (added, removed) = self.counts(f);
            let path = file.old_path.as_ref().map_or_else(|| file.path.clone(), |old| format!("{old} → {}", file.path));
            let title = format!(" {} {path}   +{added} −{removed} ", if folded { "▶" } else { "▼" });
            let mut header = Row::new().act(title, style, Act::Fold(f)).target(Target::Diff).here(on_header);
            if folded && file.lines.len() > crate::gui::LARGE_LINES && !self.loaded.contains(&file.path) {
                header = header.add(format!("  large diff · {} lines · enter loads it", file.lines.len()), fg(ORANGE));
            }
            out.push(header);
            if folded {
                continue;
            }
            if file.binary {
                let size = file.size.map(|n| format!(", {}", bytes(n))).unwrap_or_default();
                out.push(Row::text(format!("  binary{size}: no lines to show"), dim()).target(Target::Diff));
                continue;
            }
            let digits = file.lines.iter().filter_map(|l| l.old.max(l.new)).max().unwrap_or(0).to_string().len().max(3);
            if wide {
                self.side_by_side_rows(f, file, cols, digits, focused, &marks, &mut out);
            } else {
                self.unified_rows(f, file, cols, digits, focused, &marks, &mut out);
            }
        }
        out
    }

    /// The comments to draw under each line: those sent, then those waiting, the one being edited left out.
    fn marks(&self) -> Marks<'_> {
        let mut marks: Marks = HashMap::new();
        let sent = self.sent.iter().map(|n| (n, "    ✓ ", GREEN));
        let waiting = self.drafts.iter().filter(|n| Some(n.id) != self.editing).map(|n| (n, "    • ", ORANGE));
        for (note, mark, color) in sent.chain(waiting) {
            if let Some(at) = self.at.get(&note.id) {
                marks.entry(*at).or_default().push((mark, color, note.text.as_str()));
            }
        }
        marks
    }

    /// The cursor's mark in the gutter: orange when the diff has the keyboard.
    fn gutter(&self, f: usize, l: usize, first: bool, focused: bool) -> (String, Style) {
        match (self.cursor == (f, Some(l)) && first, focused) {
            (true, true) => ("▌".into(), fg(ORANGE)),
            (true, false) => ("▌".into(), fg(CURSOR_BLURRED)),
            _ => (" ".into(), Style::new()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn unified_rows(&self, f: usize, file: &File, cols: usize, digits: usize, focused: bool, marks: &Marks, out: &mut Vec<Row>) {
        let room = cols.saturating_sub(2 * digits + 5);
        let num = |n: Option<usize>| n.map_or(" ".repeat(digits), |n| format!("{n:>digits$}"));
        for (l, line) in file.lines.iter().enumerate().filter(|(l, _)| self.line_shown(f, *l)) {
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
            self.after_line(marks, l, f, cols, out);
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

    #[allow(clippy::too_many_arguments)]
    fn side_by_side_rows(&self, f: usize, file: &File, cols: usize, digits: usize, focused: bool, marks: &Marks, out: &mut Vec<Row>) {
        let half = (cols - 1) / 2;
        for pair in pairs(&file.lines) {
            let (left, right) = match pair {
                Pair::Across(l) if !self.line_shown(f, l) => continue,
                Pair::Facing(a, b) if ![a, b].into_iter().flatten().any(|l| self.line_shown(f, l)) => continue,
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
                self.after_line(marks, l, f, cols, out);
            }
        }
    }

    /// Under a line: the comments sent on it (✓), those waiting for `S` (•), and the comment being typed about it.
    fn after_line(&self, marks: &Marks, l: usize, f: usize, cols: usize, out: &mut Vec<Row>) {
        for (mark, color, text) in marks.get(&(f, l)).into_iter().flatten() {
            for (k, piece) in cockpit_core::text::wrap(text, cols.saturating_sub(8)).into_iter().enumerate() {
                out.push(Row::text(if k == 0 { *mark } else { "      " }, fg(*color)).add(piece, dim().add_modifier(Modifier::ITALIC)).target(Target::Diff));
            }
        }
        if let (Some(input), Some(anchor)) = (&self.comment, &self.anchor)
            && (anchor.f, anchor.l) == (f, l)
        {
            let mut row = input.line("    ✎ ");
            row.spans.push(ratatui::text::Span::styled("   enter adds it, then S sends them all · esc cancels", dim()));
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
        assert!(matches!(press(&mut diff, KeyCode::Enter), Key::Used));
        match press(&mut diff, KeyCode::Char('S')) {
            Key::Comment(value, label) => {
                assert_eq!(value, json!({"path": "a.py", "line": 2, "text": "ok"}));
                assert_eq!(label, "a.py:2");
            }
            _ => panic!("no comment sent"),
        }
    }

    const TWO_FILES: &str = "diff --git a/src/quotas.py b/src/quotas.py\n--- a/src/quotas.py\n+++ b/src/quotas.py\n@@ -1,3 +1,4 @@\n PLANS = {}\n+GRACE = 10\n \n def a():\n@@ -20,3 +21,3 @@ def enforce():\n x = 1\n-    if used == 0:\n+    if used >= GRACE:\n y = 2\ndiff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -1,1 +1,2 @@\n # acme/api\n+## Quotas\n";

    fn two_files() -> Diff {
        let mut diff = Diff::new(&json!({"patch": TWO_FILES}));
        diff.arrived(1, Some(crate::patch::prepare(TWO_FILES)), None);
        diff
    }

    #[test]
    fn n_and_p_go_from_hunk_to_hunk_across_files() {
        let mut diff = two_files();
        let at = |d: &Diff| (d.cursor.0, d.cursor.1.map(|l| d.files[d.cursor.0].lines[l].text.clone()));
        assert!(diff.hunk(true));
        assert_eq!(at(&diff), (0, Some("PLANS = {}".into())));
        assert!(diff.hunk(true));
        assert_eq!(at(&diff), (0, Some("x = 1".into())));
        press(&mut diff, KeyCode::Char('j'));
        assert!(diff.hunk(false), "back to the start of this hunk");
        assert_eq!(at(&diff), (0, Some("x = 1".into())));
        assert!(diff.hunk(true));
        assert_eq!(at(&diff), (1, Some("# acme/api".into())));
        assert!(!diff.hunk(true), "no hunk after the last");
        diff.fold(1);
        diff.cursor = (0, None);
        press(&mut diff, KeyCode::Char('n'));
        press(&mut diff, KeyCode::Char('n'));
        assert!(!diff.hunk(true), "a folded file's hunks are passed over");
    }

    #[test]
    fn a_step_shows_its_hunks_alone() {
        let mut diff = two_files();
        let second = diff.hunks[0][1].2.clone();
        diff.set_only(Some([second].into_iter().collect()));
        let text: Vec<String> = diff.rows(90, None).iter().map(|r| r.line.spans.iter().map(|s| s.content.as_ref()).collect()).collect();
        let all = text.join("\n");
        assert!(all.contains("GRACE:") && !all.contains("GRACE = 10") && !all.contains("README.md"), "{all}");
        diff.to_first_line();
        assert_eq!(diff.files[diff.cursor.0].lines[diff.cursor.1.unwrap()].text, "x = 1");
        diff.set_only(None);
        assert!(diff.rows(90, None).len() > text.len());
    }

    #[test]
    fn comments_are_edited_deleted_and_brought_back() {
        let mut diff = two_files();
        diff.hunk(true);
        press(&mut diff, KeyCode::Char('j'));
        for code in [KeyCode::Char('c'), KeyCode::Char('w'), KeyCode::Char('h'), KeyCode::Char('y'), KeyCode::Enter] {
            press(&mut diff, code);
        }
        let notes: Vec<(String, usize, bool, String, String)> =
            diff.notes().into_iter().map(|n| (n.path.clone(), n.line, n.old, n.code.clone(), n.text.clone())).collect();
        assert_eq!(notes, [("src/quotas.py".to_string(), 2, false, "GRACE = 10".to_string(), "why".to_string())]);
        assert!(diff.edit_here());
        press(&mut diff, KeyCode::Char('?'));
        press(&mut diff, KeyCode::Enter);
        assert_eq!(diff.notes()[0].text, "why?");
        assert!(diff.delete_here() && diff.notes().is_empty() && diff.can_undo());
        assert_eq!(diff.undo(), 1);
        assert_eq!(diff.clear(), 1);
        assert_eq!(diff.undo(), 1);
        let sent: Vec<Note> = diff.notes().into_iter().cloned().collect();
        diff.mark_sent(&sent);
        assert!(diff.notes().is_empty());
        let text: String = diff.rows(90, None).iter().flat_map(|r| r.line.spans.iter().map(|s| s.content.to_string())).collect();
        assert!(text.contains("✓ why?"), "a sent comment stays under its line");
    }

    #[test]
    fn an_edit_lands_on_its_comment_when_another_is_sent_meanwhile() {
        let mut diff = two_files();
        diff.hunk(true);
        press(&mut diff, KeyCode::Char('j'));
        for code in [KeyCode::Char('c'), KeyCode::Char('a'), KeyCode::Enter, KeyCode::Char('j'), KeyCode::Char('c'), KeyCode::Char('b'), KeyCode::Enter] {
            press(&mut diff, code);
        }
        assert!(diff.edit_here(), "the cursor is on b");
        let first: Vec<Note> = diff.notes().into_iter().filter(|n| n.text == "a").cloned().collect();
        diff.mark_sent(&first);
        press(&mut diff, KeyCode::Char('!'));
        press(&mut diff, KeyCode::Enter);
        let texts: Vec<&str> = diff.notes().iter().map(|n| n.text.as_str()).collect();
        assert_eq!(texts, ["b!"]);
        assert_eq!(diff.sent.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["a"], "the sent one is left as it was");
    }

    #[test]
    fn a_large_file_starts_folded_and_stays_open_once_loaded() {
        let table: String = (0..crate::gui::LARGE_LINES + 1).map(|i| format!("+row {i}\n")).collect();
        let patch = format!(
            "diff --git a/docs/quotas.md b/docs/limits.md\nsimilarity index 100%\nrename from docs/quotas.md\nrename to docs/limits.md\ndiff --git a/logo.png b/logo.png\nBinary files /dev/null and b/logo.png differ\ndiff --git a/table.py b/table.py\n--- /dev/null\n+++ b/table.py\n@@ -0,0 +1,{} @@\n{table}",
            crate::gui::LARGE_LINES + 1
        );
        let mut diff = Diff::new(&json!({"patch": ""}));
        let mut files = crate::patch::prepare(&patch);
        files[1].size = Some(12_288);
        diff.arrived(1, Some(files), None);
        let text =
            |d: &Diff| d.rows(100, None).iter().map(|r| r.line.spans.iter().map(|s| s.content.to_string()).collect::<String>()).collect::<Vec<_>>().join("\n");
        let screen = text(&diff);
        assert!(screen.contains("docs/quotas.md → docs/limits.md"), "{screen}");
        assert!(screen.contains("binary, 12 KB: no lines to show"), "{screen}");
        assert!(screen.contains("▶ table.py") && screen.contains("large diff · 1502 lines · enter loads it") && !screen.contains("row 7"), "{screen}");
        diff.cursor = (2, None);
        press(&mut diff, KeyCode::Enter);
        assert!(text(&diff).contains("row 7"));
        diff.arrived(2, Some(crate::patch::prepare(&patch)), None);
        assert!(text(&diff).contains("row 7"), "loaded once, it stays open");
        // Folding every file and unfolding them all loads the large one too, and it stays open on the next read.
        let mut all = Diff::new(&json!({"patch": ""}));
        all.arrived(1, Some(crate::patch::prepare(&patch)), None);
        all.fold_all();
        all.fold_all();
        all.arrived(2, Some(crate::patch::prepare(&patch)), None);
        assert!(text(&all).contains("row 7"), "unfolded with the rest, it stays open");
    }

    #[test]
    fn several_comments_go_as_one_review() {
        let patch = "diff --git a/a.py b/a.py\n--- a/a.py\n+++ b/a.py\n@@ -1,2 +1,2 @@\n x = 1\n-y = 2\n+y = 3\n";
        let mut diff = Diff::new(&json!({"patch": patch}));
        diff.arrived(1, Some(crate::patch::prepare(patch)), None);
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('c'),
            KeyCode::Char('a'),
            KeyCode::Enter,
            KeyCode::Char('j'),
            KeyCode::Char('j'),
            KeyCode::Char('c'),
            KeyCode::Char('b'),
            KeyCode::Enter,
        ] {
            press(&mut diff, code);
        }
        assert_eq!(diff.drafts(), 2);
        match press(&mut diff, KeyCode::Char('S')) {
            Key::Review(value, label) => {
                assert_eq!(value["comments"].as_array().map(Vec::len), Some(2));
                assert_eq!(label, "1) a.py:1: a 2) a.py:2: b");
            }
            _ => panic!("no review sent"),
        }
        assert_eq!(diff.drafts(), 0);
    }

    /// Times what a 1 MB diff costs: reading, drawing, moving, comments and the walkthrough over it. Run with
    /// `cargo test -p cockpit-show --lib large_diff -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn large_diff_costs() {
        use std::time::Instant;
        let patch = crate::patch::large_patch();
        let lines = patch.lines().count();
        let time = |what: &str, started: Instant| println!("{what:<34} {:>8.1} ms", started.elapsed().as_secs_f64() * 1000.0);
        println!("patch: {} bytes, {lines} lines", patch.len());
        let t = Instant::now();
        let files = crate::patch::prepare(&patch);
        time("prepare (parse and paint)", t);
        let t = Instant::now();
        for f in &files {
            crate::gui::hunk_ids(f);
        }
        time("hunk ids, every file", t);
        let mut diff = Diff::new(&json!({"patch": ""}));
        let t = Instant::now();
        diff.arrived(1, Some(files), None);
        time("arrived", t);
        let t = Instant::now();
        let rows = diff.rows(120, Some(&Target::Diff)).len();
        time(&format!("rows at 120 cols ({rows})"), t);
        let t = Instant::now();
        let mut hops = 0;
        while diff.hunk(true) && hops < 200 {
            hops += 1;
        }
        time("n, 200 times", t);
        let t = Instant::now();
        for _ in 0..200 {
            press(&mut diff, KeyCode::Char('j'));
        }
        time("j, 200 times", t);
        diff.cursor = (0, None);
        for k in 0..50 {
            diff.hunk(true);
            press(&mut diff, KeyCode::Char('j'));
            press(&mut diff, KeyCode::Char('c'));
            press(&mut diff, KeyCode::Char(char::from(b'a' + (k % 26) as u8)));
            press(&mut diff, KeyCode::Enter);
        }
        let t = Instant::now();
        for _ in 0..100 {
            diff.notes();
        }
        time("notes() with 50 comments, 100 times", t);
        let t = Instant::now();
        diff.rows(120, Some(&Target::Diff));
        time("rows with 50 comments", t);
        let only: HashSet<String> = diff.hunks.iter().flat_map(|h| h.iter().step_by(7).map(|(_, _, id)| id.clone())).collect();
        let t = Instant::now();
        diff.set_only(Some(only));
        let n = diff.rows(120, Some(&Target::Diff)).len();
        time(&format!("step filter and its rows ({n})"), t);
    }
}
