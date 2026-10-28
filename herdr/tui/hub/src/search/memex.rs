//! Past Claude sessions matching a query, from memex (github.com/nicosuave/memex) when it is on PATH: its search names
//! the sessions, its session listing, read ahead in the background, gives each one's title, folder and last activity.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate};
use serde_json::{Value, json};

use crate::proc;

/// Sessions shown after cockpit's own entries.
const BEST: usize = 6;
/// What the palette waits for memex, within the 400 ms it gives the hub; a slower answer is kept for the next search of
/// the same query.
const WAIT: Duration = Duration::from_millis(250);
/// Shorter queries match too much to be worth a memex run on each key.
const SHORTEST: usize = 3;
/// A query's sessions are searched again after this.
const FRESH: Duration = Duration::from_secs(60);
/// A session's title and folder are read again after this, and the whole listing ahead of the searches.
const META_FRESH: Duration = Duration::from_secs(600);
/// Sessions read ahead: more than most people keep.
const LISTED: &str = "2000";
/// The longest a memex run may take before it is killed.
const RUN_LIMIT: Duration = Duration::from_secs(10);
const QUIET: [&str; 2] = ["--non-interactive", "--no-update-check"];

type Found = Arc<Mutex<HashMap<String, (Instant, Vec<Value>)>>>;

/// Searches memex in the background, waiting a moment for the answer, and keeps each query's sessions for a minute.
pub struct Finder {
    wait: Duration,
    found: Found,
}

static FINDER: LazyLock<Finder> = LazyLock::new(|| Finder::new(WAIT));

/// The past sessions matching the query, as palette entries; none without memex on PATH.
pub fn sessions(query: &str) -> Vec<Value> {
    FINDER.find(proc::which("memex").as_deref(), query)
}

impl Finder {
    pub fn new(wait: Duration) -> Self {
        Self { wait, found: Arc::default() }
    }

    pub fn find(&self, bin: Option<&Path>, query: &str) -> Vec<Value> {
        let (Some(bin), query) = (bin, query.trim()) else {
            return Vec::new();
        };
        if query.chars().count() < SHORTEST {
            return Vec::new();
        }
        let key = format!("{}\n{query}", bin.display());
        if let Some((_, entries)) = self.found.lock().ok().and_then(|f| f.get(&key).filter(|(at, _)| at.elapsed() < FRESH).cloned()) {
            return entries;
        }
        read_ahead(bin);
        let (tx, rx) = channel();
        let (found, bin, query) = (self.found.clone(), bin.to_path_buf(), query.to_string());
        std::thread::spawn(move || {
            let entries = lookup(&bin, &query, Local::now().date_naive());
            if let Ok(mut found) = found.lock() {
                found.retain(|_, (at, _)| at.elapsed() < FRESH);
                found.insert(key, (Instant::now(), entries.clone()));
            }
            let _ = tx.send(entries);
        });
        rx.recv_timeout(self.wait).unwrap_or_default()
    }
}

/// The JSON lines of a memex command; its flags for a quiet run go right after the subcommand, before any `--`.
fn memex(bin: &Path, command: &str, args: &[&str]) -> Vec<Value> {
    let words = [command].into_iter().chain(QUIET).chain(args.iter().copied());
    let argv: Vec<String> = std::iter::once(bin.to_string_lossy().into_owned()).chain(words.map(str::to_string)).collect();
    let out = proc::run(&argv, RUN_LIMIT, None);
    out.stdout.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(Value::is_object).collect()
}

/// The best sessions for the query, each with its title and folder; subagents are left out, they cannot be resumed.
pub fn lookup(bin: &Path, query: &str, today: NaiveDate) -> Vec<Value> {
    let fields = "session_id,ts,project,source_path,snippet";
    let args = ["--format", "jsonl", "--source", "claude", "--unique-session", "--limit", "20", "--fields", fields, "--", query];
    let mut hits: Vec<Value> = Vec::new();
    for hit in memex(bin, "search", &args) {
        let id = hit["session_id"].as_str().unwrap_or_default();
        let is_sub = id.starts_with("agent-") || hit["source_path"].as_str().is_some_and(|p| p.contains("/subagents/"));
        if !id.is_empty() && !is_sub && !hits.iter().any(|h| h["session_id"] == id) {
            hits.push(hit);
        }
    }
    hits.truncate(BEST);
    let metas: Vec<Value> = std::thread::scope(|scope| {
        let runs: Vec<_> = hits.iter().map(|h| scope.spawn(|| meta(bin, h["session_id"].as_str().unwrap_or_default()))).collect();
        runs.into_iter().map(|r| r.join().unwrap_or(Value::Null)).collect()
    });
    hits.iter().zip(metas).map(|(hit, meta)| entry(hit, &meta, today)).collect()
}

