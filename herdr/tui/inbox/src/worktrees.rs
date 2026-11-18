//! The linked worktrees of the repos in the repos folder and herdr's worktree folder, with their uncommitted changes and
//! what keeps each (`cockpit_core::cleanup`); git and lsof are asked again at most every 30 s, or once a worktree changed.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cockpit_core::{cleanup, files};
use serde_json::{Value, json};

const FRESH: Duration = Duration::from_secs(30);

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The repos folder of the config, `~` expanded.
pub fn repos_dir(config: &Value) -> PathBuf {
    let raw = config["reposDir"].as_str().unwrap_or("~");
    match raw.strip_prefix('~') {
        Some(rest) => files::home().join(rest.trim_start_matches('/')),
        None => PathBuf::from(raw),
    }
}

/// The main checkouts to ask: each git repo right under the repos folder.
fn checkouts(config: &Value) -> Vec<PathBuf> {
    let dir = repos_dir(config);
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.join(".git").is_dir()).collect();
    found.sort();
    found
}

/// A worktree list's linked worktrees (the main one, the first, left out): (path, branch).
pub fn linked(porcelain: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut path = None;
    let mut first = true;
    for line in porcelain.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = (!first).then(|| p.to_string());
            first = false;
        } else if let (Some(branch), Some(p)) = (line.strip_prefix("branch refs/heads/"), &path) {
            out.push((p.clone(), branch.to_string()));
        }
    }
    out
}

/// The commit the code host merged a branch at, from the inbox's `merged` list (`headSha`).
fn merged_at(merged: &[Value], repo: &str, branch: &str) -> Option<String> {
    merged.iter().find(|m| m["repo"] == repo && m["branch"] == branch).and_then(|m| m["headSha"].as_str()).filter(|s| !s.is_empty()).map(str::to_string)
}

fn list(config: &Value, merged: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    let facts = cleanup::Facts::now();
    for main in checkouts(config) {
        let repo = main.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(porcelain) = cleanup::git(&main, &["worktree", "list", "--porcelain"]) else { continue };
        for (path, branch) in linked(&porcelain) {
            if !Path::new(&path).is_dir() {
                continue;
            }
            let seen = cleanup::look(Path::new(&path), &cleanup::Facts { merged_at: merged_at(merged, &repo, &branch), ..facts.clone() });
            out.push(json!({"path": path, "repo": repo, "branch": branch, "main": main, "changes": seen.changes, "merged": seen.merged,
                            "blockers": cleanup::json(&seen.blockers)}));
        }
    }
    out
}

/// What a scan is good for: when it was made, the changes it saw, and the host's merges it was made with.
struct Kept {
    at: Instant,
    made: u64,
    merges: String,
    list: Vec<Value>,
    /// A scan runs in the background for `scan_later`.
    running: bool,
    /// Scans finished, for the TUI's stamp.
    done: u64,
}

static KEPT: Mutex<Option<Kept>> = Mutex::new(None);

/// The merges a scan depends on, as one key.
fn merges_key(merged: &[Value]) -> String {
    merged.iter().map(|m| format!("{}|{}|{}", m["repo"], m["branch"], m["headSha"])).collect::<Vec<_>>().join(",")
}

fn current(kept: &Option<Kept>, merges: &str) -> bool {
    kept.as_ref().is_some_and(|k| k.at.elapsed() < FRESH && k.made == cleanup::generation() && k.merges == merges)
}

/// The worktrees, from a scan at most 30 s old, made since the last change and with these merges; scanned here when not.
pub fn scan(config: &Value, merged: &[Value]) -> Vec<Value> {
    let key = merges_key(merged);
    {
        let kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
        if current(&kept, &key) {
            return kept.as_ref().map(|k| k.list.clone()).unwrap_or_default();
        }
    }
    let made = cleanup::generation();
    let list = list(config, merged);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let done = kept.as_ref().map_or(0, |k| k.done) + 1;
    *kept = Some(Kept { at: Instant::now(), made, merges: key, list: list.clone(), running: false, done });
    list
}

/// The last scan's worktrees at once, and a new scan off this thread when it is out of date; the screen is woken when
/// it lands (`generation` moves).
pub fn scan_later(config: &Value, merged: &[Value]) -> Vec<Value> {
    let key = merges_key(merged);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let last = kept.as_ref().map(|k| k.list.clone()).unwrap_or_default();
    if current(&kept, &key) || kept.as_ref().is_some_and(|k| k.running) {
        return last;
    }
    match kept.as_mut() {
        Some(k) => k.running = true,
        None => *kept = Some(Kept { at: Instant::now() - FRESH, made: 0, merges: String::new(), list: Vec::new(), running: true, done: 0 }),
    }
    let (config, merged) = (config.clone(), merged.to_vec());
    std::thread::spawn(move || {
        let made = cleanup::generation();
        let list = list(&config, &merged);
        let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
        let done = kept.as_ref().map_or(0, |k| k.done) + 1;
        *kept = Some(Kept { at: Instant::now(), made, merges: merges_key(&merged), list, running: false, done });
        drop(kept);
        cockpit_core::term::wake();
    });
    last
}

/// How many scans finished: a new one is news for the screen.
pub fn generation() -> u64 {
    KEPT.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(0, |k| k.done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_main_checkout_and_detached_ones_are_left_out() {
        let porcelain = "worktree /r/api\nHEAD 1\nbranch refs/heads/main\n\nworktree /w/api/eng-4\nHEAD 2\nbranch refs/heads/me/eng-4-login\n\nworktree /w/api/x\nHEAD 3\ndetached\n";
        assert_eq!(linked(porcelain), vec![("/w/api/eng-4".to_string(), "me/eng-4-login".to_string())]);
    }
}
