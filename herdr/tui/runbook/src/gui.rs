//! The runbooks for the Herdr app's cockpit mod: the view model the hub pushes (every runbook, its steps with their
//! state, actions, session and PR, its refresh) and the edits the mod makes in the files, each undoable while nobody
//! else wrote the file since.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use cockpit_core::acts::Act;
use cockpit_core::files;
use serde_json::{Value, json};

use crate::acts::{book_acts, handoff_id, pr_ref, step_acts};
use crate::book::{self, Book, Step};
use crate::handoff;
use crate::table::{self, Column};

/// The folders the view reads: the runbooks and the sessions' handoffs.
pub fn dirs() -> Vec<PathBuf> {
    vec![book::dir(), handoff::dir()]
}

pub fn view(_args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let inbox = doc("inbox");
    let inbox = if inbox.is_null() { files::load(&handoff::inbox_path()).unwrap_or(Value::Null) } else { inbox };
    model(&book::dir(), &handoff::dir(), &files::cache(), &inbox)
}

fn ms(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default()
}

fn stamp(path: &Path) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    meta.created().or_else(|_| meta.modified()).ok().map(ms)
}

/// Every runbook, the newest first, as the mod draws it.
pub fn model(runbooks: &Path, handoffs: &Path, cache: &Path, inbox: &Value) -> Value {
    let books = book::read_books(runbooks);
    let sessions = handoff::read(handoffs);
    let left: usize = books.iter().map(Book::todo).sum();
    json!({
        "dir": runbooks,
        "left": left,
        "can_undo": can_undo(),
        "books": books.iter().map(|b| book_json(b, &sessions, cache, inbox)).collect::<Vec<_>>(),
    })
}

fn act_json(act: &Act) -> Value {
    json!({"id": act.id, "label": act.label, "key": act.key.map(String::from), "main": act.main, "confirm": act.confirm})
}

fn book_json(book: &Book, sessions: &HashMap<String, Value>, cache: &Path, inbox: &Value) -> Value {
    let total = book.steps.len();
    let todo = book.todo();
    json!({
        "slug": book.slug,
        "name": book.slug.to_uppercase(),
        "title": if book.title.is_empty() { &book.slug } else { &book.title },
        "updated": book.updated,
        "path": book.path,
        "todo": todo,
        "total": total,
        "done": total - todo,
        "refresh": refresh(&book.slug, cache),
        "notes": book.notes.iter().map(|n| note_parts(n)).collect::<Vec<_>>(),
        "acts": book_acts(book).iter().map(act_json).collect::<Vec<_>>(),
        "steps": book.steps.iter().map(|s| step_json(book, s, sessions, inbox)).collect::<Vec<_>>(),
    })
}

/// A background refresh of the runbook: running from its log's start until its exit status lands beside it.
fn refresh(slug: &str, cache: &Path) -> Value {
    let log = cache.join(format!("runbook-refresh-{slug}.log"));
    let exit = cache.join(format!("runbook-refresh-{slug}.exit"));
    let (since, ended) = (stamp(&log), std::fs::metadata(&exit).and_then(|m| m.modified()).ok().map(ms));
    let code = std::fs::read_to_string(&exit).ok().and_then(|s| s.trim().parse::<i64>().ok());
    json!({"running": since.is_some() && ended.is_none(), "since": since, "ended": ended, "code": code, "log": log})
}

/// What a status starts with, as a word the mod colors by.
fn tone(status: &str) -> &'static str {
    match status.chars().next() {
        Some('✅') => "done",
        Some('🚀') => "live",
        Some('🟡') => "progress",
        Some('⏳') => "waiting",
        Some('⬜') => "todo",
        Some('❌') => "blocked",
        _ => "other",
    }
}

/// The status without its leading emoji.
fn status_label(status: &str) -> String {
    let first = status.chars().next().filter(|c| !c.is_alphanumeric() && !c.is_ascii());
    match first {
        Some(c) => status[c.len_utf8()..].trim_start_matches('\u{fe0f}').trim().to_string(),
        None => status.trim().to_string(),
    }
}

fn step_json(book: &Book, step: &Step, sessions: &HashMap<String, Value>, inbox: &Value) -> Value {
    let session = sessions.get(&handoff_id(book, step));
    let pr = pr_ref(step).map(|(repo, number)| {
        let badge = handoff::live_badge(&repo, number, inbox).map(|(text, _)| {
            let (text, tone) = match text.as_str() {
                "ci✗" => ("CI failing", "destructive"),
                "open" => ("open", "warning"),
                "approved" => ("approved", "success"),
                _ => ("merged", "info"),
            };
            json!({"text": text, "tone": tone})
        });
        json!({"repo": repo, "number": number, "badge": badge})
    });
    let session = session.map(|h| {
        let state = h["state"].as_str().unwrap_or_default();
        let line = handoff::line(h).map(|(text, _)| text.trim_start_matches("↳ ").to_string());
        let stale = state == "sent" && !handoff::in_flight(h);
        let tone = match state {
            "sent" if stale => "destructive",
            "sent" | "waiting" => "warning",
            "running" => "working",
            "done" => "success",
            _ => "destructive",
        };
        json!({"state": state, "text": line, "tone": tone, "running": handoff::is_running(h), "failed": handoff::has_failed(h), "message": h["message"]})
    });
    json!({
        "key": crate::app::key(&book.slug, &step.n),
        "n": step.n,
        "step": step.step,
        "where": step.where_,
        "status": step.status,
        "label": status_label(&step.status),
        "tone": tone(&step.status),
        "command": step.command,
        "open": step.is_open(),
        "pr": pr,
        "session": session,
        "acts": step_acts(book, step, sessions).iter().map(act_json).collect::<Vec<_>>(),
    })
}

