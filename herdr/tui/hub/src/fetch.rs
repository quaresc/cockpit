//! Details on demand, kept a few minutes: a PR's unresolved review threads and changed files (gh), a branch's failing
//! tests (ci.json) and its failed CI log (GitHub Actions through gh; another CI's logs need a Claude session).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cockpit_core::files;
use serde_json::{Value, json};

use crate::config::{self, Config};
use crate::proc;
use crate::py::{list, text};

const KEPT: Duration = Duration::from_secs(180);
const GH_TIMEOUT: Duration = Duration::from_secs(25);
const LOG_LINES: usize = 200;

static CACHE: Mutex<Option<HashMap<String, (Instant, Value)>>> = Mutex::new(None);

/// The answer for this kind and arguments, from the cache while it is fresh; an error is never kept.
pub fn fetch(kind: &str, args: &Value) -> Value {
    let key = format!("{kind} {args}");
    if let Ok(mut cache) = CACHE.lock()
        && let Some((at, answer)) = cache.get_or_insert_default().get(&key)
        && at.elapsed() < KEPT
    {
        return answer.clone();
    }
    let answer = match answer(kind, args) {
        Ok(answer) => answer,
        Err(error) => return json!({"error": error}),
    };
    if let Ok(mut cache) = CACHE.lock() {
        cache.get_or_insert_default().insert(key, (Instant::now(), answer.clone()));
    }
    answer
}

fn answer(kind: &str, args: &Value) -> Result<Value, String> {
    let config = config::load();
    match kind {
        "review_threads" => review_threads(&full_repo(&config, args)?, &number(args)?),
        "pr_files" => pr_files(&full_repo(&config, args)?, &number(args)?),
        "failed_tests" => Ok(failed_tests(args)),
        "ci_log" => ci_log(&config, args),
        _ => Err(format!("unknown fetch kind {kind}")),
    }
}

/// owner/name from a short repo name and the config's GitHub organization.
fn full_repo(config: &Config, args: &Value) -> Result<String, String> {
    let repo = args["repo"].as_str().filter(|r| !r.is_empty()).ok_or("no repo given")?;
    if repo.contains('/') {
        return Ok(repo.into());
    }
    let org = config.0["github"]["org"].as_str().unwrap_or_default();
    if org.is_empty() {
        return Err("set github.org in the cockpit config".into());
    }
    Ok(format!("{org}/{repo}"))
}

fn number(args: &Value) -> Result<String, String> {
    let n = &args["number"];
    if n.is_null() { Err("no PR number given".into()) } else { Ok(text(n)) }
}

fn gh(args: &[&str]) -> Result<String, String> {
    let argv: Vec<String> = std::iter::once("gh").chain(args.iter().copied()).map(str::to_string).collect();
    let out = proc::run(&argv, GH_TIMEOUT, None);
    if out.ok() { Ok(out.stdout) } else { Err(format!("gh failed: {}", out.stderr.trim())) }
}

const THREADS: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { reviewThreads(first: 100) { nodes { isResolved isOutdated path line originalLine comments(first: 50) { nodes { author { login } body } } } } } } }";

fn review_threads(repo: &str, number: &str) -> Result<Value, String> {
    let (owner, name) = repo.split_once('/').ok_or("repo is not owner/name")?;
    let out = gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={THREADS}"),
        "-F",
        &format!("owner={owner}"),
        "-F",
        &format!("name={name}"),
        "-F",
        &format!("number={number}"),
    ])?;
    let data: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    let threads: Vec<Value> = list(&data["data"]["repository"]["pullRequest"]["reviewThreads"]["nodes"])
        .iter()
        .filter(|t| t["isResolved"].as_bool() != Some(true))
        .map(|t| {
            let comments: Vec<Value> = list(&t["comments"]["nodes"]).iter().map(|c| json!({"author": c["author"]["login"], "body": c["body"]})).collect();
            let first = comments.first().cloned().unwrap_or(Value::Null);
            let line = if t["line"].is_null() { t["originalLine"].clone() } else { t["line"].clone() };
            json!({"path": t["path"], "line": line, "author": first["author"], "body": first["body"], "outdated": t["isOutdated"], "comments": comments})
        })
        .collect();
    Ok(json!({"threads": threads}))
}

fn pr_files(repo: &str, number: &str) -> Result<Value, String> {
    let out = gh(&["pr", "view", number, "--repo", repo, "--json", "files"])?;
    let data: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    Ok(json!({"files": data["files"]}))
}

fn ci_run(args: &Value) -> Value {
    let ci = files::load(&files::cache().join("ci.json")).unwrap_or(Value::Null);
    list(&ci["runs"]).iter().find(|r| r["repo"] == args["repo"] && r["branch"] == args["branch"]).cloned().unwrap_or(Value::Null)
}

/// The failed job of the branch's CI run as ci.json has it: job, step, tests and where its logs are.
fn failed_tests(args: &Value) -> Value {
    let failed = &ci_run(args)["failed"];
    json!({"job": failed["job"], "step": failed["step"], "tests": failed.get("tests").cloned().unwrap_or(json!([])), "logs": failed["logs"]})
}

/// The last lines of the branch's failed GitHub Actions run; with another CI, a message that its logs need a session.
fn ci_log(config: &Config, args: &Value) -> Result<Value, String> {
    let ci = &config.0["ci"];
    let name = ci["name"].as_str().unwrap_or("GitHub");
    let is_github = name.eq_ignore_ascii_case("github") || ci["server"].as_str().unwrap_or_default().is_empty();
    if !is_github {
        return Ok(json!({"lines": [], "message": format!("The {name} logs need a Claude session with its connector: fix ci opens one.")}));
    }
    let repo = full_repo(config, args)?;
    let branch = args["branch"].as_str().filter(|b| !b.is_empty()).ok_or("no branch given")?;
    let runs: Value =
        serde_json::from_str(&gh(&["run", "list", "--repo", &repo, "--branch", branch, "--status", "failure", "--limit", "1", "--json", "databaseId"])?)
            .map_err(|e| e.to_string())?;
    let Some(id) = runs[0]["databaseId"].as_u64() else {
        return Ok(json!({"lines": [], "message": format!("no failed GitHub Actions run on {branch}")}));
    };
    let log = gh(&["run", "view", &id.to_string(), "--repo", &repo, "--log-failed"])?;
    let lines: Vec<&str> = log.lines().collect();
    Ok(json!({"run": id, "lines": lines[lines.len().saturating_sub(LOG_LINES)..]}))
}
