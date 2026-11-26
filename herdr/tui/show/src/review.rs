//! The review view for the Herdr app's cockpit mod: a repo's changes in one of three scopes (uncommitted, the branch
//! against its base, its pull request) drawn like a show diff, the comments made on it sent to the agent working there
//! as one message, and the guided walkthrough of those changes (`crate::walkthrough`).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::gui::diff_model_with;
use crate::patch::{File, git_diff, git_diff_range};
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
pub fn toplevel(folder: &str) -> Option<String> {
    git(folder, &["rev-parse", "--show-toplevel"])
}

/// How many of the agents a client lists are looked at, and how many folders git is asked about per request.
const AGENTS_AT_MOST: usize = 64;
const LOOKS_AT_MOST: usize = 16;

/// The handles of `agents` (`[{handle, cwd}]`) whose folder git puts in `repo`: a worktree nested inside it is a
/// repository of its own. Each folder's toplevel is kept a little while, since the panel asks every few seconds; a
/// client's list is read so far and git asked so often per request, at most.
pub fn working_in(repo: &str, agents: &Value) -> Vec<String> {
    type Seen = HashMap<String, (Option<String>, Instant)>;
    static SEEN: LazyLock<Mutex<Seen>> = LazyLock::new(Default::default);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    seen.retain(|_, (_, at)| at.elapsed() < Duration::from_secs(30));
    let mut looks = 0;
    agents
        .as_array()
        .into_iter()
        .flatten()
        .take(AGENTS_AT_MOST)
        .filter_map(|a| {
            let (handle, cwd) = (a["handle"].as_str()?, a["cwd"].as_str().filter(|c| c.starts_with('/'))?);
            if !seen.contains_key(cwd) {
                if looks >= LOOKS_AT_MOST || seen.len() >= AGENTS_AT_MOST * 4 {
                    return None;
                }
                looks += 1;
                seen.insert(cwd.to_string(), (toplevel(cwd), Instant::now()));
            }
            let top = seen.get(cwd).and_then(|(top, _)| top.clone());
            (top.as_deref() == Some(repo)).then(|| handle.to_string())
        })
        .collect()
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
    // The branch named, so a branch other than the one checked out has its own pull request read.
    let named: &[&str] = if branch.is_empty() { &[] } else { std::slice::from_ref(&branch) };
    let found = gh(repo, &[&["pr", "view"], named, &["--json", "number,title,url,baseRefName,headRefName,state,isDraft"]].concat()).and_then(|facts| {
        let facts: Value = serde_json::from_str(&facts).map_err(|e| e.to_string())?;
        let patch = gh(repo, &[&["pr", "diff"], named, &["--color", "never"]].concat())?;
        Ok((facts, patch))
    });
    PRS.lock().unwrap_or_else(|e| e.into_inner()).insert(key, (Instant::now(), found.clone()));
    found
}

/// The branch reviewed and the one checked out: `{branch, head, checked_out}`, `branch` the one asked for, else the
/// head.
fn heads(repo: &str, branch: Option<&str>) -> Value {
    let head = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let on = branch.filter(|b| !b.is_empty()).unwrap_or(&head).to_string();
    json!({"branch": on, "head": head, "checked_out": on == head})
}

/// The walkthrough's place for a scope: the scope itself on the branch checked out, else the scope and the branch.
fn place_of(scope: &str, about: &Value) -> String {
    if about["checked_out"] == false { format!("{scope}@{}", about["branch"].as_str().unwrap_or_default()) } else { scope.to_string() }
}

/// The scope's patch and what the header says about it, on the branch checked out or on `branch`: a branch that is
/// not checked out shows its committed changes (branch) or its pull request, never a working tree.
pub fn patch_of(repo: &str, scope: &str, branch: Option<&str>) -> Result<(String, Value), String> {
    let mut about = heads(repo, branch);
    let on = about["branch"].as_str().unwrap_or_default().to_string();
    let checked_out = about["checked_out"] == true;
    let patch = match scope {
        "branch" => {
            let base = base_of(repo).ok_or("no base branch found (origin/HEAD, main or master)")?;
            let fork = git(repo, &["merge-base", &on, &base]).ok_or_else(|| format!("{on} shares no history with {base}"))?;
            let commits = git(repo, &["rev-list", "--count", &format!("{fork}..{on}")]).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
            about["base"] = json!(base);
            about["commits"] = json!(commits);
            if checked_out { git_diff(repo, Some(&fork))? } else { git_diff_range(repo, &fork, &on)? }
        }
        "pr" => {
            let (facts, patch) = pull_request(repo, &on)?;
            about["base"] = facts["baseRefName"].clone();
            about["pr"] = facts;
            patch
        }
        _ => {
            if !checked_out {
                return Err(format!("{on} is not checked out: pick Branch or PR"));
            }
            about["base"] = json!("HEAD");
            git_diff(repo, None)?
        }
    };
    Ok((patch, about))
}