/// A note cut into plain text and `code`, a backtick nothing closes kept as written.
fn note_parts(note: &str) -> Vec<Value> {
    let ticks = note.matches('`').count();
    let mut parts = Vec::new();
    for (i, part) in note.split('`').enumerate() {
        let code = i % 2 == 1 && ticks > i;
        let text = if i % 2 == 1 && !code { format!("`{part}") } else { part.to_string() };
        if !text.is_empty() {
            parts.push(json!({"text": text, "code": code}));
        }
    }
    parts
}

// ---------------------------------------------------------------------------------------------------------------------
// Edits

const UNDO_DEPTH: usize = 50;

/// Each edit's files: (path, the text before, the text it wrote), newest edit last.
type Change = Vec<(PathBuf, String, String)>;
static UNDO: Mutex<Vec<Change>> = Mutex::new(Vec::new());

fn undo_stack() -> std::sync::MutexGuard<'static, Vec<Change>> {
    UNDO.lock().unwrap_or_else(|e| e.into_inner())
}

fn can_undo() -> bool {
    undo_stack().last().is_some_and(|change| change.iter().all(|(path, _, after)| std::fs::read_to_string(path).ok().as_ref() == Some(after)))
}

fn remember(change: Change) {
    if change.is_empty() {
        return;
    }
    let mut undo = undo_stack();
    undo.push(change);
    if undo.len() > UNDO_DEPTH {
        undo.remove(0);
    }
}

fn find(dir: &Path, slug: &str) -> Result<Book, String> {
    book::read_books(dir).into_iter().find(|b| b.slug == slug).ok_or_else(|| format!("no runbook {slug} any more"))
}

fn column(name: &str) -> Result<Column, String> {
    Ok(match name {
        "command" => Column::Command,
        "status" => Column::Status,
        "step" => Column::Step,
        "where" => Column::Where,
        other => return Err(format!("no column {other}")),
    })
}

fn texts(value: Option<&Value>) -> Vec<String> {
    value.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

/// The mod's edits: `done [[slug, n], ...]` (their sessions told), `set [slug, n, column, value]` (column command,
/// status, step or where), `add [slug, after n or null, step, where, command]`, `undo []`.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let today = chrono::Local::now().format("%d %b").to_string();
    act_in(&book::dir(), &handoff::dir(), action, args, &today)
}

pub fn act_in(dir: &Path, handoffs: &Path, action: &str, args: &[Value], today: &str) -> Option<Result<String, String>> {
    let s = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or_default().to_string();
    Some(match action {
        "done" => done(dir, handoffs, args.first(), today),
        "set" => set(dir, &s(0), &s(1), &s(2), &s(3)),
        "add" => add(dir, &s(0), args.get(1).and_then(Value::as_str), &s(2), &s(3), &s(4)),
        "undo" => undo(),
        _ => return None,
    })
}

fn done(dir: &Path, handoffs: &Path, steps: Option<&Value>, today: &str) -> Result<String, String> {
    let steps: Vec<Vec<String>> = steps.and_then(Value::as_array).into_iter().flatten().map(|pair| texts(Some(pair))).filter(|p| p.len() == 2).collect();
    let mut slugs: Vec<&String> = steps.iter().map(|p| &p[0]).collect();
    slugs.dedup();
    let (mut change, mut count, mut names) = (Change::new(), 0, Vec::new());
    for slug in slugs {
        let book = find(dir, slug)?;
        let ns: Vec<String> = steps.iter().filter(|p| &p[0] == slug).map(|p| p[1].clone()).collect();
        let before = std::fs::read_to_string(&book.path).map_err(|e| e.to_string())?;
        let (after, found) = table::mark_done(&before, &ns, today);
        if after != before {
            table::write(&book.path, &after).map_err(|e| e.to_string())?;
            change.push((book.path.clone(), before, after));
        }
        let sessions = handoff::read(handoffs);
        for step in book.steps.iter().filter(|s| found.contains(&s.n)) {
            if let Some(h) = sessions.get(&handoff_id(&book, step)) {
                handoff::close(handoffs, h);
            }
        }
        count += found.len();
        names.push(format!("{} {}", book.slug.to_uppercase(), found.join(", ")));
    }
    remember(change);
    Ok(match count {
        0 => "nothing to mark done".into(),
        1 => format!("{} done", names.join("").replacen(' ', " step ", 1)),
        n => format!("{n} steps done · {}", names.join(" · ")),
    })
}

