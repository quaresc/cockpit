//! The runbook files, read as runbook.py's `parse` reads them: a title, an `updated:` line, a step table and notes; a
//! `\|` in a cell is a pipe of its text, not a column's end.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Value, json};

use crate::table;

/// A status starting with one of these is a step still to do.
pub const OPEN: [&str; 4] = ["⬜", "⏳", "🟡", "❌"];

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub n: String,
    pub step: String,
    pub where_: String,
    pub status: String,
    pub command: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Book {
    pub path: PathBuf,
    pub slug: String,
    pub title: String,
    pub updated: String,
    pub steps: Vec<Step>,
    pub notes: Vec<String>,
}

impl Step {
    pub fn is_open(&self) -> bool {
        OPEN.iter().any(|o| self.status.starts_with(o))
    }

    /// The step as `runbook.parse` makes it, for the bridge.
    pub fn json(&self) -> Value {
        json!({"n": self.n, "step": self.step, "where": self.where_, "status": self.status, "command": self.command})
    }
}

impl Book {
    pub fn todo(&self) -> usize {
        self.steps.iter().filter(|s| s.is_open()).count()
    }

    pub fn done_steps(&self) -> Vec<String> {
        self.steps.iter().filter(|s| !s.is_open()).map(|s| s.n.clone()).collect()
    }

    pub fn json(&self) -> Value {
        json!({"path": self.path, "slug": self.slug, "title": self.title, "updated": self.updated,
               "steps": self.steps.iter().map(Step::json).collect::<Vec<_>>(), "notes": self.notes})
    }
}

/// `~/.config/cockpit/runbooks`, or another folder to try the app on.
pub fn dir() -> PathBuf {
    std::env::var("COCKPIT_RUNBOOKS_DIR").map(PathBuf::from).unwrap_or_else(|_| cockpit_core::files::config_dir().join("runbooks"))
}

pub fn parse(path: &Path) -> std::io::Result<Book> {
    Ok(parse_text(path, &std::fs::read_to_string(path)?))
}

/// The runbook in this text; anything that is not a title, an `updated:` line, a step row or a note is skipped.
pub fn parse_text(path: &Path, text: &str) -> Book {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let mut book = Book {
        path: path.to_path_buf(),
        slug: name.strip_suffix(".md").unwrap_or(name).to_string(),
        title: String::new(),
        updated: String::new(),
        steps: Vec::new(),
        notes: Vec::new(),
    };
    let mut in_notes = false;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(title) = line.strip_prefix("# ").filter(|_| book.title.is_empty()) {
            book.title = title.to_string();
        } else if line.to_lowercase().starts_with("updated:") {
            book.updated = line.split_once(':').map(|(_, v)| v.trim().to_string()).unwrap_or_default();
        } else if line.starts_with("## ") {
            in_notes = line.to_lowercase().contains("note");
        } else if line.starts_with('|') {
            if let Some(mut cells) = table::step_cells(line) {
                cells.resize(cells.len().max(5), String::new());
                book.steps.push(Step {
                    n: cells[0].clone(),
                    step: cells[1].clone(),
                    where_: cells[2].clone(),
                    status: cells[3].clone(),
                    command: cells[4].trim_matches('`').to_string(),
                });
            }
        } else if in_notes && (line.starts_with('-') || line.starts_with('*') || line.starts_with('•')) {
            book.notes.push(line.trim_start_matches(['-', '*', '•', ' ']).trim().to_string());
        }
    }
    book
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Every runbook, the newest file first.
pub fn read_books(dir: &Path) -> Vec<Book> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut paths: Vec<(PathBuf, Option<SystemTime>)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .map(|p| {
            let at = mtime(&p);
            (p, at)
        })
        .collect();
    paths.sort_by_key(|p| std::cmp::Reverse(p.1));
    paths.into_iter().filter_map(|(p, _)| parse(&p).ok()).collect()
}

/// Each file's name and mtime in these folders, and these files' mtimes: a change in it means something to read again.
pub fn stamp(dirs: &[&Path], files: &[&Path]) -> Vec<(String, Option<SystemTime>)> {
    let mut stamp = Vec::new();
    for dir in dirs {
        let mut names: Vec<(String, Option<SystemTime>)> = std::fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), mtime(&e.path()))).collect())
            .unwrap_or_default();
        names.sort();
        stamp.push((String::from("/"), None));
        stamp.extend(names);
    }
    stamp.extend(files.iter().map(|f| (f.to_string_lossy().into_owned(), mtime(f))));
    stamp
}
