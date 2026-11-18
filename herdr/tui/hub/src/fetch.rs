//! Details on demand, kept a few minutes: a PR's unresolved review threads and changed files (gh), a branch's failing
//! tests (ci.json) and the failed CI logs of its head commit. Text from outside goes through `untrusted` on its way in.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cockpit_core::files;
use serde_json::{Value, json};

use crate::config::{self, Config};
use crate::proc;
use crate::value::{cut, list, text};

const KEPT: Duration = Duration::from_secs(180);
const GH_TIMEOUT: Duration = Duration::from_secs(25);
const LOG_LINES: usize = 200;
/// A fix's prompt carries the end of this many failing jobs' logs, this many lines each.
const LOG_JOBS: usize = 5;
const JOB_LINES: usize = 150;

static CACHE: Mutex<Option<HashMap<String, (Instant, Value)>>> = Mutex::new(None);

/// The answer for this kind and arguments, from the cache while it is fresh; an error is never kept.
pub fn fetch(kind: &str, args: &Value) -> Value {
    // A start's checks look at the world as it is now.
    if kind == "start_plan" || kind == "start_work" {
        return answer(kind, args).unwrap_or_else(|error| json!({"error": error}));
    }
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

/// Drops the kept answers of a kind, after a change made them stale.
pub fn forget(kind: &str) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.get_or_insert_default().retain(|key, _| !key.starts_with(&format!("{kind} ")));
    }
}

fn answer(kind: &str, args: &Value) -> Result<Value, String> {
    let config = config::load();
    match kind {
        "review_threads" => review_threads(&full_repo(&config, args)?, &number(args)?),
        "pr_files" => pr_files(&full_repo(&config, args)?, &number(args)?),
        "failed_tests" => Ok(failed_tests(args)),
        "ci_log" => ci_log(&config, args),
        "pr_merge" => crate::act::pr_merge(args),
        "ticket_card" => crate::act::ticket_card(args),
        "start_plan" => Ok(crate::act::start_plan(&config, args.get("target").unwrap_or(args))),
        "start_work" => Ok(crate::act::start_work(args)),
        _ => Err(format!("unknown fetch kind {kind}")),
    }
}