fn set(dir: &Path, slug: &str, n: &str, name: &str, value: &str) -> Result<String, String> {
    let column = column(name)?;
    let book = find(dir, slug)?;
    let before = std::fs::read_to_string(&book.path).map_err(|e| e.to_string())?;
    let after = table::set_cell(&before, n, column, value).ok_or_else(|| format!("step {n} is no longer in {}", slug.to_uppercase()))?;
    if after != before {
        table::write(&book.path, &after).map_err(|e| e.to_string())?;
        remember(vec![(book.path, before, after)]);
    }
    Ok(format!("{} step {n}: {name} saved", slug.to_uppercase()))
}

fn add(dir: &Path, slug: &str, after: Option<&str>, step: &str, where_: &str, command: &str) -> Result<String, String> {
    if step.trim().is_empty() {
        return Err("a step needs its text".into());
    }
    let book = find(dir, slug)?;
    let before = std::fs::read_to_string(&book.path).map_err(|e| e.to_string())?;
    let (text, n) = table::insert_step(&before, after, step, where_, command);
    table::write(&book.path, &text).map_err(|e| e.to_string())?;
    remember(vec![(book.path, before, text)]);
    Ok(format!("{} step {n} added", slug.to_uppercase()))
}

fn undo() -> Result<String, String> {
    let mut undo = undo_stack();
    let Some(change) = undo.pop() else { return Ok("nothing to undo".into()) };
    if !change.iter().all(|(path, _, after)| std::fs::read_to_string(path).ok().as_ref() == Some(after)) {
        // Undoing past another writer's change would drop it.
        undo.clear();
        return Ok("the runbook changed since: nothing to undo".into());
    }
    for (path, before, _) in &change {
        table::write(path, before).map_err(|e| e.to_string())?;
    }
    Ok("undone".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOK: &str = "# ENG-1 · Quotas\nupdated: today\n\n| # | Step | Where | Status | Command |\n|---|---|---|---|---|\n\
                        | 1 | Merge | api#12 | ✅ merged | |\n| 2 | Backfill | prod | ⬜ to do | `bin/backfill` |\n\n## Notes\n- run `x` first\n";

    #[test]
    fn the_view_and_its_edits() {
        let root = tempfile::tempdir().unwrap();
        let (dir, handoffs, cache) = (root.path().join("runbooks"), root.path().join("handoffs"), root.path().join("cache"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(dir.join("eng-1.md"), BOOK).unwrap();
        let inbox = json!({"merged": [{"repo": "api", "number": 12}]});
        let view = model(&dir, &handoffs, &cache, &inbox);
        let book = &view["books"][0];
        assert_eq!((book["name"].as_str(), book["todo"].as_u64(), view["left"].as_u64()), (Some("ENG-1"), Some(1), Some(1)));
        assert_eq!(book["steps"][0]["pr"]["badge"]["text"], "merged");
        assert_eq!((book["steps"][0]["tone"].as_str(), book["steps"][0]["label"].as_str()), (Some("done"), Some("merged")));
        assert_eq!(book["steps"][1]["acts"][0]["id"], "run");
        assert_eq!(book["notes"][0][1], json!({"text": "x", "code": true}));
        assert_eq!(book["refresh"]["running"], false);

        let run = |action: &str, args: Value| act_in(&dir, &handoffs, action, args.as_array().unwrap(), "2 Nov").unwrap();
        assert_eq!(run("done", json!([[["eng-1", "2"]]])).unwrap(), "ENG-1 step 2 done");
        assert!(std::fs::read_to_string(dir.join("eng-1.md")).unwrap().contains("| 2 | Backfill | prod | ✅ done 2 Nov |"));
        assert_eq!(run("undo", json!([])).unwrap(), "undone");
        run("set", json!(["eng-1", "2", "command", "bin/backfill --all"])).unwrap();
        assert!(std::fs::read_to_string(dir.join("eng-1.md")).unwrap().contains("`bin/backfill --all`"));
        assert_eq!(run("add", json!(["eng-1", "1", "Announce", "slack", ""])).unwrap(), "ENG-1 step 2 added");
        assert!(run("add", json!(["eng-1", null, " ", "", ""])).is_err());
        std::fs::write(dir.join("eng-1.md"), BOOK).unwrap();
        assert_eq!(run("undo", json!([])).unwrap(), "the runbook changed since: nothing to undo");
        assert!(act_in(&dir, &handoffs, "run_step", &[], "x").is_none());

        std::fs::write(cache.join("runbook-refresh-eng-1.log"), "").unwrap();
        assert_eq!(model(&dir, &handoffs, &cache, &inbox)["books"][0]["refresh"]["running"], true);
        std::fs::write(cache.join("runbook-refresh-eng-1.exit"), "1\n").unwrap();
        let refresh = &model(&dir, &handoffs, &cache, &inbox)["books"][0]["refresh"];
        assert_eq!((refresh["running"].as_bool(), refresh["code"].as_i64()), (Some(false), Some(1)));
    }
}
