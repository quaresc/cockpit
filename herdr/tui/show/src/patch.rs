//! A unified diff read into files and lines (`git_diff`, `new_file` and `parse_patch`), then painted: each
//! line in its file type's syntax colours, the words a changed line changed marked against the line it replaced.

use std::io::Read;
use std::ops::Range;
use std::process::Command;
use std::sync::LazyLock;

use ratatui::style::Style;
use regex::Regex;
use similar::{Algorithm, ChangeTag};

use cockpit_core::theme::{BLUE, dim, fg};

use crate::highlight::{Grammars, Painter, token_for_path};

static HUNK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").unwrap());
/// Past this many lines a diff stays plain: painting it would take seconds.
const PAINTED_LINES: usize = 20_000;
/// Longer lines are not compared word by word.
const COMPARED_BYTES: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Hunk,
    Add,
    Del,
    Same,
    Note,
}

pub struct Line {
    pub kind: Kind,
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
    /// The text in coloured pieces, each true when the word-level diff says it changed.
    pub paint: Vec<(String, Style, bool)>,
}

pub struct File {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub lines: Vec<Line>,
}

impl File {
    fn push(&mut self, kind: Kind, old: Option<usize>, new: Option<usize>, text: &str) {
        self.lines.push(Line { kind, old, new, text: text.to_string(), paint: Vec::new() });
    }
}

/// Files of a unified diff: their path, counts and lines. A hunk's own counts say where it ends, so a removed `-- x`
/// or an added `++ x` line is not taken for a file header, and what sits between hunks (`Binary files … differ`) is a note.
pub fn parse_patch(patch: &str) -> Vec<File> {
    let (mut files, mut old, mut new): (Vec<File>, usize, usize) = (Vec::new(), 0, 0);
    let (mut old_left, mut new_left) = (0usize, 0usize);
    const SKIPPED: [&str; 10] = ["index ", "new file", "deleted file", "similarity", "dissimilarity", "rename ", "copy ", "old mode", "new mode", "--- "];
    for raw in patch.lines() {
        if old_left > 0 || new_left > 0 {
            let Some(current) = files.last_mut() else { break };
            if let Some(text) = raw.strip_prefix('+') {
                current.added += 1;
                current.push(Kind::Add, None, Some(new), text);
                (new, new_left) = (new + 1, new_left.saturating_sub(1));
            } else if let Some(text) = raw.strip_prefix('-') {
                current.removed += 1;
                current.push(Kind::Del, Some(old), None, text);
                (old, old_left) = (old + 1, old_left.saturating_sub(1));
            } else if raw.starts_with('\\') {
                current.push(Kind::Note, None, None, raw);
            } else {
                current.push(Kind::Same, Some(old), Some(new), raw.get(1..).unwrap_or(""));
                (old, new, old_left, new_left) = (old + 1, new + 1, old_left.saturating_sub(1), new_left.saturating_sub(1));
            }
            continue;
        }
        if let Some(rest) = raw.strip_prefix("diff --git ") {
            let path = rest.split_once(" b/").map_or(rest, |(_, b)| b).to_string();
            files.push(File { path, added: 0, removed: 0, lines: Vec::new() });
            continue;
        }
        // A plain `diff -u` has no `diff --git` line: its `--- ` starts the file.
        if raw.starts_with("--- ") && files.last().is_none_or(|f| !f.lines.is_empty()) {
            let path = raw[4..].split('\t').next().unwrap_or_default();
            files.push(File { path: path.strip_prefix("a/").unwrap_or(path).to_string(), added: 0, removed: 0, lines: Vec::new() });
            continue;
        }
        if raw.starts_with('\\') {
            if let Some(current) = files.last_mut() {
                current.push(Kind::Note, None, None, raw);
            }
            continue;
        }
        let Some(current) = files.last_mut() else { continue };
        if let Some(path) = raw.strip_prefix("+++ ").map(|p| p.split('\t').next().unwrap_or_default()) {
            if path != "/dev/null" {
                current.path = path.strip_prefix("b/").unwrap_or(path).to_string();
            }
        } else if let Some(m) = HUNK.captures(raw) {
            let number = |i: usize, default: usize| m.get(i).map_or(default, |g| g.as_str().parse().unwrap_or(0));
            (old, old_left, new, new_left) = (number(1, 0), number(2, 1), number(3, 0), number(4, 1));
            current.push(Kind::Hunk, None, None, raw);
        } else if !raw.is_empty() && !SKIPPED.iter().any(|s| raw.starts_with(s)) {
            current.push(Kind::Note, None, None, raw);
        }
    }
    files
}

/// The byte ranges of a removed and an added line that changed.
type Changed = (Vec<Range<usize>>, Vec<Range<usize>>);

