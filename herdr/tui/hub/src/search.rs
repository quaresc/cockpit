//! The palette's search across the apps: one index of the inbox's PRs and tickets, the runbooks and their steps, the
//! show views and their blocks, the to-do items and the meetings, rebuilt when the watcher says a file changed.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cockpit_core::files;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde_json::{Value, json};

use crate::act::{read_books, runbooks_dir};
use crate::py::{list, text};

const BEST: usize = 50;
const OPEN: [&str; 4] = ["⬜", "⏳", "🟡", "❌"];

#[derive(Clone, Debug)]
pub struct Entry {
    pub app: &'static str,
    pub kind: &'static str,
    pub title: String,
    pub help: String,
    pub key: String,
}

impl Entry {
    fn json(&self) -> Value {
        json!({"app": self.app, "kind": self.kind, "title": self.title, "help": self.help, "target": {"app": self.app, "key": self.key}})
    }
}

static INDEX: Mutex<Option<(Instant, Vec<Entry>)>> = Mutex::new(None);
static DIRTY: AtomicBool = AtomicBool::new(true);
/// Built again at least this often, should the watcher miss a change.
const STALE: Duration = Duration::from_secs(60);

/// Something the index reads changed: the next search builds it again.
pub fn invalidate() {
    DIRTY.store(true, Ordering::SeqCst);
}

/// The entries matching the query, best first, 50 at most.
pub fn search(query: &str) -> Vec<Value> {
    let Ok(mut index) = INDEX.lock() else {
        return Vec::new();
    };
    let is_stale = index.as_ref().is_none_or(|(at, _)| at.elapsed() > STALE);
    if DIRTY.swap(false, Ordering::SeqCst) || is_stale {
        *index = Some((Instant::now(), build(&files::home())));
    }
    best(index.as_ref().map(|(_, e)| e.as_slice()).unwrap_or_default(), query).into_iter().map(Entry::json).collect()
}

pub fn best<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| pattern.score(Utf32Str::new(&format!("{} {}", e.title, e.help), &mut buf), &mut matcher).map(|s| (s, i)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().take(BEST).map(|(_, i)| &entries[i]).collect()
}

/// Every searchable item under this home.
pub fn build(home: &Path) -> Vec<Entry> {
    let cache = home.join(".cache/cockpit");
    let mut out = Vec::new();
    inbox(&files::load(&cache.join("inbox.json")).unwrap_or(Value::Null), &mut out);
    let runbooks = if std::env::var("COCKPIT_RUNBOOKS_DIR").is_ok() { runbooks_dir() } else { home.join(".config/cockpit/runbooks") };
    runbook(&read_books(&runbooks), &mut out);
    show(&cache.join("show"), &mut out);
    todo(&std::fs::read_to_string(home.join(".config/cockpit/todo.md")).unwrap_or_default(), &mut out);
    meetings(&files::load(&cache.join("meetings.json")).unwrap_or(Value::Null), &mut out);
    out
}

fn entry(app: &'static str, kind: &'static str, title: String, help: String, key: String) -> Entry {
    Entry { app, kind, title, help, key }
}

fn pr_title(pr: &Value) -> String {
    format!("{}#{} {}", text(&pr["repo"]), text(&pr["number"]), text(&pr["title"]))
}