/// A scope's changes read for the TUI's review space: the files painted (None when the patch hashes to the `since`
/// asked with), where they come from, the walkthrough over them, or why they could not be read.
pub struct Read {
    pub repo: String,
    pub scope: String,
    pub hash: u64,
    pub files: Option<Vec<File>>,
    pub about: Value,
    pub walkthrough: Value,
    pub error: Option<String>,
}

/// Reads a folder's repository in one scope, as `view` does for the Herdr app.
pub fn read(folder: &str, scope: &str, since: Option<u64>) -> Read {
    let mut out = Read {
        repo: folder.to_string(),
        scope: scope.to_string(),
        hash: 0,
        files: None,
        about: Value::Null,
        walkthrough: json!({"status": "none"}),
        error: None,
    };
    let Some(repo) = toplevel(folder) else {
        out.error = Some(format!("{folder} is not in a git repository"));
        return out;
    };
    out.repo = repo;
    match patch_of(&out.repo, scope, None) {
        Ok((patch, about)) => {
            out.hash = crate::diff::hash(&patch);
            let files = || crate::patch::prepare(&patch);
            out.files = (since != Some(out.hash)).then(|| if scope == "pr" { files() } else { crate::patch::with_sizes(files(), &out.repo) });
            out.walkthrough = walkthrough::peek(&out.repo, scope, &patch).unwrap_or_else(|| taken_in(&out.repo, scope, &patch));
            out.about = about;
        }
        Err(error) => {
            out.about = heads(&out.repo, None);
            out.error = Some(error);
        }
    }
    out
}

/// A walkthrough that finished, taken in by the hub (its review view does it) and read back from it.
fn taken_in(repo: &str, scope: &str, patch: &str) -> Value {
    let since = format!("{:016x}", crate::gui::fnv(patch));
    let request = json!({"op": "view", "name": "review", "args": {"repo": repo, "scope": scope, "since": since}});
    cockpit_core::hub::request(&request, Duration::from_secs(10))
        .ok()
        .map(|answer| answer["value"]["walkthrough"].clone())
        .filter(|w| w.is_object())
        .unwrap_or_else(|| json!({"status": "running"}))
}

/// Forgets what `gh` said about a repo's pull requests, so the next read asks it again.
pub fn forget(repo: &str) {
    PRS.lock().unwrap_or_else(|e| e.into_inner()).retain(|(r, _), _| r != repo);
}

/// `{repo, scope, branch, since, load}`: the changes (`{same: true}` with the walkthrough when the patch hashes to
/// `since`), a large file's lines only once `load` names it, where they come from (`about`), and the walkthrough of
/// that scope over them; `branch` reads another branch than the one checked out. `{prompt: {scope, comments}}` is the
/// review as an agent reads it, for a new agent started on it.
pub fn view(args: &Value, _doc: &dyn Fn(&str) -> Value) -> Value {
    if args["prompt"].is_object() {
        return match message(&args["prompt"]) {
            Ok(prompt) => json!({"prompt": prompt}),
            Err(error) => json!({"error": error}),
        };
    }
    let Some(asked) = args["repo"].as_str().filter(|r| r.starts_with('/')) else { return json!({"error": "no repository: focus an agent working in one"}) };
    let Some(repo) = toplevel(asked) else { return json!({"error": format!("{asked} is not in a git repository")}) };
    let scope = args["scope"].as_str().filter(|s| matches!(*s, "branch" | "pr")).unwrap_or("uncommitted");
    let branch = args["branch"].as_str().filter(|b| !b.is_empty());
    let (patch, about) = match patch_of(&repo, scope, branch) {
        Ok(found) => found,
        Err(error) => return json!({"repo": repo, "scope": scope, "about": heads(&repo, branch), "error": error}),
    };
    let load: HashSet<String> = args["load"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_string)).collect();
    // A PR's binary files are not the working tree's: their size is left out rather than guessed.
    let sizes = (scope != "pr").then_some(repo.as_str());
    let mut out = diff_model_with(&patch, args["since"].as_str(), Some(&load), sizes);
    out["here"] = json!(working_in(&repo, &args["agents"]));
    let name = Path::new(&repo).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    out["repo"] = json!(repo);
    out["name"] = json!(name);
    out["scope"] = json!(scope);
    out["walkthrough"] = walkthrough::state(&repo, &place_of(scope, &about), &patch);
    out["about"] = about;
    out
}