/// The byte ranges of `old` and `new` the word-level diff says changed; None when the lines share too little for word
/// marks to help.
fn changed(old: &str, new: &str) -> Option<Changed> {
    if old.len() > COMPARED_BYTES || new.len() > COMPARED_BYTES {
        return None;
    }
    let (mut o, mut n, mut same) = (0, 0, 0);
    let (mut gone, mut came) = (Vec::new(), Vec::new());
    for (tag, piece) in similar::utils::diff_unicode_words(Algorithm::Myers, old, new) {
        let len = piece.len();
        match tag {
            ChangeTag::Equal => {
                same += piece.trim().len();
                (o, n) = (o + len, n + len);
            }
            ChangeTag::Delete => {
                gone.push(o..o + len);
                o += len;
            }
            ChangeTag::Insert => {
                came.push(n..n + len);
                n += len;
            }
        }
    }
    let longest = old.trim().len().max(new.trim().len());
    (same * 5 >= longest * 2).then_some((gone, came))
}

/// Pieces split where the ranges start and end, the pieces inside them marked.
fn mark(pieces: Vec<(String, Style, bool)>, ranges: &[Range<usize>]) -> Vec<(String, Style, bool)> {
    let mut out = Vec::new();
    let mut at = 0;
    for (text, style, _) in pieces {
        let end = at + text.len();
        let mut cuts: Vec<usize> = ranges.iter().flat_map(|r| [r.start, r.end]).filter(|c| *c > at && *c < end).collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut from = at;
        for cut in cuts.into_iter().chain([end]) {
            if cut > from && text.is_char_boundary(from - at) && text.is_char_boundary(cut - at) {
                let inside = ranges.iter().any(|r| r.start <= from && cut <= r.end);
                out.push((text[from - at..cut - at].to_string(), style, inside));
                from = cut;
            }
        }
        at = end;
    }
    out
}

fn expand(text: &str) -> String {
    text.replace('\t', "    ")
}

/// Colours every line for its file's type, the grammar following each side of a hunk, then marks changed words.
fn paint_file(file: &mut File, grammars: Option<&Grammars>) {
    let token = token_for_path(&file.path);
    let painter = || grammars.map_or_else(Painter::plain, |g| g.painter(&token));
    let (mut old, mut new) = (painter(), painter());
    for line in &mut file.lines {
        let text = expand(&line.text);
        let pieces = match line.kind {
            Kind::Hunk => {
                old.restart();
                new.restart();
                vec![(text, fg(BLUE), false)]
            }
            Kind::Note => vec![(text, dim(), false)],
            Kind::Same => {
                old.line(&text);
                new.line(&text).into_iter().map(|(t, s)| (t, s, false)).collect()
            }
            Kind::Del => old.line(&text).into_iter().map(|(t, s)| (t, s, false)).collect(),
            Kind::Add => new.line(&text).into_iter().map(|(t, s)| (t, s, false)).collect(),
        };
        line.paint = pieces;
    }
    // A run of removed lines followed by added ones: each pair compared word by word.
    let mut i = 0;
    while i < file.lines.len() {
        let dels = file.lines[i..].iter().take_while(|l| l.kind == Kind::Del).count();
        let adds = file.lines[i + dels..].iter().take_while(|l| l.kind == Kind::Add).count();
        for k in 0..dels.min(adds) {
            let (a, b) = (i + k, i + dels + k);
            if let Some((gone, came)) = changed(&expand(&file.lines[a].text), &expand(&file.lines[b].text)) {
                file.lines[a].paint = mark(std::mem::take(&mut file.lines[a].paint), &gone);
                file.lines[b].paint = mark(std::mem::take(&mut file.lines[b].paint), &came);
            }
        }
        i += (dels + adds).max(1);
    }
}

/// The patch read and painted, ready to draw.
pub fn prepare(patch: &str) -> Vec<File> {
    let mut files = parse_patch(patch);
    let colour = files.iter().map(|f| f.lines.len()).sum::<usize>() <= PAINTED_LINES;
    let grammars = (colour && !files.is_empty()).then(Grammars::load);
    for file in &mut files {
        paint_file(file, grammars.as_ref());
    }
    files
}

/// An untracked file as git would show it added, read here rather than by one git process per file.
fn new_file(repo: &str, path: &str) -> String {
    let head = format!("diff --git a/{path} b/{path}\nnew file mode 100644\n");
    let mut raw = Vec::new();
    let Ok(file) = std::fs::File::open(std::path::Path::new(repo).join(path)) else { return String::new() };
    if file.take(1_000_000).read_to_end(&mut raw).is_err() {
        return String::new();
    }
    if raw.is_empty() {
        return head;
    }
    if raw.contains(&0) {
        return format!("{head}Binary files /dev/null and b/{path} differ\n");
    }
    let text = String::from_utf8_lossy(&raw);
    let mut lines: Vec<&str> = text.split('\n').collect();
    let ends_cleanly = lines.last() == Some(&"");
    if ends_cleanly {
        lines.pop();
    }
    let body: String = lines.iter().map(|l| format!("+{l}\n")).collect::<String>() + if ends_cleanly { "" } else { "\\ No newline at end of file\n" };
    let count = if lines.len() != 1 { format!("1,{}", lines.len()) } else { "1".into() };
    format!("{head}--- /dev/null\n+++ b/{path}\n@@ -0,0 +{count} @@\n{body}")
}