fn inbox(data: &Value, out: &mut Vec<Entry>) {
    for pr in list(&data["mine"]) {
        let state = [pr["ci"].as_str().map(|ci| format!("CI {}", ci.to_lowercase())), pr["review"].as_str().map(|r| r.to_lowercase().replace('_', " "))];
        let help = std::iter::once(text(&pr["branch"])).chain(state.into_iter().flatten()).collect::<Vec<_>>().join(" · ");
        out.push(entry("inbox", "pr", pr_title(pr), help, text(&pr["url"])));
    }
    for (field, who) in [("direct", "review for you"), ("team", "team review")] {
        for pr in list(&data[field]) {
            let author = pr["authorName"].as_str().filter(|n| !n.is_empty()).unwrap_or(pr["author"].as_str().unwrap_or_default());
            out.push(entry("inbox", "review", pr_title(pr), format!("{who} · {author}"), text(&pr["url"])));
        }
    }
    for ticket in list(&data["tickets"]) {
        let id = text(&ticket["id"]);
        let help = [ticket["status"].as_str(), ticket["project"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        out.push(entry("inbox", "ticket", format!("{id} {}", text(&ticket["title"])), help, id));
    }
}

fn runbook(books: &[Value], out: &mut Vec<Entry>) {
    for book in books {
        let slug = text(&book["slug"]);
        let title = book["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(&slug).to_string();
        let steps = list(&book["steps"]);
        let todo = steps.iter().filter(|s| s["status"].as_str().is_some_and(|st| OPEN.iter().any(|o| st.starts_with(o)))).count();
        out.push(entry("runbook", "runbook", title.clone(), format!("{todo} to do · updated {}", text(&book["updated"])), slug.clone()));
        for step in steps {
            let n = text(&step["n"]);
            let help = format!("{title} · {} · {}", text(&step["status"]), text(&step["where"]));
            out.push(entry("runbook", "step", format!("{n}. {}", text(&step["step"])), help, format!("{slug}#{n}")));
        }
    }
}

fn show(dir: &Path, out: &mut Vec<Entry>) {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".json")).map(str::to_string))
        .collect();
    names.sort();
    for name in names {
        let Some(view) = files::load(&dir.join(format!("{name}.json"))) else {
            continue;
        };
        let title = view["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(&name).to_string();
        let blocks = list(&view["blocks"]);
        out.push(entry("show", "view", title.clone(), format!("{name} · {} blocks", blocks.len()), name.clone()));
        for (i, block) in blocks.iter().enumerate() {
            if let Some(block_title) = block["title"].as_str().filter(|t| !t.is_empty()) {
                out.push(entry("show", "block", block_title.into(), format!("{title} · {}", text(&block["type"])), format!("{name}#{i}")));
            }
        }
    }
}

fn todo(content: &str, out: &mut Vec<Entry>) {
    for line in content.lines().map(str::trim) {
        let (done, rest) = match (line.strip_prefix("- [ ] "), line.strip_prefix("- [x] ")) {
            (Some(rest), _) => (false, rest),
            (_, Some(rest)) => (true, rest),
            _ => continue,
        };
        out.push(entry("todo", "todo", rest.into(), if done { "done".into() } else { "to do".into() }, rest.into()));
    }
}

fn meetings(data: &Value, out: &mut Vec<Entry>) {
    for meeting in list(&data["meetings"]) {
        let when = meeting["start"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&chrono::Local).format("%a %d %b %H:%M").to_string())
            .unwrap_or_default();
        out.push(entry("meetings", "meeting", text(&meeting["title"]), when, text(&meeting["id"])));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join(".cache/cockpit");
        std::fs::create_dir_all(cache.join("show")).unwrap();
        std::fs::create_dir_all(home.path().join(".config/cockpit/runbooks")).unwrap();
        let inbox = json!({"mine": [{"repo": "api", "number": 4, "title": "Login page", "url": "https://x/api/pull/4", "branch": "me/eng-401-login", "ci": "FAILURE", "review": "CHANGES_REQUESTED"}],
                           "direct": [{"repo": "web", "number": 9, "title": "Dark mode", "url": "https://x/web/pull/9", "author": "maya", "authorName": "Maya Haddad"}],
                           "team": [], "tickets": [{"id": "ENG-401", "title": "Sign in with Google", "status": "In Progress", "project": "Auth"}]});
        files::write(&cache.join("inbox.json"), &inbox).unwrap();
        files::write(
            &cache.join("show/demo.json"),
            &json!({"title": "Quotas demo", "blocks": [{"type": "chart", "title": "Blocked previews"}, {"type": "markdown"}]}),
        )
        .unwrap();
        files::write(&cache.join("meetings.json"), &json!({"meetings": [{"id": "m1", "title": "Team daily", "start": "2026-10-28T09:45:00+01:00"}]}))
            .unwrap();
        std::fs::write(home.path().join(".config/cockpit/todo.md"), "# To-do\n\n- [ ] call the bank\n- [x] ship ENG-401\n").unwrap();
        std::fs::write(home.path().join(".config/cockpit/runbooks/eng-401.md"), "# ENG-401 · Google sign-in\nupdated: today\n\n| # | Step | Where | Status | Command |\n|---|---|---|---|---|\n| 1 | Run the migration | api | ⬜ to do | mix ecto.migrate |\n").unwrap();
        home
    }

    #[test]
    fn indexes_every_app() {
        let home = home();
        let entries = build(home.path());
        let kinds: Vec<&str> = entries.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, ["pr", "review", "ticket", "runbook", "step", "view", "block", "todo", "todo", "meeting"]);
        let step = entries.iter().find(|e| e.kind == "step").unwrap();
        assert_eq!((step.key.as_str(), step.title.as_str()), ("eng-401#1", "1. Run the migration"));
        assert_eq!(entries.iter().find(|e| e.kind == "block").unwrap().key, "demo#0");
    }

    #[test]
    fn finds_the_best_first() {
        let home = home();
        let entries = build(home.path());
        let found = best(&entries, "migration");
        assert_eq!(found[0].key, "eng-401#1");
        let found = best(&entries, "dark");
        assert_eq!((found[0].app, found[0].key.as_str()), ("inbox", "https://x/web/pull/9"));
        assert!(best(&entries, "zzzzqqq").is_empty());
    }

    #[test]
    fn targets_carry_the_app() {
        let home = home();
        let entries = build(home.path());
        let bank = best(&entries, "bank")[0].json();
        assert_eq!(bank["target"], json!({"app": "todo", "key": "call the bank"}));
    }

    #[test]
    fn at_most_fifty() {
        let many: Vec<Entry> = (0..80).map(|i| entry("todo", "todo", format!("item {i}"), String::new(), i.to_string())).collect();
        assert_eq!(best(&many, "item").len(), 50);
    }
}