/// `send [handle, {scope, repo, comments: [{path, line, old, code, text}], to}]` hands the review to the agent as one
/// message through `deliver` (a terminal's or a driven session's), the note naming `to`; `walkthrough [repo, scope,
/// model, branch?]` starts one; `stop_walkthrough [repo, scope, branch?]` stops it.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let first = args.first().and_then(Value::as_str).unwrap_or_default();
    // A walkthrough starts an agent and stops one: the person's to do.
    if matches!(action, "walkthrough" | "stop_walkthrough")
        && let Err(why) = cockpit_core::peer::person("a walkthrough is started and stopped by a person, not by an agent's tools")
    {
        return Some(Err(why));
    }
    Some(match action {
        "send" => send(first, args.get(1).unwrap_or(&Value::Null)),
        "walkthrough" => {
            let scope = args.get(1).and_then(Value::as_str).unwrap_or("uncommitted");
            let model = args.get(2).and_then(Value::as_str).unwrap_or("haiku");
            let branch = args.get(3).and_then(Value::as_str).filter(|b| !b.is_empty());
            let repo = toplevel(first).ok_or_else(|| format!("{first} is not in a git repository"));
            repo.and_then(|repo| {
                patch_of(&repo, scope, branch).and_then(|(patch, about)| walkthrough::start(&repo, &place_of(scope, &about), model, &patch, &about))
            })
        }
        "stop_walkthrough" => {
            let scope = args.get(1).and_then(Value::as_str).unwrap_or("uncommitted");
            let branch = args.get(2).and_then(Value::as_str).filter(|b| !b.is_empty());
            let repo = toplevel(first).unwrap_or_else(|| first.to_string());
            let place = place_of(scope, &heads(&repo, branch));
            walkthrough::stop(&repo, &place)
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
    // The person's review, or an agent's own tools reviewing for their own agent: never words for another agent.
    cockpit_core::peer::may_touch(&handle)?;
    cockpit_core::deliver::deliver(&handle, &line)?;
    Ok(sent_note(review))
}

/// What a send that went through says: who took the review, and how many comments it held.
fn sent_note(review: &Value) -> String {
    let n = review["comments"].as_array().map_or(0, Vec::len);
    let to = review["to"].as_str().filter(|t| !t.trim().is_empty()).unwrap_or("the agent");
    format!("review sent to {to} ({n} comment{})", if n == 1 { "" } else { "s" })
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
        assert_eq!(sent_note(&json!({"comments": [{}, {}], "to": "Daily quotas"})), "review sent to Daily quotas (2 comments)");
        assert_eq!(view(&json!({"prompt": review}), &|_| Value::Null)["prompt"], json!(line), "the prompt a new agent starts on");
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

    #[test]
    fn a_branch_that_is_not_checked_out_reads_its_committed_changes() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().to_string_lossy().into_owned();
        let run = |args: &[&str]| {
            assert!(Command::new("git").args(["-C", &repo, "-c", "user.email=a@b", "-c", "user.name=a"]).args(args).output().unwrap().status.success())
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["commit", "-q", "--allow-empty", "-m", "start"]);
        run(&["checkout", "-q", "-b", "feat"]);
        std::fs::write(root.path().join("feat.txt"), "hello\n").unwrap();
        run(&["add", "feat.txt"]);
        run(&["commit", "-q", "-m", "feat"]);
        run(&["checkout", "-q", "main"]);
        let v = view(&json!({"repo": repo, "scope": "branch", "branch": "feat"}), &|_| Value::Null);
        assert_eq!(v["files"][0]["path"], "feat.txt", "the branch's commits, read without checking it out");
        assert_eq!(v["about"], json!({"branch": "feat", "head": "main", "base": "main", "commits": 1, "checked_out": false}));
        let no_tree = view(&json!({"repo": repo, "scope": "uncommitted", "branch": "feat"}), &|_| Value::Null);
        assert_eq!(no_tree["error"], "feat is not checked out: pick Branch or PR");
        assert_eq!(no_tree["about"]["checked_out"], false, "the error still says which branch and head");
        let here = view(&json!({"repo": repo, "scope": "branch"}), &|_| Value::Null);
        assert_eq!(here["about"]["checked_out"], true, "no branch asked: the one checked out");
        assert_eq!(here["about"]["branch"], "main");
        assert_eq!(place_of("branch", &v["about"]), "branch@feat");
        assert_eq!(place_of("branch", &here["about"]), "branch");
    }

    #[test]
    fn the_agents_of_a_repo_are_the_ones_git_puts_in_it_not_a_nested_worktree() {
        let root = tempfile::tempdir().unwrap();
        let api = root.path().join("api");
        let nested = api.join("e2e-worktree");
        for dir in [&api, &nested] {
            std::fs::create_dir_all(dir).unwrap();
            assert!(Command::new("git").args(["init", "-q"]).current_dir(dir).output().unwrap().status.success());
        }
        std::fs::create_dir_all(api.join("lib")).unwrap();
        let commit = ["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "start"];
        assert!(Command::new("git").args(commit).current_dir(&api).output().unwrap().status.success());
        let repo = toplevel(&api.to_string_lossy()).unwrap();
        let path = |p: &Path| p.to_string_lossy().into_owned();
        let agents = json!([
            {"handle": "app:t41", "cwd": path(&api)},
            {"handle": "app:t45", "cwd": path(&api.join("lib"))},
            {"handle": "app:t48", "cwd": path(&nested)},
            {"handle": "app:t42", "cwd": path(root.path())},
        ]);
        assert_eq!(working_in(&repo, &agents), ["app:t41", "app:t45"]);
        let v = view(&json!({"repo": path(&api), "agents": agents}), &|_| Value::Null);
        assert_eq!(v["here"], json!(["app:t41", "app:t45"]), "the view answers with them");
    }
}