/// Each session's row in memex's listing (label, cwd, last_at), by memex binary and session id.
static METAS: LazyLock<Mutex<HashMap<String, (Instant, Value)>>> = LazyLock::new(Mutex::default);

fn keep(bin: &Path, rows: Vec<Value>) {
    if let Ok(mut metas) = METAS.lock() {
        for row in rows {
            let id = row["session_id"].as_str().unwrap_or_default();
            metas.insert(format!("{}\n{id}", bin.display()), (Instant::now(), row));
        }
    }
}

/// Reads memex's whole listing in the background every ten minutes, so a search needs one memex run, not one per hit.
fn read_ahead(bin: &Path) {
    static STARTED: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Mutex::default);
    let Ok(mut started) = STARTED.lock() else { return };
    let key = bin.display().to_string();
    if started.get(&key).is_some_and(|at| at.elapsed() < META_FRESH) {
        return;
    }
    started.insert(key, Instant::now());
    let bin = bin.to_path_buf();
    std::thread::spawn(move || keep(&bin, memex(&bin, "sessions", &["--format", "jsonl", "--source", "claude", "--limit", LISTED])));
}

/// The session's row: read ahead, else asked for alone and kept ten minutes.
fn meta(bin: &Path, id: &str) -> Value {
    let key = format!("{}\n{id}", bin.display());
    if let Some((_, row)) = METAS.lock().ok().and_then(|m| m.get(&key).filter(|(at, _)| at.elapsed() < META_FRESH).cloned()) {
        return row;
    }
    let row = memex(bin, "sessions", &["--format", "jsonl", "--origin", "all", "--session-id", id, "--limit", "1"]).into_iter().next().unwrap_or(Value::Null);
    if let Ok(mut metas) = METAS.lock() {
        metas.insert(key, (Instant::now(), row.clone()));
    }
    row
}

fn entry(hit: &Value, meta: &Value, today: NaiveDate) -> Value {
    let id = hit["session_id"].as_str().unwrap_or_default();
    let label = meta["label"].as_str().map(str::trim).filter(|l| !l.is_empty());
    let snippet = hit["snippet"].as_str().map(|s| s.trim_start_matches('…').trim()).filter(|s| !s.is_empty());
    let title: String = label.or(snippet).unwrap_or(id).chars().take(80).collect();
    let project = meta["project"].as_str().or(hit["project"].as_str()).filter(|p| !p.is_empty());
    let when = meta["last_at"].as_str().or(hit["ts"].as_str()).and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Local).date_naive());
    let help: Vec<String> = project.map(str::to_string).into_iter().chain(when.map(|d| d.format("%a %d %b").to_string())).chain(when.map(|d| age(d, today))).collect();
    let mut target = json!({"app": "session", "key": id, "title": title});
    if let Some(cwd) = meta["cwd"].as_str().filter(|c| !c.is_empty()) {
        target["cwd"] = json!(cwd);
    }
    json!({"app": "session", "kind": "session", "title": title, "help": help.join(" · "), "target": target})
}

