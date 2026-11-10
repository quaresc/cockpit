//! The review view for the Herdr app's cockpit mod: a repo's changes in one of three scopes (uncommitted, the branch
//! against its base, its pull request) drawn like a show diff, the comments made on it sent to the agent working there
//! as one message, and the guided walkthrough of those changes (`crate::walkthrough`).

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::gui::diff_model;
use crate::patch::git_diff;
use crate::walkthrough;

/// How long a pull request's facts and diff are kept before `gh` is asked again.
const PR_FOR: Duration = Duration::from_secs(60);

fn git(repo: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false", "-C", repo])
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The repository a folder is in, as git names it (symlinks resolved), so the view and the actions agree on it.
fn toplevel(folder: &str) -> Option<String> {
    git(folder, &["rev-parse", "--show-toplevel"])
}

/// The branch a branch's changes are counted from: the remote's default, else main or master.
fn base_of(repo: &str) -> Option<String> {
    git(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).or_else(|| {
        ["origin/main", "main", "origin/master", "master"]
            .into_iter()
            .find(|b| git(repo, &["rev-parse", "--verify", "--quiet", b]).is_some())
            .map(str::to_string)
    })
}

/// What `gh` said about the branch's pull request, and its diff, kept a minute per repo and branch.
type Prs = HashMap<(String, String), (Instant, Result<(Value, String), String>)>;
static PRS: LazyLock<Mutex<Prs>> = LazyLock::new(Default::default);

fn gh(repo: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new("gh").args(args).current_dir(repo).stdin(Stdio::null()).output().map_err(|e| format!("gh did not start: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().lines().next().unwrap_or("gh failed").to_string())
    }
}

fn pull_request(repo: &str, branch: &str) -> Result<(Value, String), String> {
    let key = (repo.to_string(), branch.to_string());
    if let Some((at, found)) = PRS.lock().unwrap_or_else(|e| e.into_inner()).get(&key)
        && at.elapsed() < PR_FOR
    {
        return found.clone();
    }
    let found = gh(repo, &["pr", "view", "--json", "number,title,url,baseRefName,headRefName,state,isDraft"]).and_then(|facts| {
        let facts: Value = serde_json::from_str(&facts).map_err(|e| e.to_string())?;
        let patch = gh(repo, &["pr", "diff", "--color", "never"])?;
        Ok((facts, patch))
    });
    PRS.lock().unwrap_or_else(|e| e.into_inner()).insert(key, (Instant::now(), found.clone()));
    found
}

/// The scope's patch and what the header says about it.
pub fn patch_of(repo: &str, scope: &str) -> Result<(String, Value), String> {
    let branch = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    match scope {
        "branch" => {
            let base = base_of(repo).ok_or("no base branch found (origin/HEAD, main or master)")?;
            let fork = git(repo, &["merge-base", "HEAD", &base]).ok_or_else(|| format!("{branch} shares no history with {base}"))?;
            let ahead = git(repo, &["rev-list", "--count", &format!("{fork}..HEAD")]).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
            Ok((git_diff(repo, Some(&fork))?, json!({"branch": branch, "base": base, "commits": ahead})))
        }
        "pr" => {
            let (facts, patch) = pull_request(repo, &branch)?;
            Ok((patch, json!({"branch": branch, "base": facts["baseRefName"], "pr": facts})))
        }
        _ => Ok((git_diff(repo, None)?, json!({"branch": branch, "base": "HEAD"}))),
    }
}

/// `{repo, scope, since}`: the changes (`{same: true}` with the walkthrough when the patch hashes to `since`), where
/// they come from, and the walkthrough of that scope as it stands over them.
pub fn view(args: &Value, _doc: &dyn Fn(&str) -> Value) -> Value {
    let Some(asked) = args["repo"].as_str().filter(|r| r.starts_with('/')) else { return json!({"error": "no repository: focus an agent working in one"}) };
    let Some(repo) = toplevel(asked) else { return json!({"error": format!("{asked} is not in a git repository")}) };
    let scope = args["scope"].as_str().filter(|s| matches!(*s, "branch" | "pr")).unwrap_or("uncommitted");
    let (patch, about) = match patch_of(&repo, scope) {
        Ok(found) => found,
        Err(error) => return json!({"repo": repo, "scope": scope, "error": error}),
    };
    let mut out = diff_model(&patch, args["since"].as_str());
    let name = Path::new(&repo).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    out["repo"] = json!(repo);
    out["name"] = json!(name);
    out["scope"] = json!(scope);
    out["about"] = about;
    out["walkthrough"] = walkthrough::state(&repo, scope, &patch);
    out
}

/// `send [terminal, {scope, repo, comments: [{path, line, old, code, text}]}]` types the review into the agent as one
/// line; `walkthrough [repo, scope, model]` starts one; `stop_walkthrough [repo, scope]` stops it.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let first = args.first().and_then(Value::as_str).unwrap_or_default();
    Some(match action {
        "send" => send(first, args.get(1).unwrap_or(&Value::Null)),
        "walkthrough" => {
            let scope = args.get(1).and_then(Value::as_str).unwrap_or("uncommitted");
            let model = args.get(2).and_then(Value::as_str).unwrap_or("haiku");
            let repo = toplevel(first).ok_or_else(|| format!("{first} is not in a git repository"));
            repo.and_then(|repo| patch_of(&repo, scope).and_then(|(patch, about)| walkthrough::start(&repo, scope, model, &patch, &about)))
        }
        "stop_walkthrough" => {
            walkthrough::stop(&toplevel(first).unwrap_or_else(|| first.to_string()), args.get(1).and_then(Value::as_str).unwrap_or("uncommitted"))
        }
        _ => return None,
    })
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The review as the agent reads it: one line (a newline would send it early), each comment numbered with its place
/// and the code it is about.
pub fn message(review: &Value) -> Result<String, String> {
    let comments = review["comments"].as_array().filter(|c| !c.is_empty()).ok_or("no comments to send")?;
    let what = match review["scope"].as_str() {
        Some("branch") => "this branch's changes",
        Some("pr") => "the pull request's changes",
        Some("show") => "the diff I was shown",
        Some("file") => "a file I was reading",
        _ => "the uncommitted changes",
    };
    let items: Vec<String> = comments
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let side = if c["old"] == true { " (removed line)" } else { "" };
            let code = c["code"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| format!(" `{}`", one_line(s).chars().take(80).collect::<String>()))
                .unwrap_or_default();
            format!(
                "{}) {}:{}{side}{code}: {}",
                i + 1,
                one_line(c["path"].as_str().unwrap_or("?")),
                c["line"],
                one_line(c["text"].as_str().unwrap_or_default())
            )
        })
        .collect();
    let count = if items.len() == 1 { "1 comment".to_string() } else { format!("{} comments", items.len()) };
    Ok(format!("My review of {what} ({count}), please address each: {}", items.join(" ")))
}

