//! Landing PRs and starting work: whether GitHub would merge a PR now and what the merge menu offers (after
//! mrolafsson/herdr-github's readiness), merge, auto-merge, draft and the merge queue through gh; a ticket or a PR
//! handed to start work; and the worktrees of merged branches removed.
//! The readiness line and merge menu are ported from https://github.com/mrolafsson/herdr-github (MIT, Copyright (c) 2026
//! Hjortur Stefan Olafsson); see THIRD_PARTY_NOTICES.md.

use std::path::Path;
use std::time::Duration;

use cockpit_core::{files, frontend};
use serde_json::{Value, json};

use super::{Ctx, arg, inbox, opt};
use crate::config::{self, Config};
use crate::herdr;
use crate::proc;
use crate::value::{Res, list, text};

const GH_TIMEOUT: Duration = Duration::from_secs(40);

fn gh(args: &[&str]) -> Res<String> {
    let argv: Vec<String> = std::iter::once("gh").chain(args.iter().copied()).map(str::to_string).collect();
    let out = proc::run(&argv, GH_TIMEOUT, None);
    if out.ok() { Ok(out.stdout) } else { Err(format!("gh: {}", out.stderr.trim().lines().next().unwrap_or("failed"))) }
}

/// git in `dir` as every cleanup runs it: never asking for a password, a minute at most.
pub(super) fn git(dir: &Path, args: &[&str]) -> proc::Out {
    match cockpit_core::cleanup::git_out(dir, args, Duration::from_secs(60), true) {
        Some(out) => proc::Out { started: true, code: if out.ok { 0 } else { 1 }, stdout: out.stdout, stderr: out.stderr },
        None => proc::Out { started: false, code: 1, stdout: String::new(), stderr: format!("git {} ran out of time", args.first().unwrap_or(&"")) },
    }
}

/// The inbox's actions called the short way, as other views do: a PR by its URL, `{pr, method}` for a merge, `fix_ci`
/// with the PR alone (its CI run and "here" added).
pub fn shorthand(action: &str, mut args: Vec<Value>) -> Vec<Value> {
    if let Some(Value::Object(first)) = args.first().cloned()
        && let Some(pr) = first.get("pr")
    {
        args = vec![pr.clone()];
        args.extend(["method", "place"].iter().filter_map(|k| first.get(*k).cloned()));
    }
    if let Some(url) = args.first().and_then(Value::as_str).filter(|u| u.contains("/pull/")).map(str::to_string) {
        let inbox = config::read_inbox();
        let known = ["mine", "direct", "team"].iter().flat_map(|f| list(&inbox[*f]).iter()).find(|pr| pr["url"] == url.as_str()).cloned();
        args[0] = known.unwrap_or_else(|| json!({"url": url}));
    }
    if action == "fix_ci" && (1..3).contains(&args.len()) {
        let ci = files::load(&files::cache().join("ci.json")).unwrap_or(Value::Null);
        let pr = args[0].clone();
        let run = list(&ci["runs"]).iter().find(|r| r["repo"] == pr["repo"] && r["branch"] == pr["branch"]).cloned().unwrap_or(Value::Null);
        let place = args.get(1).cloned().unwrap_or(json!("here"));
        args = vec![pr, run, place];
    }
    args
}

/// owner, name and number of a PR, from its URL, else from its repo and the config's GitHub organization.
pub fn coords(cfg: &Config, pr: &Value) -> Res<(String, String, String)> {
    let url = pr["url"].as_str().or(pr.as_str()).unwrap_or_default();
    let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
    if let Some(at) = parts.iter().position(|p| *p == "pull")
        && at >= 2
        && let Some(n) = parts.get(at + 1)
    {
        return Ok((parts[at - 2].to_string(), parts[at - 1].to_string(), n.to_string()));
    }
    let repo = pr["repo"].as_str().filter(|r| !r.is_empty()).ok_or("the PR names no repo")?;
    let number = text(&pr["number"]);
    if let Some((owner, name)) = repo.split_once('/') {
        return Ok((owner.into(), name.into(), number));
    }
    let org = cfg.0["github"]["org"].as_str().unwrap_or_default();
    if org.is_empty() {
        return Err("set github.org in the cockpit config".into());
    }
    Ok((org.into(), repo.into(), number))
}