/// The repo's changes against `base`, untracked files included as new ones.
pub fn git_diff(repo: &str, base: Option<&str>) -> Result<String, String> {
    let base = base.unwrap_or("HEAD");
    let run = |args: &[&str]| Command::new("git").arg("-C").arg(repo).args(args).output();
    let patch = match run(&["diff", base, "--"]) {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if err.is_empty() { format!("git diff {base} failed") } else { err });
        }
        Err(err) => return Err(format!("git did not answer: {err}")),
    };
    let untracked = run(&["ls-files", "--others", "--exclude-standard"]).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let extra: String = untracked.lines().filter(|f| !f.is_empty()).take(30).map(|f| new_file(repo, f)).collect();
    Ok(patch + &extra)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    let x = 1;\n+    let x = 2;\n }\n";

    #[test]
    fn counts_and_numbers() {
        let files = parse_patch(PATCH);
        assert_eq!((files.len(), files[0].path.as_str(), files[0].added, files[0].removed), (1, "src/a.rs", 1, 1));
        let numbers: Vec<(Option<usize>, Option<usize>)> = files[0].lines.iter().map(|l| (l.old, l.new)).collect();
        assert_eq!(numbers, vec![(None, None), (Some(1), Some(1)), (Some(2), None), (None, Some(2)), (Some(3), Some(3))]);
    }

    #[test]
    fn marks_the_changed_word_only() {
        let files = prepare(PATCH);
        let marked = |l: &Line| l.paint.iter().filter(|p| p.2).map(|p| p.0.clone()).collect::<String>();
        assert_eq!((marked(&files[0].lines[2]), marked(&files[0].lines[3])), ("1".to_string(), "2".to_string()));
        let text = |l: &Line| l.paint.iter().map(|p| p.0.clone()).collect::<String>();
        assert_eq!(text(&files[0].lines[3]), "    let x = 2;");
    }

    /// A parsed line by kind, numbers and text.
    type Seen = (Kind, Option<usize>, Option<usize>, String);

    /// Path, counts and lines, the parts of a parsed file git's own diff must agree on.
    fn summary(files: &[File]) -> Vec<(String, usize, usize, Vec<Seen>)> {
        files.iter().map(|f| (f.path.clone(), f.added, f.removed, f.lines.iter().map(|l| (l.kind, l.old, l.new, l.text.clone())).collect())).collect()
    }

    #[test]
    fn a_new_file_reads_like_git_shows_it() {
        let dir = std::env::temp_dir().join(format!("cockpit-show-new-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let repo = dir.to_string_lossy().into_owned();
        assert!(Command::new("git").args(["init", "-q", &repo]).status().unwrap().success());
        let files: [(&str, &[u8]); 4] = [("lines.txt", b"a\nb\n"), ("no-newline.txt", b"a"), ("binary.bin", b"\0\x01"), ("empty.txt", b"")];
        for (name, body) in files {
            std::fs::write(dir.join(name), body).unwrap();
        }
        for (name, _) in files {
            let git = Command::new("git").args(["-C", &repo, "diff", "--no-index", "--", "/dev/null", name]).output().unwrap();
            let git = String::from_utf8_lossy(&git.stdout).into_owned();
            assert_eq!(summary(&parse_patch(&new_file(&repo, name))), summary(&parse_patch(&git)), "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    proptest! {
        /// Whatever the patch, parsing ends, every count matches the lines kept, and painting keeps the text.
        #[test]
        fn any_patch_parses(lines in prop::collection::vec(prop_oneof![
            Just("diff --git a/x.rs b/x.rs".to_string()),
            Just("--- a/x.rs".to_string()),
            Just("+++ b/x.rs".to_string()),
            (0usize..9, 0usize..4, 0usize..9, 0usize..4).prop_map(|(a, b, c, d)| format!("@@ -{a},{b} +{c},{d} @@")),
            "[-+ \\\\]?[ -~\t]{0,20}",
            "\\PC{0,12}",
        ], 0..40)) {
            let patch = lines.join("\n");
            let files = prepare(&patch);
            for file in &files {
                prop_assert_eq!(file.added, file.lines.iter().filter(|l| l.kind == Kind::Add).count());
                prop_assert_eq!(file.removed, file.lines.iter().filter(|l| l.kind == Kind::Del).count());
                for line in &file.lines {
                    let painted: String = line.paint.iter().map(|p| p.0.as_str()).collect();
                    prop_assert_eq!(painted, expand(&line.text));
                }
            }
        }
    }
}
