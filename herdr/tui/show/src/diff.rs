//! A diff: a patch given whole, or a repo's working tree read again every few seconds in the background, one fold per
//! file with its +/− counts, line numbers and colours. `git_diff`, `new_file` and `parse_patch` are show.py's.

use std::collections::HashSet;
use std::io::Read;
use std::process::Command;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Modifier, Style};
use regex::Regex;
use serde_json::Value;

use cockpit_core::text::fit;
use cockpit_core::theme::{BG, BLUE, FG, GREEN, ORANGE, RED, dim, fg};

use crate::row::{Act, Row, Target};

/// A repo diff is read again this often, so the pane follows the agent's edits.
const EVERY: Duration = Duration::from_secs(3);
static HUNK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@@ -(\d+)(?:,\d+)? \+(\d+)").unwrap());

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Hunk,
    Add,
    Del,
    Same,
    Note,
}

struct File {
    path: String,
    added: usize,
    removed: usize,
    lines: Vec<(Kind, Option<usize>, Option<usize>, String)>,
}

/// Files of a unified diff: their path, counts and lines.
fn parse_patch(patch: &str) -> Vec<File> {
    let (mut files, mut old, mut new): (Vec<File>, usize, usize) = (Vec::new(), 0, 0);
    const SKIPPED: [&str; 8] = ["--- ", "index ", "new file", "deleted file", "similarity", "rename ", "old mode", "new mode"];
    for raw in patch.lines() {
        if let Some(rest) = raw.strip_prefix("diff --git ") {
            let path = rest.split_once(" b/").map_or(rest, |(_, b)| b).to_string();
            files.push(File { path, added: 0, removed: 0, lines: Vec::new() });
            continue;
        }
        let Some(current) = files.last_mut() else { continue };
        if let Some(path) = raw.strip_prefix("+++ ").filter(|p| *p != "/dev/null") {
            current.path = path.strip_prefix("b/").unwrap_or(path).to_string();
        } else if SKIPPED.iter().any(|s| raw.starts_with(s)) {
        } else if raw.starts_with("@@") {
            (old, new) = HUNK.captures(raw).map_or((0, 0), |m| (m[1].parse().unwrap_or(0), m[2].parse().unwrap_or(0)));
            current.lines.push((Kind::Hunk, None, None, raw.to_string()));
        } else if let Some(text) = raw.strip_prefix('+') {
            current.added += 1;
            current.lines.push((Kind::Add, None, Some(new), text.to_string()));
            new += 1;
        } else if let Some(text) = raw.strip_prefix('-') {
            current.removed += 1;
            current.lines.push((Kind::Del, Some(old), None, text.to_string()));
            old += 1;
        } else if raw.starts_with('\\') {
            current.lines.push((Kind::Note, None, None, raw.to_string()));
        } else {
            current.lines.push((Kind::Same, Some(old), Some(new), raw.get(1..).unwrap_or("").to_string()));
            (old, new) = (old + 1, new + 1);
        }
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

/// The repo's changes against `base`, untracked files included as new ones: (patch, error).
pub fn git_diff(repo: &str, base: Option<&str>) -> (Option<String>, Option<String>) {
    let base = base.unwrap_or("HEAD");
    let run = |args: &[&str]| Command::new("git").arg("-C").arg(repo).args(args).output();
    let patch = match run(&["diff", base, "--"]) {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return (None, Some(if err.is_empty() { format!("git diff {base} failed") } else { err }));
        }
        Err(err) => return (None, Some(format!("git did not answer: {err}"))),
    };
    let untracked = run(&["ls-files", "--others", "--exclude-standard"]).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let extra: String = untracked.lines().filter(|f| !f.is_empty()).take(30).map(|f| new_file(repo, f)).collect();
    (Some(patch + &extra), None)
}

pub struct Diff {
    pub repo: Option<String>,
    pub base: Option<String>,
    patch: Option<(Option<String>, Option<String>)>,
    files: Vec<File>,
    folded: HashSet<String>,
    pub reading: bool,
    read_at: Option<Instant>,
}

impl Diff {
    pub fn new(block: &Value) -> Self {
        let given = block["patch"].as_str().map(str::to_string);
        let repo = if given.is_none() { block["repo"].as_str().map(str::to_string) } else { None };
        let mut diff = Self { repo, base: block["base"].as_str().map(str::to_string), patch: None, files: Vec::new(), folded: HashSet::new(), reading: false, read_at: None };
        if given.is_some() {
            diff.arrived(given, None);
        }
        diff
    }

    /// True when a background read of the repo is due.
    pub fn due(&mut self) -> bool {
        if self.repo.is_none() || self.reading || self.read_at.is_some_and(|at| at.elapsed() < EVERY) {
            return false;
        }
        self.reading = true;
        self.read_at = Some(Instant::now());
        true
    }

    /// A read came back; false when nothing changed.
    pub fn arrived(&mut self, patch: Option<String>, error: Option<String>) -> bool {
        self.reading = false;
        let fresh = Some((patch, error));
        if fresh == self.patch {
            return false;
        }
        self.patch = fresh;
        self.files = parse_patch(self.patch.as_ref().and_then(|p| p.0.as_deref()).unwrap_or(""));
        true
    }

    pub fn rows(&self, cols: usize, focus: Option<&Target>) -> Vec<Row> {
        let Some((_, error)) = &self.patch else {
            return vec![Row::text(format!("reading {}…", self.repo.as_deref().unwrap_or("")), dim())];
        };
        if let Some(error) = error {
            return vec![Row::text(error.clone(), fg(RED))];
        }
        let (added, removed): (usize, usize) = (self.files.iter().map(|f| f.added).sum(), self.files.iter().map(|f| f.removed).sum());
        let mut out = vec![Row::text(format!("{} files  ", self.files.len()), dim()).add(format!("+{added}"), fg(GREEN)).add(" ", Style::new()).add(format!("−{removed}"), fg(RED))];
        for f in &self.files {
            let target = Target::File(f.path.clone());
            let folded = self.folded.contains(&f.path);
            let style = if focus == Some(&target) { Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD) } else { fg(FG) };
            let title = format!(" {} {}   +{} −{} ", if folded { "▶" } else { "▼" }, f.path, f.added, f.removed);
            out.push(Row::new().act(title, style, Act::Fold(f.path.clone())).target(target));
            if folded {
                continue;
            }
            out.push(Row::new());
            for (kind, old, new, text) in &f.lines {
                let gutter = new.or(*old).map_or("      ".to_string(), |n| format!("{n:>5} "));
                let (sign, style) = match kind {
                    Kind::Add => ("+", fg(GREEN)),
                    Kind::Del => ("-", fg(RED)),
                    Kind::Same => (" ", Style::new()),
                    Kind::Note => (" ", dim()),
                    Kind::Hunk => ("", fg(BLUE)),
                };
                let body = fit(&format!("{sign} {}", text.replace('\t', "    ")), cols.saturating_sub(9));
                out.push(Row::text("   ", Style::new()).add(gutter, dim()).add(body.trim_end().to_string(), style));
            }
        }
        out
    }

    pub fn targets(&self) -> Vec<Target> {
        let shown = self.patch.as_ref().is_some_and(|(_, error)| error.is_none());
        if shown { self.files.iter().map(|f| Target::File(f.path.clone())).collect() } else { vec![] }
    }

    pub fn act(&mut self, act: &Act) {
        if let Act::Fold(path) = act
            && !self.folded.remove(path)
        {
            self.folded.insert(path.clone());
        }
    }

    pub fn key(&mut self, target: &Target, key: KeyEvent) -> bool {
        match (target, key.code) {
            (Target::File(path), KeyCode::Enter) => {
                self.act(&Act::Fold(path.clone()));
                true
            }
            _ => false,
        }
    }
}