const DETAIL: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) {
  nameWithOwner squashMergeAllowed mergeCommitAllowed rebaseMergeAllowed autoMergeAllowed viewerPermission viewerDefaultMergeMethod
  pullRequest(number: $number) { id number title url state isDraft mergeable mergeStateStatus reviewDecision headRefOid headRefName
    baseRefName isCrossRepository headRepositoryOwner { login } body additions deletions changedFiles isMergeQueueEnabled
    viewerCanMergeAsAdmin author { login } mergeQueueEntry { position state estimatedTimeToMerge } autoMergeRequest { mergeMethod }
    labels(first: 12) { nodes { name color } }
    latestReviews(first: 20) { nodes { author { login } state } }
    reviewRequests(first: 20) { nodes { requestedReviewer { ... on User { login } ... on Team { name } } } }
    reviewThreads(first: 60) { nodes { isResolved } }
    commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes { __typename
      ... on CheckRun { name status conclusion isRequired(pullRequestNumber: $number) }
      ... on StatusContext { context state isRequired(pullRequestNumber: $number) } } } } } } } } } }";

/// The PR and its repo's merge settings as GitHub has them now: {repo: {...}, pr: {...}}.
pub fn detail(cfg: &Config, pr: &Value) -> Res<Value> {
    let (owner, name, number) = coords(cfg, pr)?;
    let out = gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={DETAIL}"),
        "-F",
        &format!("owner={owner}"),
        "-F",
        &format!("name={name}"),
        "-F",
        &format!("number={number}"),
    ])?;
    let data: Value = serde_json::from_str(&out).map_err(|e| format!("gh answered no JSON: {e}"))?;
    let repo = &data["data"]["repository"];
    if repo["pullRequest"].is_null() {
        return Err(data["errors"][0]["message"].as_str().unwrap_or("GitHub has no such PR").to_string());
    }
    Ok(json!({"repo": repo, "pr": repo["pullRequest"]}))
}

fn checks(d: &Value) -> Vec<(String, &'static str, bool)> {
    list(&d["pr"]["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"])
        .iter()
        .map(|c| {
            let name = c["name"].as_str().or(c["context"].as_str()).unwrap_or("?").to_string();
            let state = c["conclusion"].as_str().or(c["state"].as_str()).unwrap_or_default();
            let outcome = match state {
                "SUCCESS" => "pass",
                "SKIPPED" | "NEUTRAL" => "skip",
                "FAILURE" | "ERROR" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" | "CANCELLED" | "STALE" => "fail",
                _ => "pending",
            };
            (name, outcome, c["isRequired"] == true)
        })
        .collect()
}

fn can_merge(d: &Value) -> bool {
    d["pr"]["viewerCanMergeAsAdmin"] == true || matches!(d["repo"]["viewerPermission"].as_str(), Some("ADMIN" | "MAINTAIN" | "WRITE"))
}