/// How long ago a day was, in the unit that reads best.
pub fn age(day: NaiveDate, today: NaiveDate) -> String {
    match (today - day).num_days() {
        ..=0 => "today".into(),
        1 => "yesterday".into(),
        n @ 2..14 => format!("{n} days ago"),
        n @ 14..60 => format!("{} weeks ago", n / 7),
        n @ 60..730 => format!("{} months ago", n / 30),
        n => format!("{} years ago", n / 365),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 28).unwrap()
    }

    /// tests/fixtures/memex.sh in its own folder, its folders under /acme, answering after `delay` seconds.
    fn fake(dir: &Path, delay: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = include_str!("../../tests/fixtures/memex.sh").replace("@BOX@", "/acme");
        let path = dir.join("memex");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("delay"), delay).unwrap();
        path
    }

    fn runs(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("memex.log")).unwrap_or_default().lines().map(str::to_string).collect()
    }

    #[test]
    fn sessions_with_their_title_folder_and_age() {
        let dir = tempfile::tempdir().unwrap();
        let found = lookup(&fake(dir.path(), "0"), "sign-in", today());
        assert_eq!(found.len(), 2, "the subagent is left out: {found:?}");
        assert_eq!(found[0]["title"], "Google sign-in for ENG-401");
        assert_eq!(found[0]["help"], "api · Tue 27 Oct · yesterday");
        assert_eq!(found[0]["target"], json!({"app": "session", "key": "s-login", "cwd": "/acme/repos/api", "title": "Google sign-in for ENG-401"}));
        assert_eq!((found[1]["title"].as_str(), found[1]["help"].as_str()), (Some("dark mode tokens for the sign-in page"), Some("web · Wed 02 Sep · 8 weeks ago")));
        let search = &runs(dir.path())[0];
        assert!(search.starts_with("search --non-interactive --no-update-check --format jsonl --source claude --unique-session") && search.contains("-- sign-in"), "{search}");
    }

    #[test]
    fn no_memex_finds_nothing() {
        assert!(Finder::new(WAIT).find(None, "sign-in").is_empty());
    }

    #[test]
    fn short_queries_are_not_searched() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake(dir.path(), "0");
        assert!(Finder::new(Duration::from_secs(5)).find(Some(&bin), " si ").is_empty());
        assert!(runs(dir.path()).is_empty());
    }

    #[test]
    fn a_slow_memex_is_not_waited_for_but_kept_for_the_next_search() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake(dir.path(), "0.5");
        let finder = Finder::new(Duration::from_millis(100));
        let start = Instant::now();
        assert!(finder.find(Some(&bin), "dark mode").is_empty());
        assert!(start.elapsed() < Duration::from_millis(400), "waited {:?}", start.elapsed());
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(finder.find(Some(&bin), "dark mode")[0]["target"]["key"], "s-dark");
    }

    #[test]
    fn a_query_is_searched_once_a_minute() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake(dir.path(), "0");
        let finder = Finder::new(Duration::from_secs(5));
        assert_eq!(finder.find(Some(&bin), "google").len(), 1);
        assert_eq!(finder.find(Some(&bin), " google ").len(), 1);
        assert_eq!(runs(dir.path()).iter().filter(|r| r.starts_with("search")).count(), 1);
    }

    #[test]
    fn the_listing_read_ahead_spares_a_run_per_session() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake(dir.path(), "0");
        let finder = Finder::new(Duration::from_secs(5));
        assert_eq!(finder.find(Some(&bin), "google").len(), 1);
        let start = Instant::now();
        while !runs(dir.path()).iter().any(|r| r.starts_with("sessions") && !r.contains("--session-id")) {
            assert!(start.elapsed() < Duration::from_secs(5), "no listing read ahead: {:?}", runs(dir.path()));
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(finder.find(Some(&bin), "dark mode")[0]["help"], "web · Wed 02 Sep · 8 weeks ago");
        assert!(!runs(dir.path()).iter().any(|r| r.contains("--session-id s-dark")), "{:?}", runs(dir.path()));
    }

    #[test]
    fn a_failing_memex_finds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("memex");
        std::fs::write(&bin, "#!/bin/sh\necho 'no index' >&2\nexit 2\n").unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        assert!(lookup(&bin, "x", today()).is_empty());
    }

    #[test]
    fn ages() {
        let day = |d: u32, m: u32| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        let ages: Vec<String> = [day(28, 10), day(27, 10), day(20, 10), day(1, 10), day(1, 6)].into_iter().map(|d| age(d, today())).collect();
        assert_eq!(ages, ["today", "yesterday", "8 days ago", "3 weeks ago", "4 months ago"]);
        assert_eq!(age(NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(), today()), "3 years ago");
    }
}