fn send(terminal: &str, review: &Value) -> Result<String, String> {
    let line = message(review)?;
    if terminal.is_empty() {
        return Err("no agent to send it to: focus the agent's terminal first".into());
    }
    // An app terminal comes as its address (t3); a herdr pane id or a handle (app:t3) as it is.
    let handle = if terminal.contains(':') || terminal.contains('-') { terminal.to_string() } else { format!("{}{terminal}", cockpit_core::frontend::APP) };
    if cockpit_core::frontend::run_in(&handle, &line) {
        let n = review["comments"].as_array().map_or(0, Vec::len);
        Ok(format!("review sent to the agent ({n} comment{})", if n == 1 { "" } else { "s" }))
    } else {
        Err("the agent's terminal did not take it".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_review_is_one_numbered_line() {
        let review = json!({"scope": "branch", "comments": [
            {"path": "src/a.rs", "line": 12, "code": "let x =\n 2;", "text": "use the\nconstant"},
            {"path": "src/\nb.rs", "line": 3, "old": true, "text": "keep this"},
        ]});
        let line = message(&review).unwrap();
        assert_eq!(
            line,
            "My review of this branch's changes (2 comments), please address each: 1) src/a.rs:12 `let x = 2;`: use the constant 2) src/ b.rs:3 (removed line): keep this"
        );
        assert!(!line.contains('\n'));
        assert!(message(&json!({"comments": []})).is_err());
    }

    #[test]
    fn the_uncommitted_scope_reads_a_repo() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().to_string_lossy().into_owned();
        let run = |args: &[&str]| assert!(Command::new("git").args(["-C", &repo]).args(args).output().unwrap().status.success());
        run(&["init", "-q"]);
        run(&["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "start"]);
        std::fs::write(root.path().join("new.txt"), "hello\n").unwrap();
        let v = view(&json!({"repo": repo, "scope": "uncommitted"}), &|_| Value::Null);
        assert_eq!(v["files"][0]["path"], "new.txt");
        assert_eq!(v["walkthrough"]["status"], "none");
        let again = view(&json!({"repo": repo, "since": v["hash"]}), &|_| Value::Null);
        assert_eq!(again["same"], true);
        assert!(view(&json!({"repo": "/nonexistent-dir"}), &|_| Value::Null)["error"].is_string());
    }
}