/// owner/name: as given, else the owner of the PR's own URL, else the config's GitHub organization.
fn full_repo(config: &Config, args: &Value) -> Result<String, String> {
    let repo = args["repo"].as_str().filter(|r| !r.is_empty()).ok_or("no repo given")?;
    if repo.contains('/') {
        return Ok(repo.into());
    }
    let owner = args["url"].as_str().and_then(|u| u.strip_prefix("https://github.com/")).and_then(|path| path.split('/').next()).filter(|o| !o.is_empty());
    if let Some(owner) = owner {
        return Ok(format!("{owner}/{repo}"));
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

const THREADS: &str = "query($owner: String!, $name: String!, $number: Int!, $after: String) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { \
  reviewThreads(first: 100, after: $after) { pageInfo { hasNextPage endCursor } nodes { isResolved isOutdated path line originalLine \
  comments(first: 50) { totalCount nodes { author { __typename login } authorAssociation body } } } } \
  reviews(last: 30, states: [CHANGES_REQUESTED, COMMENTED]) { nodes { author { __typename login } authorAssociation state body } } } } }";
/// The most pages of 100 review threads read.
const THREAD_PAGES: usize = 5;

/// Who wrote a comment, its words made safe, whether it comes from a bot and whether its author collaborates on the repo.
fn comment(c: &Value) -> Value {
    let login = c["author"]["login"].as_str().unwrap_or_default();
    let bot = c["author"]["__typename"] == "Bot" || login.ends_with("[bot]");
    let member = matches!(c["authorAssociation"].as_str(), Some("OWNER" | "MEMBER" | "COLLABORATOR"));
    json!({"author": untrusted(login), "body": untrusted(c["body"].as_str().unwrap_or_default()), "bot": bot, "member": member})
}

/// The PR's unresolved review threads, page by page (`cut` when more were left), and the bodies of its reviews that
/// asked for changes or commented.
fn review_threads(repo: &str, number: &str) -> Result<Value, String> {
    let (owner, name) = repo.split_once('/').ok_or("repo is not owner/name")?;
    let (mut threads, mut reviews, mut after, mut cut) = (Vec::new(), Vec::new(), None::<String>, false);
    for page in 0..THREAD_PAGES {
        let mut args = vec!["api".to_string(), "graphql".into(), "-f".into(), format!("query={THREADS}")];
        for field in [format!("owner={owner}"), format!("name={name}"), format!("number={number}")] {
            args.extend(["-F".to_string(), field]);
        }
        if let Some(cursor) = &after {
            args.extend(["-f".to_string(), format!("after={cursor}")]);
        }
        let out = gh(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        let data: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
        let pr = &data["data"]["repository"]["pullRequest"];
        if page == 0 {
            reviews = list(&pr["reviews"]["nodes"])
                .iter()
                .filter(|r| r["body"].as_str().is_some_and(|b| !b.trim().is_empty()))
                .map(|r| json!({"state": r["state"], "comment": comment(r)}))
                .collect();
        }
        for t in list(&pr["reviewThreads"]["nodes"]).iter().filter(|t| t["isResolved"].as_bool() != Some(true)) {
            let comments: Vec<Value> = list(&t["comments"]["nodes"]).iter().map(comment).collect();
            let more = t["comments"]["totalCount"].as_u64().unwrap_or(0).saturating_sub(comments.len() as u64);
            let first = comments.first().cloned().unwrap_or(Value::Null);
            let line = if t["line"].is_null() { t["originalLine"].clone() } else { t["line"].clone() };
            threads.push(json!({"path": untrusted(t["path"].as_str().unwrap_or_default()), "line": line, "author": first["author"], "body": first["body"],
                "bot": first["bot"], "outdated": t["isOutdated"], "comments": comments, "more": more}));
        }
        let info = &pr["reviewThreads"]["pageInfo"];
        after = info["endCursor"].as_str().map(str::to_string);
        if info["hasNextPage"] != true || after.is_none() {
            break;
        }
        cut = page + 1 == THREAD_PAGES;
    }
    Ok(json!({"threads": threads, "reviews": reviews, "cut": cut}))
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

/// The failed logs of the branch's head commit: every failed GitHub Actions run of it through gh, another CI's failing
/// jobs through the hub's own server; else a message that its logs need a session.
fn ci_log(config: &Config, args: &Value) -> Result<Value, String> {
    let ci = &config.0["ci"];
    let name = ci["name"].as_str().unwrap_or("GitHub");
    let is_github = name.eq_ignore_ascii_case("github") || ci["server"].as_str().unwrap_or_default().is_empty();
    if !is_github {
        let raw = files::load(&files::config_dir().join("config.json")).unwrap_or(Value::Null);
        let run = ci_run(args);
        let failing: Vec<Value> = match run["failedJobs"].as_array().filter(|j| !j.is_empty()) {
            Some(jobs) => jobs.clone(),
            None => vec![run["failed"].clone()],
        };
        let failing: Vec<(String, String)> = failing
            .iter()
            .filter_map(|j| Some((j["jobId"].as_str().filter(|id| !id.is_empty())?.to_string(), j["job"].as_str().unwrap_or("CI").to_string())))
            .take(LOG_JOBS)
            .collect();
        if let Some(server) = crate::feed::mcp::server(&raw, &config.0, "ci").filter(|_| !failing.is_empty()) {
            let mapping = cockpit_connectors::ci::mapping(&raw["ci"]);
            let (mut jobs, mut lines) = (Vec::new(), Vec::new());
            for (id, job) in &failing {
                let log = crate::feed::mcp::with(&server, async |c| cockpit_connectors::call_mapped_text(c, &mapping, "logs", &json!({"job": id})).await)?;
                lines.extend(log.lines().map(untrusted));
                jobs.extend(jobs_of(&log, job, false));
            }
            return Ok(json!({"lines": lines[lines.len().saturating_sub(LOG_LINES)..], "jobs": jobs}));
        }
        return Ok(json!({"lines": [], "message": format!("The {name} logs need an agent session with its connector: fix ci opens one.")}));
    }
    let repo = full_repo(config, args)?;
    let branch = args["branch"].as_str().filter(|b| !b.is_empty()).ok_or("no branch given")?;
    let sha = args["sha"].as_str().filter(|s| !s.is_empty());
    let mut list_args = vec!["run", "list", "--repo", &repo, "--limit", "30", "--json", "databaseId,conclusion,workflowName,headSha"];
    match sha {
        Some(sha) => list_args.extend(["--commit", sha]),
        None => list_args.extend(["--branch", branch]),
    }
    let runs: Value = serde_json::from_str(&gh(&list_args)?).map_err(|e| e.to_string())?;
    // Without the PR's head, the newest run's commit stands for it.
    let head = sha.map(str::to_string).or_else(|| runs[0]["headSha"].as_str().map(str::to_string));
    let failed: Vec<&Value> = list(&runs)
        .iter()
        .filter(|r| head.is_none() || r["headSha"].as_str().is_none() || r["headSha"].as_str() == head.as_deref())
        .filter(|r| matches!(r["conclusion"].as_str(), Some("failure" | "timed_out" | "startup_failure")))
        .collect();
    let Some(first) = failed.first().and_then(|r| r["databaseId"].as_u64()) else {
        let at = sha.map(|s| format!("commit {}", cut(s, 7))).unwrap_or_else(|| branch.to_string());
        return Ok(json!({"lines": [], "message": format!("no failed GitHub Actions run on {at}")}));
    };
    let (mut jobs, mut lines, mut ids) = (Vec::new(), Vec::new(), Vec::new());
    for run in failed.iter().take(LOG_JOBS) {
        let Some(id) = run["databaseId"].as_u64() else { continue };
        ids.push(id);
        let workflow = untrusted(run["workflowName"].as_str().unwrap_or("CI"));
        if run["conclusion"] == "startup_failure" {
            jobs.push(json!({"job": workflow, "step": "", "lines": ["(this run did not start: startup_failure)"]}));
            continue;
        }
        let log = gh(&["run", "view", &id.to_string(), "--repo", &repo, "--log-failed"])?;
        lines.extend(log.lines().map(untrusted));
        jobs.extend(jobs_of(&log, &workflow, true));
    }
    jobs.truncate(LOG_JOBS);
    Ok(json!({"run": first, "runs": ids, "sha": head, "lines": lines[lines.len().saturating_sub(LOG_LINES)..], "jobs": jobs}))
}

/// Text from outside (a log, a review, a test's name) made safe to show and to hand an agent: colours, terminal commands,
/// control, line-separator and bidi characters out, a line rewritten by carriage returns kept as it ended, tabs as
/// spaces, and unusual spaces shown as `\u{a0}` escapes.
pub fn untrusted(text: &str) -> String {
    static ESCAPES: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?|\x1b\[[0-?]*[ -/]*[@-~]|\x1b[@-Z\\-_]?").expect("a valid pattern"));
    let text = ESCAPES.replace_all(text, "");
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let line = line.rsplit('\r').next().unwrap_or(line);
            let mut out = String::with_capacity(line.len());
            for c in line.chars() {
                match c {
                    '\t' => out.push_str("    "),
                    // Line and paragraph separators, and the marks that reorder text on screen.
                    '\u{2028}' | '\u{2029}' | '\u{200e}' | '\u{200f}' | '\u{061c}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
                    // Spaces that look like a space, or like nothing.
                    '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200d}' | '\u{202f}' | '\u{205f}' | '\u{2060}' | '\u{3000}' | '\u{feff}' => {
                        out.push_str(&format!("\\u{{{:x}}}", c as u32));
                    }
                    c if c.is_control() => {}
                    c => out.push(c),
                }
            }
            out
        })
        .collect();
    lines.join("\n")
}

/// A failed log split by job: with `prefixed`, as `gh run view --log-failed` writes its lines (`job<TAB>step<TAB>time
/// text`), else all of it one job named `fallback`. The first few jobs, each with its last step and the end of its
/// lines, made safe (`untrusted`).
pub fn jobs_of(log: &str, fallback: &str, prefixed: bool) -> Vec<Value> {
    static STAMP: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T[\d:.]+Z ?").expect("a valid pattern"));
    let mut jobs: Vec<(String, String, Vec<String>)> = Vec::new();
    for line in log.lines() {
        let mut parts = line.splitn(3, '\t');
        let (job, step, text) = match (prefixed, parts.next(), parts.next(), parts.next()) {
            (true, Some(job), Some(step), Some(text)) => (untrusted(job), untrusted(step), STAMP.replace(text, "").into_owned()),
            _ => (fallback.to_string(), String::new(), line.to_string()),
        };
        let text = untrusted(&text);
        match jobs.iter().position(|(j, _, _)| *j == job) {
            Some(at) => {
                jobs[at].1 = step;
                jobs[at].2.push(text);
            }
            None if jobs.len() < LOG_JOBS => jobs.push((job, step, vec![text])),
            None => {}
        }
    }
    jobs.into_iter().map(|(job, step, lines)| json!({"job": job, "step": step, "lines": lines[lines.len().saturating_sub(JOB_LINES)..]})).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_log_splits_by_job_and_keeps_each_ones_end() {
        let log = "test\tRun mix test\t2026-11-12T10:01:02.1234567Z \u{1b}[31m  1) test daily quota resets at midnight (QuotasTest)\u{1b}[0m\n\
                   test\tRun mix test\t2026-11-12T10:01:03.0000000Z      ** (MatchError) no match of right hand side value: {:error, :over_quota}\n\
                   lint\tRun mix credo\t2026-11-12T10:00:40.0000000Z lib/quotas/daily.ex:12 Nested modules could be aliased\n";
        let jobs = jobs_of(log, "CI", true);
        assert_eq!(jobs.len(), 2);
        assert_eq!((jobs[0]["job"].as_str(), jobs[0]["step"].as_str()), (Some("test"), Some("Run mix test")));
        assert_eq!(jobs[0]["lines"][0], "  1) test daily quota resets at midnight (QuotasTest)", "time and colour taken out");
        assert_eq!(jobs[1]["lines"][0], "lib/quotas/daily.ex:12 Nested modules could be aliased");
        let long: String = (0..400).map(|n| format!("e2e\tRun playwright\tline {n}\n")).collect();
        let e2e = &jobs_of(&long, "CI", true)[0];
        assert_eq!((e2e["lines"].as_array().unwrap().len(), e2e["lines"][0].as_str()), (JOB_LINES, Some("line 250")));
        let many: String = (0..9).map(|n| format!("job{n}\tstep\tfailed\n")).collect();
        assert_eq!(jobs_of(&many, "CI", true).len(), LOG_JOBS, "the first few jobs only");
        let plain = jobs_of("Error: 2 tests failed\n", "build", true);
        assert_eq!((plain[0]["job"].as_str(), plain[0]["lines"][0].as_str()), (Some("build"), Some("Error: 2 tests failed")));
        let circle = jobs_of("deploy\tstaging\tfailed with a tab\n", "build", false);
        assert_eq!(
            (circle[0]["job"].as_str(), circle[0]["lines"][0].as_str()),
            (Some("build"), Some("deploy    staging    failed with a tab")),
            "another CI's tabs are text"
        );
    }

    #[test]
    fn text_from_outside_loses_colours_terminal_commands_and_overwritten_progress() {
        assert_eq!(untrusted("\u{1b}[31mred\u{1b}[0m and \u{1b}]0;title\u{7}plain"), "red and plain");
        assert_eq!(untrusted("10%\r50%\r100% done\r\nnext\ttab"), "100% done\nnext    tab", "a CRLF ends a line, a bare CR rewrites it");
        assert_eq!(untrusted("bell\u{7} nul\u{0} esc\u{1b}"), "bell nul esc");
        assert_eq!(untrusted("```suggestion\nlet x = 1;\n```"), "```suggestion\nlet x = 1;\n```", "newlines stay");
        assert_eq!(untrusted("a\u{2028}b\u{2029}c \u{202e}evil\u{202c} \u{2066}x\u{2069}"), "abc evil x", "separators and bidi marks out");
        assert_eq!(untrusted("no\u{a0}break zero\u{200b}width"), "no\\u{a0}break zero\\u{200b}width", "odd spaces made visible");
    }
}