fn mergeable_now(d: &Value) -> bool {
    matches!(d["pr"]["mergeStateStatus"].as_str(), Some("CLEAN" | "UNSTABLE" | "HAS_HOOKS")) && d["pr"]["mergeable"] != "CONFLICTING"
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Whether the PR can merge and if not why, in one line: (ready | blocked | waiting | draft | queued | merged | closed,
/// text, tone bad | warn | good | dim).
pub fn readiness(d: &Value) -> (&'static str, String, &'static str) {
    let pr = &d["pr"];
    let base = pr["baseRefName"].as_str().unwrap_or("its base");
    match pr["state"].as_str() {
        Some("MERGED") => return ("merged", "Merged".into(), "good"),
        Some("CLOSED") => return ("closed", "Closed without merging".into(), "bad"),
        _ => {}
    }
    if pr["isDraft"] == true {
        return ("draft", "Draft: mark it ready for review before it can merge".into(), "dim");
    }
    if pr["mergeable"] == "CONFLICTING" || pr["mergeStateStatus"] == "DIRTY" {
        return ("blocked", format!("Conflicts with {base}: resolve them before merging"), "bad");
    }
    if let Some(q) = pr["mergeQueueEntry"].as_object() {
        let what = match q.get("state").and_then(Value::as_str).unwrap_or_default() {
            "QUEUED" => "waiting its turn",
            "AWAITING_CHECKS" => "its checks are running",
            "MERGEABLE" => "about to merge",
            "UNMERGEABLE" => "it can't merge and will be taken out",
            "LOCKED" => "merging now",
            _ => "queued",
        };
        let position = q.get("position").and_then(Value::as_u64).unwrap_or(0);
        return ("queued", format!("In the merge queue, #{position} in line: {what}"), if what.starts_with("it can't") { "bad" } else { "warn" });
    }
    let all = checks(d);
    let failing = all.iter().filter(|(_, o, r)| *r && *o == "fail").count();
    let pending = all.iter().filter(|(_, o, r)| *r && *o == "pending").count();
    let review = pr["reviewDecision"].as_str().unwrap_or_default();
    let (state, mut line, tone) = match pr["mergeStateStatus"].as_str().unwrap_or_default() {
        "CLEAN" => ("ready", "Ready to merge".to_string(), "good"),
        "HAS_HOOKS" => ("ready", "Ready to merge (the server runs pre-receive hooks)".to_string(), "good"),
        "UNSTABLE" => ("ready", "Can merge, but checks that aren't required are failing".to_string(), "warn"),
        "BEHIND" => ("blocked", format!("Behind {base}: bring it up to date first"), "warn"),
        "BLOCKED" if failing > 0 => ("blocked", format!("Blocked: {} failing", plural(failing, "required check", "required checks")), "bad"),
        "BLOCKED" if review == "CHANGES_REQUESTED" => ("blocked", "Blocked: changes requested".to_string(), "bad"),
        "BLOCKED" if review == "REVIEW_REQUIRED" => ("blocked", "Blocked: needs an approving review".to_string(), "bad"),
        "BLOCKED" if pending > 0 => ("waiting", format!("Waiting on {}", plural(pending, "required check", "required checks")), "warn"),
        "BLOCKED" => ("blocked", "Blocked by the branch's rules".to_string(), "bad"),
        _ => ("waiting", "GitHub is still working out whether it can merge".to_string(), "dim"),
    };
    if pr["isMergeQueueEnabled"] == true && state == "ready" {
        line += ": through the merge queue";
    }
    if !can_merge(d) {
        line += " · you can't merge in this repo";
    }
    (state, line, tone)
}

const METHODS: [(&str, &str, &str); 3] =
    [("SQUASH", "squash", "Squash and merge"), ("MERGE", "merge", "Create a merge commit"), ("REBASE", "rebase", "Rebase and merge")];

fn allowed(d: &Value) -> Vec<(&'static str, &'static str, &'static str)> {
    let repo = &d["repo"];
    let preferred = repo["viewerDefaultMergeMethod"].as_str().unwrap_or("SQUASH");
    let mut out: Vec<_> = METHODS
        .into_iter()
        .filter(|(m, _, _)| repo[format!("{}Allowed", if *m == "MERGE" { "mergeCommit".to_string() } else { m.to_lowercase() + "Merge" })] != false)
        .collect();
    out.sort_by_key(|(m, _, _)| *m != preferred);
    out
}

/// What the merge menu offers: [{do: merge | auto | auto-off | enqueue | dequeue, method, label}], else why not.
pub fn choices(d: &Value) -> (Vec<Value>, String) {
    let pr = &d["pr"];
    let n = text(&pr["number"]);
    if pr["state"] != "OPEN" {
        return (Vec::new(), format!("#{n} is already {}.", pr["state"].as_str().unwrap_or("closed").to_lowercase()));
    }
    if pr["isDraft"] == true {
        return (Vec::new(), "It's a draft: mark it ready for review first.".into());
    }
    if !can_merge(d) {
        return (Vec::new(), format!("You can't merge in {}.", d["repo"]["nameWithOwner"].as_str().unwrap_or("this repo")));
    }
    if pr["isMergeQueueEnabled"] == true {
        if pr["mergeQueueEntry"].is_object() {
            return (vec![json!({"do": "dequeue", "label": "Take it out of the merge queue"})], String::new());
        }
        if !mergeable_now(d) && readiness(d).0 != "waiting" {
            return (Vec::new(), format!("It can't join the merge queue yet: {}", readiness(d).1));
        }
        return (vec![json!({"do": "enqueue", "label": "Add it to the merge queue"})], String::new());
    }
    let mut out = Vec::new();
    if pr["autoMergeRequest"].is_object() {
        out.push(json!({"do": "auto-off", "label": "Turn off auto-merge"}));
    }
    let now = mergeable_now(d);
    for (_, method, label) in allowed(d) {
        if now {
            out.push(json!({"do": "merge", "method": method, "label": label}));
        } else if d["repo"]["autoMergeAllowed"] == true && !pr["autoMergeRequest"].is_object() && pr["mergeable"] != "CONFLICTING" {
            out.push(json!({"do": "auto", "method": method, "label": format!("Auto-merge when ready: {}", label.to_lowercase())}));
        }
    }
    let why = if out.is_empty() { readiness(d).1 } else { String::new() };
    (out, why)
}

/// The `pr_merge` details: readiness, the merge menu, and the hover card's facts.
pub fn pr_merge(args: &Value) -> Res<Value> {
    let d = detail(&config::load(), args)?;
    let pr = &d["pr"];
    let (state, line, tone) = readiness(&d);
    let (menu, why) = choices(&d);
    let mut reviewers: Vec<Value> = list(&pr["latestReviews"]["nodes"]).iter().map(|r| json!({"login": r["author"]["login"], "state": r["state"]})).collect();
    for r in list(&pr["reviewRequests"]["nodes"]) {
        let who = r["requestedReviewer"]["login"].as_str().or(r["requestedReviewer"]["name"].as_str()).unwrap_or_default();
        if !who.is_empty() {
            reviewers.push(json!({"login": who, "state": "PENDING"}));
        }
    }
    let all = checks(&d);
    let count = |o: &str| all.iter().filter(|(_, x, _)| *x == o).count();
    let failing: Vec<&String> = all.iter().filter(|(_, o, _)| *o == "fail").map(|(n, _, _)| n).collect();
    let threads = list(&pr["reviewThreads"]["nodes"]).iter().filter(|t| t["isResolved"] != true).count();
    let body: String = pr["body"].as_str().unwrap_or_default().chars().take(1200).collect();
    Ok(json!({
        "ready": {"state": state, "text": line, "tone": tone},
        "choices": menu, "why": why,
        "head": pr["headRefOid"], "base": pr["baseRefName"], "branch": pr["headRefName"], "auto": pr["autoMergeRequest"]["mergeMethod"],
        "card": {
            "title": pr["title"], "author": pr["author"]["login"], "body": body, "labels": pr["labels"]["nodes"],
            "additions": pr["additions"], "deletions": pr["deletions"], "files": pr["changedFiles"], "reviewers": reviewers, "threads": threads,
            "checks": {"pass": count("pass"), "fail": count("fail"), "pending": count("pending"), "skip": count("skip"), "failing": failing},
        },
    }))
}

// ---------------------------------------------------------------------------------------------------------------------
// Merge, auto-merge, draft, the merge queue

fn after_change(ctx: &mut Ctx, note: String) {
    crate::fetch::forget("pr_merge");
    let _ = std::fs::write(files::cache().join("refresh-request"), crate::value::now().to_string());
    ctx.note(note);
}

fn method_flag(method: &str) -> Res<&'static str> {
    match method.to_lowercase().as_str() {
        "squash" => Ok("--squash"),
        "merge" => Ok("--merge"),
        "rebase" => Ok("--rebase"),
        other => Err(format!("no merge method {other:?}: squash, merge or rebase")),
    }
}

fn mutate(query: &str, vars: &[(&str, &str)]) -> Res {
    let mut args: Vec<String> = vec!["api".into(), "graphql".into(), "-f".into(), format!("query={query}")];
    for (k, v) in vars {
        args.extend(["-f".to_string(), format!("{k}={v}")]);
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = gh(&refs)?;
    let answer: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
    match answer["errors"][0]["message"].as_str() {
        Some(error) => Err(error.to_string()),
        None => Ok(()),
    }
}

/// `merge [pr, method]`: merges now, pinned to the head commit GitHub reported a moment ago; where the base branch has
/// a merge queue, the PR joins it.
/// A merge, ready or draft through the code host's MCP server, when the hub reaches one (`code` in the config); None
/// when the PRs are GitHub's.
fn by_code_server(ctx: &mut Ctx, verb: &str, pr: &Value, done: &str) -> Option<Res> {
    let raw = files::load(&files::config_dir().join("config.json")).unwrap_or(Value::Null);
    let server = crate::feed::mcp::server(&raw, &ctx.cfg.0, "code")?;
    use cockpit_connectors::{code, tools::Mapped};
    let mapping = code::mapping(&raw["code"]);
    let outcome = crate::feed::mcp::with(&server, async |c| code::act(&Mapped { connection: c, section: &mapping }, verb, pr).await);
    let repo = pr["repo"].as_str().unwrap_or_default().to_string();
    Some(outcome.map(|()| after_change(ctx, format!("{repo}#{} {done}", text(&pr["number"])))))
}

pub fn merge(ctx: &mut Ctx, args: &[Value]) -> Res {
    let pr = arg(args, 0)?;
    if let Some(done) = by_code_server(ctx, "merge", pr, "merged") {
        return done;
    }
    let d = detail(&ctx.cfg, pr)?;
    let (owner, name, n) = coords(&ctx.cfg, pr)?;
    let head = d["pr"]["headRefOid"].as_str().unwrap_or_default().to_string();
    if d["pr"]["state"] != "OPEN" {
        return Err(format!("#{n} is {}", d["pr"]["state"].as_str().unwrap_or("closed").to_lowercase()));
    }
    if d["pr"]["isMergeQueueEnabled"] == true {
        let id = d["pr"]["id"].as_str().unwrap_or_default();
        mutate(
            "mutation($id: ID!, $oid: GitObjectID) { enqueuePullRequest(input: {pullRequestId: $id, expectedHeadOid: $oid}) { clientMutationId } }",
            &[("id", id), ("oid", &head)],
        )?;
        after_change(ctx, format!("{name}#{n} joined the merge queue"));
        return Ok(());
    }
    let method = opt(args, 1).as_str().map(str::to_string).unwrap_or_else(|| allowed(&d).first().map(|m| m.1.to_string()).unwrap_or("squash".into()));
    let repo = format!("{owner}/{name}");
    gh(&["pr", "merge", &n, "--repo", &repo, method_flag(&method)?, "--match-head-commit", &head])?;
    // Said where the cleanup reads it, so its worktree can go before the feed hears of the merge.
    let merged = json!({"repo": name, "slug": repo, "url": d["pr"]["url"], "number": d["pr"]["number"], "branch": d["pr"]["headRefName"],
                        "headSha": head, "mergedAt": chrono::Utc::now().to_rfc3339(), "mergeSha": ""});
    crate::store::global().update("inbox", |doc| {
        let mut doc = doc.clone();
        if doc.is_object() && !list(&doc["merged"]).iter().any(|m| m["url"] == merged["url"]) {
            let mut all = list(&doc["merged"]).to_vec();
            all.insert(0, merged.clone());
            doc["merged"] = Value::from(all);
        }
        doc
    });
    after_change(ctx, format!("merged {name}#{n} ({method}): its worktree is in To clean up"));
    Ok(())
}

/// `automerge [pr, method | "off"]`.
pub fn automerge(ctx: &mut Ctx, args: &[Value]) -> Res {
    let pr = arg(args, 0)?;
    let (owner, name, n) = coords(&ctx.cfg, pr)?;
    let repo = format!("{owner}/{name}");
    let method = opt(args, 1).as_str().unwrap_or("squash").to_string();
    if method == "off" {
        gh(&["pr", "merge", &n, "--repo", &repo, "--disable-auto"])?;
        after_change(ctx, format!("auto-merge is off for {name}#{n}"));
        return Ok(());
    }
    let d = detail(&ctx.cfg, pr)?;
    let head = d["pr"]["headRefOid"].as_str().unwrap_or_default().to_string();
    gh(&["pr", "merge", &n, "--repo", &repo, "--auto", method_flag(&method)?, "--match-head-commit", &head])?;
    after_change(ctx, format!("{name}#{n} merges by itself ({method}) once checks and reviews allow"));
    Ok(())
}

fn set_draft(ctx: &mut Ctx, args: &[Value], draft: bool) -> Res {
    let (owner, name, n) = coords(&ctx.cfg, arg(args, 0)?)?;
    let repo = format!("{owner}/{name}");
    let mut argv = vec!["pr", "ready", n.as_str(), "--repo", repo.as_str()];
    if draft {
        argv.push("--undo");
    }
    gh(&argv)?;
    after_change(ctx, if draft { format!("{name}#{n} is a draft again") } else { format!("{name}#{n} is ready for review") });
    Ok(())
}

pub fn ready(ctx: &mut Ctx, args: &[Value]) -> Res {
    if let Some(done) = by_code_server(ctx, "ready", arg(args, 0)?, "is ready for review") {
        return done;
    }
    set_draft(ctx, args, false)
}

pub fn draft(ctx: &mut Ctx, args: &[Value]) -> Res {
    if let Some(done) = by_code_server(ctx, "draft", arg(args, 0)?, "is a draft again") {
        return done;
    }
    set_draft(ctx, args, true)
}

pub fn dequeue(ctx: &mut Ctx, args: &[Value]) -> Res {
    let pr = arg(args, 0)?;
    let d = detail(&ctx.cfg, pr)?;
    let id = d["pr"]["id"].as_str().unwrap_or_default();
    mutate("mutation($id: ID!) { dequeuePullRequest(input: {id: $id}) { clientMutationId } }", &[("id", id)])?;
    after_change(ctx, format!("#{} is out of the merge queue", text(&d["pr"]["number"])));
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// From a ticket or a PR to a worktree with an agent

/// Whether a branch names the ticket as a whole token: eng-4-login does, eng-42-login does not.
pub(super) fn names_ticket(branch: &str, ticket: &str) -> bool {
    let (branch, key) = (branch.to_lowercase(), ticket.to_lowercase());
    !key.is_empty()
        && branch.match_indices(&key).any(|(at, _)| {
            let before = branch[..at].chars().next_back();
            let after = branch[at + key.len()..].chars().next();
            !before.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
        })
}

/// A ticket or a PR as start work's source: the PR's URL, else the ticket's key.
fn source_of(target: &Value) -> String {
    match target["url"].as_str().filter(|u| u.contains("/pull/")) {
        Some(url) => url.to_string(),
        None => target["id"].as_str().or(target.as_str()).unwrap_or_default().to_uppercase(),
    }
}

/// What `start` does for a ticket or a PR, or why it can't: {ok, steps, blockers}, each a line, as start work plans it.
pub fn plan(_: &Config, target: &Value) -> Value {
    let p = super::start::plan_json(&json!({"text": source_of(target), "lastRepo": false}));
    let steps: Vec<Value> = list(&p["steps"]).iter().map(|s| s["text"].clone()).collect();
    let blockers: Vec<Value> = list(&p["blockers"]).iter().map(|b| b["text"].clone()).collect();
    json!({"ok": p["ok"], "kind": p["kind"], "label": p["label"], "steps": steps, "blockers": blockers})
}

/// `start [ticket | pr]`: start work on it once confirmed; a ticket needs its PR's repo, since nobody picks one here.
pub fn start(ctx: &mut Ctx, args: &[Value]) -> Res {
    let source = source_of(arg(args, 0)?);
    if !ctx.confirmed(&format!("start-{source}")) {
        return Ok(());
    }
    super::start::work(ctx, &[json!({"text": source, "lastRepo": false})])
}

// ---------------------------------------------------------------------------------------------------------------------
// Worktrees

/// `remove_worktree [path]`: refused with what keeps it, the merge fetched only when it is the last question; else
/// removed as git allows, then its space closed and its branch deleted, never forced.
pub fn remove_worktree(ctx: &mut Ctx, args: &[Value]) -> Res {
    use cockpit_core::cleanup;
    let path = text(arg(args, 0)?);
    if !ctx.confirmed(&format!("remove-{path}")) {
        return Ok(());
    }
    let dir = Path::new(&path);
    let Some((branch, main)) = cleanup::place(dir).filter(|_| dir.is_dir()) else {
        return Err(format!("{path} is not a git worktree any more"));
    };
    let repo = main.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let merged_at = list(&config::read_inbox()["merged"])
        .iter()
        .find(|m| m["repo"] == repo.as_str() && m["branch"] == branch.as_str())
        .and_then(|m| m["headSha"].as_str().map(str::to_string));
    let facts = cleanup::Facts { merged_at, ..cleanup::Facts::now() };
    let seen = cleanup::look(dir, &facts);
    // What a fetch cannot change refuses at once; only the merge is worth asking the remote.
    let fetched = !seen.blockers.is_empty() && seen.blockers.iter().all(cleanup::Blocker::is_merge);
    let seen = if fetched {
        let pruned = cleanup::fetch(&seen.repo).and_then(|p| p.get(&seen.branch).cloned());
        cleanup::look(dir, &cleanup::Facts { pruned, ..facts })
    } else {
        seen
    };
    if !seen.blockers.is_empty() {
        if fetched {
            cleanup::changed();
            crate::views::touch("inbox");
        }
        return Err(format!("kept {path}: {}", cleanup::said(&seen.blockers)));
    }
    // The folder as the side browser names it, read while it is still there.
    let named = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let removed = git(&seen.repo, &["worktree", "remove", &path]);
    if !removed.ok() {
        return Err(format!("git did not remove {path}: {}", removed.stderr.trim()));
    }
    // The Chromium side browser closes the tabs it kept for this worktree, when it is running.
    if cockpit_core::browser::connect().is_some() {
        let _ = cockpit_core::browser::request(&json!({"op": "worktree.forget", "worktree": named}), std::time::Duration::from_secs(5));
    }
    // Its space closes once the folder is gone, not before git agreed.
    if let Some(open) = inbox::space_on(&path) {
        let id = text(&open["workspace_id"]);
        if id.starts_with(frontend::APP) {
            frontend::close_space(&id);
        } else {
            herdr::run(&["workspace", "close", &id]);
        }
    }
    let branch = &seen.branch;
    let deleted = !branch.is_empty() && git(&seen.repo, &["branch", "-d", branch]).ok();
    cleanup::changed();
    crate::views::touch("inbox");
    ctx.note(if deleted {
        format!("removed {path} and the branch {branch}")
    } else {
        format!("removed {path}; the branch {branch} stays until git counts it merged")
    });
    Ok(())
}

/// `open_worktree [path]`: the space open on it in view, else a new space there.
pub fn open_worktree(ctx: &mut Ctx, args: &[Value]) -> Res {
    let path = text(arg(args, 0)?);
    if let Some(open) = inbox::space_on(&path) {
        inbox::focus_space(&text(&open["workspace_id"]));
        ctx.note(format!("focused \u{201c}{}\u{201d}", text(&open["label"])));
        return Ok(());
    }
    let label = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match frontend::new_space(&path, &label) {
        Some(_) => ctx.note(format!("opened {path} in a new space")),
        None => return Err("no space opened".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(pr: Value) -> Value {
        json!({"repo": {"nameWithOwner": "acme/api", "viewerPermission": "WRITE", "autoMergeAllowed": true, "viewerDefaultMergeMethod": "SQUASH", "rebaseMergeAllowed": false}, "pr": pr})
    }

    #[test]
    fn readiness_says_why_in_one_line() {
        let clean = d(json!({"state": "OPEN", "mergeStateStatus": "CLEAN", "number": 4, "baseRefName": "main"}));
        assert_eq!(readiness(&clean), ("ready", "Ready to merge".to_string(), "good"));
        let (menu, why) = choices(&clean);
        assert_eq!((menu.len(), why.as_str()), (2, ""));
        assert_eq!((menu[0]["method"].as_str(), menu[1]["method"].as_str()), (Some("squash"), Some("merge")));
        let failing = d(
            json!({"state": "OPEN", "mergeStateStatus": "BLOCKED", "commits": {"nodes": [{"commit": {"statusCheckRollup": {"contexts": {"nodes": [{"name": "lint", "conclusion": "FAILURE", "isRequired": true}]}}}}]}}),
        );
        assert_eq!(readiness(&failing).1, "Blocked: 1 required check failing");
        assert_eq!(choices(&failing).0[0]["do"], "auto");
        let draft = d(json!({"state": "OPEN", "isDraft": true}));
        assert_eq!((readiness(&draft).0, choices(&draft).1.as_str()), ("draft", "It's a draft: mark it ready for review first."));
        let queued = d(json!({"state": "OPEN", "isMergeQueueEnabled": true, "mergeQueueEntry": {"position": 2, "state": "QUEUED"}}));
        assert_eq!(readiness(&queued).1, "In the merge queue, #2 in line: waiting its turn");
        assert_eq!(choices(&queued).0[0]["do"], "dequeue");
    }

    #[test]
    fn other_views_call_the_short_way() {
        let args = shorthand("merge", vec![json!({"pr": {"url": "https://github.com/acme/web/pull/12"}, "method": "squash"})]);
        assert_eq!(args, vec![json!({"url": "https://github.com/acme/web/pull/12"}), json!("squash")]);
        let args = shorthand("fix_ci", vec![json!({"url": "u", "repo": "web", "branch": "b"})]);
        assert_eq!((args.len(), &args[2]), (3, &json!("here")));
    }

    #[test]
    fn a_pr_is_known_by_its_url() {
        let cfg = Config(json!({"github": {"org": "acme"}}));
        assert_eq!(coords(&cfg, &json!({"url": "https://github.com/acme/web/pull/12"})).unwrap(), ("acme".into(), "web".into(), "12".into()));
        assert_eq!(coords(&cfg, &json!({"repo": "api", "number": 7})).unwrap(), ("acme".into(), "api".into(), "7".into()));
    }
}
