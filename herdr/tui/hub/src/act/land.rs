//! Landing PRs and starting work: whether GitHub would merge a PR now and what the merge menu offers (after
//! mrolafsson/herdr-github's readiness), merge, auto-merge, draft and the merge queue through gh; a ticket or a PR to a
//! worktree with Claude in a space of its own, checked first; and the worktrees of merged branches removed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cockpit_core::{files, frontend};
use serde_json::{Value, json};

use super::{Ctx, arg, inbox, opt, tickets};
use crate::config::{self, Config};
use crate::herdr;
use crate::proc;
use crate::value::{Res, list, s, text};

const GH_TIMEOUT: Duration = Duration::from_secs(40);

fn gh(args: &[&str]) -> Res<String> {
    let argv: Vec<String> = std::iter::once("gh").chain(args.iter().copied()).map(str::to_string).collect();
    let out = proc::run(&argv, GH_TIMEOUT, None);
    if out.ok() { Ok(out.stdout) } else { Err(format!("gh: {}", out.stderr.trim().lines().next().unwrap_or("failed"))) }
}

fn git(dir: &Path, args: &[&str]) -> proc::Out {
    let argv: Vec<String> = ["git", "-C", &dir.to_string_lossy()].into_iter().map(str::to_string).chain(args.iter().map(|a| a.to_string())).collect();
    proc::run(&argv, Duration::from_secs(60), None)
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
    let out = gh(&["api", "graphql", "-f", &format!("query={DETAIL}"), "-F", &format!("owner={owner}"), "-F", &format!("name={name}"), "-F", &format!("number={number}")])?;
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

const METHODS: [(&str, &str, &str); 3] = [("SQUASH", "squash", "Squash and merge"), ("MERGE", "merge", "Create a merge commit"), ("REBASE", "rebase", "Rebase and merge")];

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
pub fn merge(ctx: &mut Ctx, args: &[Value]) -> Res {
    let pr = arg(args, 0)?;
    let d = detail(&ctx.cfg, pr)?;
    let (owner, name, n) = coords(&ctx.cfg, pr)?;
    let head = d["pr"]["headRefOid"].as_str().unwrap_or_default().to_string();
    if d["pr"]["state"] != "OPEN" {
        return Err(format!("#{n} is {}", d["pr"]["state"].as_str().unwrap_or("closed").to_lowercase()));
    }
    if d["pr"]["isMergeQueueEnabled"] == true {
        let id = d["pr"]["id"].as_str().unwrap_or_default();
        mutate("mutation($id: ID!, $oid: GitObjectID) { enqueuePullRequest(input: {pullRequestId: $id, expectedHeadOid: $oid}) { clientMutationId } }", &[("id", id), ("oid", &head)])?;
        after_change(ctx, format!("{name}#{n} joined the merge queue"));
        return Ok(());
    }
    let method = opt(args, 1).as_str().map(str::to_string).unwrap_or_else(|| allowed(&d).first().map(|m| m.1.to_string()).unwrap_or("squash".into()));
    let repo = format!("{owner}/{name}");
    gh(&["pr", "merge", &n, "--repo", &repo, method_flag(&method)?, "--match-head-commit", &head])?;
    after_change(ctx, format!("merged {name}#{n} ({method}): its worktree can go from Alerts"));
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
    set_draft(ctx, args, false)
}

pub fn draft(ctx: &mut Ctx, args: &[Value]) -> Res {
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
// From a ticket or a PR to a worktree with Claude

/// Where a new worktree of the branch goes: herdr's convention, ~/.herdr/worktrees/<repo>/<branch>.
fn worktree_path(repo: &str, branch: &str) -> PathBuf {
    files::home().join(".herdr/worktrees").join(repo).join(branch.replace('/', "-"))
}

fn is_pr(target: &Value) -> bool {
    target["url"].as_str().is_some_and(|u| u.contains("/pull/")) || (target["number"].is_number() && target["repo"].is_string())
}

/// Whether a space of this label is open, in either frontend.
fn space_named(label: &str) -> bool {
    match frontend::current() {
        frontend::Frontend::App => frontend::app_space_named(label).is_some(),
        frontend::Frontend::Tui => herdr::spaces().iter().any(|w| w["label"] == label),
    }
}

/// Whether a branch names the ticket as a whole token: eng-4-login does, eng-42-login does not.
fn names_ticket(branch: &str, ticket: &str) -> bool {
    let (branch, key) = (branch.to_lowercase(), ticket.to_lowercase());
    !key.is_empty()
        && branch.match_indices(&key).any(|(at, _)| {
            let before = branch[..at].chars().next_back();
            let after = branch[at + key.len()..].chars().next();
            !before.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
        })
}

/// What `start` does for a target, or why it can't: {ok, kind, label, steps, blockers, ...what it needs to run}.
pub fn plan(cfg: &Config, target: &Value) -> Value {
    if is_pr(target) { pr_plan(cfg, target, None) } else { ticket_plan(cfg, target) }
}

fn finish(mut plan: Value) -> Value {
    plan["ok"] = json!(list(&plan["blockers"]).is_empty());
    plan
}

fn pr_plan(cfg: &Config, target: &Value, ticket: Option<&Value>) -> Value {
    let mut steps: Vec<String> = Vec::new();
    let mut blockers: Vec<String> = Vec::new();
    let d = match detail(cfg, target) {
        Ok(d) => d,
        Err(e) => return finish(json!({"kind": "pr", "steps": [], "blockers": [format!("GitHub: {e}")]})),
    };
    let pr = &d["pr"];
    let repo = d["repo"]["nameWithOwner"].as_str().unwrap_or_default().rsplit('/').next().unwrap_or_default().to_string();
    let branch = pr["headRefName"].as_str().unwrap_or_default().to_string();
    let label = ticket.map(|t| s(t, "id").unwrap_or_default().to_lowercase()).unwrap_or_else(|| format!("{repo}#{}", text(&pr["number"])));
    if pr["state"] != "OPEN" {
        blockers.push(format!("{repo}#{} is {}", text(&pr["number"]), pr["state"].as_str().unwrap_or("closed").to_lowercase()));
    }
    let main = PathBuf::from(format!("{}/{repo}", cfg.repos()));
    let mut path = String::new();
    let mut place = "worktree";
    if space_named(&label) {
        place = "focus";
        steps.push(format!("\u{201c}{label}\u{201d} already runs: bring it into view"));
    } else if !main.join(".git").exists() {
        blockers.push(format!("no checkout of {repo} in {}: clone it there first", cfg.repos()));
    } else if let Some(found) = inbox::checkout_in(&main.to_string_lossy(), &branch) {
        path = found.clone();
        if inbox::space_on(&found).is_some() {
            place = "space";
            steps.push(format!("{branch} is open in a space already: bring it into view"));
        } else {
            place = "checkout";
            steps.push(format!("{branch} is checked out in {found}: Claude starts there in a new space \u{201c}{label}\u{201d}"));
        }
    } else {
        let at = worktree_path(&repo, &branch);
        path = at.to_string_lossy().into_owned();
        if at.exists() {
            blockers.push(format!("{path} exists but is not a worktree of {branch}"));
        }
        let from = if pr["isCrossRepository"] == true { format!("{}'s fork (pull/{}/head)", pr["headRepositoryOwner"]["login"].as_str().unwrap_or("its author"), text(&pr["number"])) } else { format!("origin/{branch}") };
        steps.push(format!("fetch {branch} from {from}"));
        steps.push(format!("add a worktree of it at {path}"));
        steps.push(format!("start Claude there in a new space \u{201c}{label}\u{201d}"));
    }
    finish(json!({
        "kind": "pr", "label": label, "place": place, "path": path, "repo": repo, "main": main, "branch": branch,
        "number": pr["number"], "url": pr["url"], "title": pr["title"], "cross": pr["isCrossRepository"], "steps": steps, "blockers": blockers,
    }))
}

fn ticket_plan(cfg: &Config, target: &Value) -> Value {
    let id = target["id"].as_str().or(target.as_str()).unwrap_or_default().to_uppercase();
    if id.is_empty() {
        return finish(json!({"kind": "ticket", "steps": [], "blockers": ["no ticket given"]}));
    }
    let inbox = config::read_inbox();
    let known = list(&inbox["tickets"]).iter().find(|t| t["id"] == id.as_str()).cloned().unwrap_or_else(|| json!({"id": id}));
    let mut steps: Vec<String> = Vec::new();
    let mut blockers: Vec<String> = Vec::new();
    let mut branch = String::new();
    let issue = tickets::issue(&id);
    let ticket_name = cfg.0["tickets"]["name"].as_str().unwrap_or("Linear").to_string();
    match &issue {
        Ok(Some(issue)) => {
            branch = issue["branchName"].as_str().unwrap_or_default().to_string();
            match issue["state"]["type"].as_str() {
                Some("completed") | Some("canceled") => blockers.push(format!("{id} is {}: reopen it first", issue["state"]["name"].as_str().unwrap_or("closed"))),
                Some("started") => {}
                _ => steps.push(format!("move {id} to In Progress")),
            }
            match issue["assignee"].as_object() {
                None => steps.push(format!("assign {id} to you")),
                Some(a) if a.get("isMe") != Some(&json!(true)) => {
                    blockers.push(format!("{id} is assigned to {}: take it over in {ticket_name} first", a.get("displayName").and_then(Value::as_str).unwrap_or("someone else")))
                }
                Some(_) => {}
            }
        }
        Ok(None) => {
            if known["statusType"] != "started" {
                steps.push(format!("the session moves {id} to In Progress and assigns it to you ({ticket_name} has no token here)"));
            }
        }
        Err(e) => blockers.push(format!("{ticket_name}: {e}")),
    }
    let mine = list(&inbox["mine"]).iter().find(|pr| pr["branch"].as_str().is_some_and(|b| names_ticket(b, &id))).cloned();
    if let Some(pr) = mine {
        let mut on = pr_plan(cfg, &pr, Some(&known));
        let mut all: Vec<Value> = steps.into_iter().map(Value::from).collect();
        all.extend(list(&on["steps"]).iter().cloned());
        let mut stop: Vec<Value> = blockers.into_iter().map(Value::from).collect();
        stop.extend(list(&on["blockers"]).iter().cloned());
        on["steps"] = json!(all);
        on["blockers"] = json!(stop);
        on["ticket"] = known;
        on["issue"] = issue.ok().flatten().unwrap_or(Value::Null);
        return finish(on);
    }
    let label = id.to_lowercase();
    let place = if space_named(&label) { "focus" } else { "repos" };
    if place == "focus" {
        steps = vec![format!("\u{201c}{label}\u{201d} already runs: bring it into view")];
    } else {
        let name = if branch.is_empty() { format!("a branch named after {label}") } else { format!("the branch {branch}") };
        steps.push(format!("start Claude in {} in a new space \u{201c}{label}\u{201d}: it finds the repo and makes {name} in a worktree", cfg.repos()));
    }
    finish(json!({"kind": "ticket", "label": label, "place": place, "branch": branch, "ticket": known, "issue": issue.ok().flatten(), "steps": steps, "blockers": blockers}))
}

/// `start [ticket | pr]`: the plan's checks, then the ticket moved and assigned, then Claude on the branch.
pub fn start(ctx: &mut Ctx, args: &[Value]) -> Res {
    let target = arg(args, 0)?;
    let key = if is_pr(target) { target["url"].as_str().unwrap_or_default().to_string() } else { target["id"].as_str().or(target.as_str()).unwrap_or_default().to_uppercase() };
    if !ctx.confirmed(&format!("start-{key}")) {
        return Ok(());
    }
    let plan = plan(&ctx.cfg, target);
    if plan["ok"] != true {
        let why: Vec<String> = list(&plan["blockers"]).iter().map(text).collect();
        return Err(format!("can't start: {}", why.join("; ")));
    }
    let label = plan["label"].as_str().unwrap_or_default().to_string();
    let mut moved = String::new();
    if let Some(issue) = plan["issue"].as_object() {
        let id = issue.get("identifier").and_then(Value::as_str).unwrap_or_default();
        moved = tickets::take(id, issue).map_err(|e| format!("{id}: {e}"))?;
    }
    let ask = if plan["issue"].is_null() && plan["ticket"].is_object() && plan["ticket"]["statusType"] != "started" {
        let id = plan["ticket"]["id"].as_str().unwrap_or_default();
        format!(" First move {id} to In Progress and assign it to me in {}.", ctx.cfg.0["tickets"]["name"].as_str().unwrap_or("the ticket tool"))
    } else {
        String::new()
    };
    match plan["place"].as_str().unwrap_or_default() {
        "focus" => {
            frontend::focus_space_named(&label);
            ctx.note(format!("\u{201c}{label}\u{201d} already runs: focused it"));
        }
        "space" => {
            if let Some(open) = inbox::space_on(plan["path"].as_str().unwrap_or_default()) {
                inbox::focus_space(&text(&open["workspace_id"]));
            }
            ctx.note(format!("{} is already open: focused it", plan["branch"].as_str().unwrap_or_default()));
        }
        "checkout" | "worktree" => {
            let path = plan["path"].as_str().unwrap_or_default().to_string();
            if plan["place"] == "worktree" {
                add_worktree(&plan)?;
            }
            let prompt = format!(
                "Work on {} ({}) on its branch {}, checked out in {path}.{ask} Read the PR's state, review comments and CI, and summarize where it stands before changing anything.",
                text(&plan["url"]),
                text(&plan["title"]),
                text(&plan["branch"])
            );
            inbox::start_space(ctx, &path, &label, &prompt, &[]);
        }
        _ => {
            let t = &plan["ticket"];
            let branch = plan["branch"].as_str().filter(|b| !b.is_empty()).map(|b| format!("the branch {b}")).unwrap_or_else(|| format!("a branch named after {label}"));
            let prompt = format!(
                "Start {} ({}): {}.{ask} Read the ticket, find the repo it touches, create {branch} in a git worktree of that repo (git worktree add ~/.herdr/worktrees/<repo>/<branch>), and propose a plan before coding.",
                text(&t["id"]),
                t["url"].as_str().unwrap_or(""),
                t["title"].as_str().unwrap_or("")
            );
            let repos = ctx.cfg.repos();
            inbox::start_space(ctx, &repos, &label, &prompt, &[]);
        }
    }
    if !moved.is_empty() {
        ctx.note(format!("{} · {moved}", ctx.note));
    }
    Ok(())
}

fn add_worktree(plan: &Value) -> Res {
    let main = PathBuf::from(plan["main"].as_str().unwrap_or_default());
    let (path, branch) = (plan["path"].as_str().unwrap_or_default(), plan["branch"].as_str().unwrap_or_default());
    if let Some(dir) = Path::new(path).parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let fetched = if plan["cross"] == true {
        git(&main, &["fetch", "-q", "origin", &format!("pull/{}/head:{branch}", text(&plan["number"]))])
    } else {
        git(&main, &["fetch", "-q", "origin", branch])
    };
    if !fetched.ok() {
        return Err(format!("git fetch: {}", fetched.stderr.trim()));
    }
    let has_local = git(&main, &["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")]).ok();
    let added = if has_local {
        git(&main, &["worktree", "add", path, branch])
    } else {
        git(&main, &["worktree", "add", "--track", "-b", branch, path, &format!("origin/{branch}")])
    };
    if added.ok() { Ok(()) } else { Err(format!("git worktree add: {}", added.stderr.trim())) }
}

// ---------------------------------------------------------------------------------------------------------------------
// Worktrees

/// `remove_worktree [path]`: its space closed, the worktree removed and its branch deleted, never forced.
pub fn remove_worktree(ctx: &mut Ctx, args: &[Value]) -> Res {
    let path = text(arg(args, 0)?);
    if !ctx.confirmed(&format!("remove-{path}")) {
        return Ok(());
    }
    let dir = Path::new(&path);
    let status = git(dir, &["--no-optional-locks", "status", "--porcelain"]);
    if !status.ok() {
        return Err(format!("{path} is not a git worktree any more"));
    }
    let changes = status.stdout.lines().count();
    if changes > 0 {
        return Err(format!("{path} holds {changes} uncommitted change{}: commit or drop them first", if changes == 1 { "" } else { "s" }));
    }
    let branch = git(dir, &["branch", "--show-current"]).stdout.trim().to_string();
    let common = git(dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).stdout.trim().to_string();
    let main = Path::new(&common).parent().map(Path::to_path_buf).ok_or("its main checkout is unknown")?;
    if let Some(open) = inbox::space_on(&path) {
        let id = text(&open["workspace_id"]);
        if id.starts_with(frontend::APP) {
            frontend::close_space(&id);
        } else {
            herdr::run(&["workspace", "close", &id]);
        }
    }
    let removed = git(&main, &["worktree", "remove", &path]);
    if !removed.ok() {
        return Err(format!("git worktree remove: {}", removed.stderr.trim()));
    }
    let deleted = !branch.is_empty() && git(&main, &["branch", "-d", &branch]).ok();
    ctx.note(if deleted { format!("removed {path} and the branch {branch}") } else { format!("removed {path}; the branch {branch} stays (not merged here)") });
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
        let failing = d(json!({"state": "OPEN", "mergeStateStatus": "BLOCKED", "commits": {"nodes": [{"commit": {"statusCheckRollup": {"contexts": {"nodes": [{"name": "lint", "conclusion": "FAILURE", "isRequired": true}]}}}}]}}));
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
        assert_eq!(worktree_path("api", "me/eng-4-login").file_name().unwrap(), "me-eng-4-login");
    }
}
