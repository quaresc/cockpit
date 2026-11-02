//! The linked worktrees of the repos in the repos folder and herdr's worktree folder, with their uncommitted changes;
//! git is asked again at most every 30 s, since every rebuild asks.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cockpit_core::files;
use serde_json::{Value, json};

const FRESH: Duration = Duration::from_secs(30);

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
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

fn list(config: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for main in checkouts(config) {
        let repo = main.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(porcelain) = git(&main, &["worktree", "list", "--porcelain"]) else { continue };
        for (path, branch) in linked(&porcelain) {
            if !Path::new(&path).is_dir() {
                continue;
            }
            let changes = git(Path::new(&path), &["--no-optional-locks", "status", "--porcelain"]).map(|s| s.lines().count()).unwrap_or(0);
            out.push(json!({"path": path, "repo": repo, "branch": branch, "main": main, "changes": changes}));
        }
    }
    out
}

/// The worktrees, from a scan at most 30 s old.
pub fn scan(config: &Value) -> Vec<Value> {
    static KEPT: Mutex<Option<(Instant, Vec<Value>)>> = Mutex::new(None);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, list)) = kept.as_ref().filter(|(at, _)| at.elapsed() < FRESH) {
        return list.clone();
    }
    let fresh = list(config);
    *kept = Some((Instant::now(), fresh.clone()));
    fresh
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
