//! inbox.py's actions: Claude sessions in new spaces, tabs of the actions space or worktrees, mutes, the brief, and the
//! requests (rerun, redeploy, preview, release) the mod carries out.

use std::path::{Path, PathBuf};

use cockpit_core::files;
use serde_json::{Value, json};

use super::{Ctx, arg, opt, space};
use crate::config;
use crate::herdr;
use crate::proc;
use crate::py::{Res, cut, get, join, key, list, now, now_ms, or, quote, s, text, truthy};

fn requests_dir() -> PathBuf {
    files::cache().join("requests")
}

fn muted_path() -> PathBuf {
    files::config_dir().join("muted.json")
}

fn projects_dir() -> PathBuf {
    std::env::var("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|_| files::home().join(".claude")).join("projects")
}

fn is_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

pub fn open_url(ctx: &mut Ctx, args: &[Value]) -> Res {
    let url = text(arg(args, 0)?);
    config::open_url(&url);
    ctx.note(format!("opened {url}"));
    Ok(())
}

pub fn details(ctx: &mut Ctx, args: &[Value]) -> Res {
    let pr = arg(args, 0)?;
    let url = s(pr, "url")?;
    let env = format!("PEEK_URL={url}");
    let result = herdr::run(&["plugin", "pane", "open", "--plugin", "cockpit", "--entrypoint", "peek", "--placement", "popup", "--env", &env]);
    if !result.ok() {
        open_url(ctx, &[Value::from(url)])?;
    }
    Ok(())
}

/// A Claude session in a new space; when a space of that label already runs, it comes into view instead.
fn start_space(ctx: &mut Ctx, cwd: &str, label: &str, prompt: &str, flags: &[String]) {
    if let Some(running) = herdr::spaces().into_iter().find(|w| w["label"].as_str() == Some(label)) {
        herdr::run(&["workspace", "focus", &text(&running["workspace_id"])]);
        ctx.note(format!("\"{label}\" already runs: focused it"));
        return;
    }
    let created = herdr::run(&["workspace", "create", "--cwd", cwd, "--label", label, "--focus"]);
    let Some(pane) = herdr::result_of(&created).and_then(|r| r["root_pane"]["pane_id"].as_str().map(str::to_string)) else {
        ctx.note(format!("herdr workspace create failed: {}", created.stderr.trim()));
        return;
    };
    herdr::run(&["pane", "run", &pane, &claude_line(flags, prompt)]);
    ctx.note(format!("started \"{label}\" in a new space"));
}

fn claude_line(flags: &[String], prompt: &str) -> String {
    let words: Vec<String> = std::iter::once("claude".to_string()).chain(flags.iter().cloned()).chain([prompt.to_string()]).collect();
    join(&words)
}

/// A one-off Claude session in a tab of the actions space, else in a new space; pressed again while it runs, its tab
/// comes back into view.
fn start_action(ctx: &mut Ctx, cwd: &str, label: &str, prompt: &str, flags: &[String]) {
    if let Some(pane) = space::pane_of(label)
        && space::focus(&pane)
    {
        ctx.note(format!("\"{label}\" already runs: focused its tab"));
        return;
    }
    let Some(pane) = space::open_tab(&ctx.cfg, cwd, label, Some(label)) else {
        start_space(ctx, cwd, label, prompt, flags);
        return;
    };
    herdr::run(&["pane", "run", &pane, &claude_line(flags, prompt)]);
    ctx.note(format!("started \"{label}\" in the actions space"));
}

fn repo_dir(ctx: &Ctx, repo: &str) -> String {
    let path = format!("{}/{repo}", ctx.repos());
    if is_dir(&path) { path } else { ctx.repos() }
}

/// Where the branch is checked out: the repo's main checkout or one of its worktrees; None when nowhere.
fn checkout_of(ctx: &Ctx, repo: &str, branch: &Value) -> Option<String> {
    let main = format!("{}/{repo}", ctx.repos());
    if !truthy(branch) || !is_dir(&main) {
        return None;
    }
    let wanted = format!("branch refs/heads/{}", text(branch));
    let listed = proc::run15(&proc::strings(&["git", "-C", &main, "worktree", "list", "--porcelain"]));
    let mut path = None;
    for line in listed.stdout.lines() {
        if let Some(found) = line.strip_prefix("worktree ") {
            path = Some(found.to_string());
        } else if line == wanted {
            return path;
        }
    }
    None
}

fn real(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

/// The herdr space open on a checkout, if any.
fn space_on(path: &str) -> Option<Value> {
    let wanted = real(path);
    herdr::spaces().into_iter().find(|w| w["worktree"]["checkout_path"].as_str().is_some_and(|p| !p.is_empty() && real(p) == wanted))
}

/// A herdr worktree of the PR's branch, from its remote head, with Claude in its space; false when herdr refuses.
fn new_worktree(ctx: &mut Ctx, pr: &Value, label: &str, prompt: &str) -> Res<bool> {
    let main = format!("{}/{}", ctx.repos(), s(pr, "repo")?);
    let branch = get(pr, "branch");
    if !truthy(branch) || !is_dir(&main) {
        return Ok(false);
    }
    let branch = text(branch);
    proc::run15(&proc::strings(&["git", "-C", &main, "fetch", "-q", "origin", &branch]));
    let base = format!("origin/{branch}");
    let created = herdr::run(&["worktree", "create", "--cwd", &main, "--branch", &branch, "--base", &base, "--label", label, "--focus"]);
    let Some(pane) = herdr::result_of(&created).and_then(|r| r["root_pane"]["pane_id"].as_str().map(str::to_string)) else {
        return Ok(false);
    };
    herdr::run(&["pane", "run", &pane, &format!("claude {}", quote(prompt))]);
    ctx.note(format!("started \"{label}\" in a new worktree"));
    Ok(true)
}

/// Claude on the PR's branch: the space open on its checkout, else a new space there, else a new worktree with
/// `worktrees` on, else the main checkout where it checks the branch out.
fn on_branch(ctx: &mut Ctx, pr: &Value, label: &str, prompt: &str) -> Res {
    let branch = get(pr, "branch");
    let repo = s(pr, "repo")?;
    if let Some(path) = checkout_of(ctx, &repo, branch) {
        if let Some(open) = space_on(&path) {
            herdr::run(&["workspace", "focus", &text(&open["workspace_id"])]);
            ctx.note(format!("{} is already open in \"{}\": focused it", text(branch), text(&open["label"])));
            return Ok(());
        }
        start_space(ctx, &path, label, &format!("{prompt} The branch {} is checked out here, in {path}: work in it.", text(branch)), &[]);
        return Ok(());
    }
    if truthy(get(&ctx.cfg.0, "worktrees")) && new_worktree(ctx, pr, label, &format!("{prompt} This is a new worktree of {}: work in it.", text(branch)))? {
        return Ok(());
    }
    let dir = repo_dir(ctx, &repo);
    start_space(ctx, &dir, label, &format!("{prompt} Check the branch {} out first.", text(branch)), &[]);
    Ok(())
}

pub fn review(ctx: &mut Ctx, args: &[Value]) -> Res {
    let pr = arg(args, 0)?;
    let (repo, number) = (s(pr, "repo")?, s(pr, "number")?);
    let target = if is_dir(&format!("{}/{repo}", ctx.repos())) { number.clone() } else { s(pr, "url")? };
    let dir = repo_dir(ctx, &repo);
    start_action(ctx, &dir, &format!("review {repo}#{number}"), &format!("/code-review {target}"), &[]);
    Ok(())
}

pub fn fix_ci(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (pr, ci_run, place) = (arg(args, 0)?, arg(args, 1)?, text(arg(args, 2)?));
    let failed = get(ci_run, "failed");
    let mut job = String::new();
    if truthy(failed) {
        let step = if truthy(get(failed, "step")) { format!(", step \"{}\"", s(failed, "step")?) } else { String::new() };
        job = format!(" The failing job is {}{step}: read its logs with {}.", s(failed, "job")?, or(failed, "logs", "the CI connector"));
    }
    let mut checks = Vec::new();
    for check in list(get(pr, "failedChecks")) {
        if !truthy(failed) || check != key(failed, "job")? {
            checks.push(text(check));
        }
    }
    let github =
        if checks.is_empty() { String::new() } else { format!(" Failing GitHub checks: {} (gh pr checks, gh run view --log-failed).", checks.join(", ")) };
    let (repo, url) = (s(pr, "repo")?, s(pr, "url")?);
    let label = format!("ci {repo}#{}", s(pr, "number")?);
    if place == "here" {
        let prompt = format!("CI is failing on {url}.{job}{github} The fix goes on this PR's branch. Explain the cause and propose a fix. Do not push.");
        return on_branch(ctx, pr, &label, &prompt);
    }
    let plan = format!(
        " Treat it as unrelated to this PR (flaky test or broken main): fix it on a new branch off an updated local main and prepare a separate PR, leaving {} untouched.",
        text(get(pr, "branch"))
    );
    let dir = repo_dir(ctx, &repo);
    start_space(ctx, &dir, &label, &format!("CI is failing on {url}.{job}{github}{plan} Explain the cause and propose a fix. Do not push."), &[]);
    Ok(())
}

pub fn plan(ctx: &mut Ctx, args: &[Value]) -> Res {
    let ticket = arg(args, 0)?;
    let id = s(ticket, "id")?;
    let prompt = format!("Pick up {id} ({}): read the ticket, find the repos involved, and propose a plan before coding.", s(ticket, "url")?);
    let repos = ctx.repos();
    start_action(ctx, &repos, &format!("plan {}", id.to_lowercase()), &prompt, &[]);
    Ok(())
}

/// The ticket's work: a space already open on its PR's branch, else its plan's tab moved out to a space of its own,
/// else a new session.
pub fn continue_ticket(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (ticket, prs) = (arg(args, 0)?, list(arg(args, 1)?));
    let id = s(ticket, "id")?;
    let lower = id.to_lowercase();
    let path = match prs.first() {
        Some(pr) => checkout_of(ctx, &s(pr, "repo")?, get(pr, "branch")),
        None => None,
    };
    let has_space = path.as_deref().is_some_and(|p| space_on(p).is_some());
    if !has_space && space::promote(&format!("plan {lower}"), &lower) {
        ctx.note(format!("moved the plan of {id} to a space of its own"));
        return Ok(());
    }
    let url = s(ticket, "url")?;
    if let Some(pr) = prs.first() {
        let prompt = format!(
            "Continue {id} ({url}). Its PR is {} on branch {} in {}: read the PR's state, review comments and CI, and summarize where it stands before changing anything.",
            s(pr, "url")?,
            text(get(pr, "branch")),
            s(pr, "repo")?
        );
        return on_branch(ctx, pr, &lower, &prompt);
    }
    let prompt = format!(
        "Continue {id} ({url}). Find its branch (named after {lower}) in the repo it touches, check it out, and summarize where it stands before changing anything."
    );
    let repos = ctx.repos();
    start_space(ctx, &repos, &lower, &prompt, &[]);
    Ok(())
}

/// The folder a Claude session started in, read from its transcript; None once the transcript is gone.
fn session_cwd(session: &str) -> Option<String> {
    let mut projects: Vec<PathBuf> = std::fs::read_dir(projects_dir()).ok()?.flatten().map(|e| e.path()).collect();
    projects.sort();
    for project in projects {
        let Ok(transcript) = std::fs::read_to_string(project.join(format!("{session}.jsonl"))) else {
            continue;
        };
        for line in transcript.lines().take(50) {
            let cwd = serde_json::from_str::<Value>(line).ok().and_then(|v| v.get("cwd").and_then(Value::as_str).map(str::to_string));
            if let Some(cwd) = cwd.filter(|c| !c.is_empty() && is_dir(c)) {
                return Some(cwd);
            }
        }
    }
    None
}

/// The session that spent the most on the ticket, with the folder it resumes from.
fn ticket_session(id: &str) -> Option<(String, String)> {
    let mut spent: Vec<(f64, String)> = Vec::new();
    for entry in std::fs::read_dir(files::cache().join("costs")).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(session) = name.strip_suffix(".json") else {
            continue;
        };
        let usd = files::load(&entry.path()).and_then(|c| c.get(id).and_then(Value::as_f64)).unwrap_or(0.0);
        if usd != 0.0 {
            spent.push((usd, session.to_string()));
        }
    }
    spent.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    spent.into_iter().find_map(|(_, session)| session_cwd(&session).map(|cwd| (session, cwd)))
}

/// What the inbox knows of a ticket, on one line: its PRs, deploys, previews and runbook.
fn ticket_brief(ticket: &Value, prs: &[Value], data: &Value) -> Res<String> {
    let id = s(ticket, "id")?;
    let mut first = format!("{id} \"{}\"", s(ticket, "title")?);
    if truthy(get(ticket, "url")) {
        first += &format!(" ({})", s(ticket, "url")?);
    }
    if truthy(get(ticket, "status")) {
        first += &format!(" is {}", s(ticket, "status")?);
    }
    let mut facts = vec![first];
    for pr in prs {
        let review = or(pr, "review", "no review").to_lowercase().replace('_', " ");
        let checks = if truthy(get(pr, "failedChecks")) {
            format!(", failing {}", list(key(pr, "failedChecks")?).iter().map(text).collect::<Vec<_>>().join(", "))
        } else {
            String::new()
        };
        let draft = if truthy(get(pr, "isDraft")) { "draft, " } else { "" };
        facts.push(format!(
            "PR {} on branch {} (base {}): {draft}{review}, CI {}{checks}",
            s(pr, "url")?,
            text(get(pr, "branch")),
            text(get(pr, "base")),
            or(pr, "ci", "none")
        ));
    }
    for done in list(get(get(data, "deploys"), &id)) {
        let live =
            if truthy(key(done, "isLive")?) { "is live in production".to_string() } else { format!("is not in production yet, prod runs {}", s(done, "tag")?) };
        facts.push(format!("{}#{} {live}", s(done, "repo")?, s(done, "number")?));
    }
    let branches: Vec<(String, String)> = prs.iter().map(|pr| Ok((s(pr, "repo")?, text(get(pr, "branch"))))).collect::<Res<_>>()?;
    for preview in list(get(data, "previews")) {
        if branches.contains(&(s(preview, "app")?, s(preview, "branch")?)) {
            let stopped = if truthy(get(preview, "activated")) { "" } else { " (stopped)" };
            facts.push(format!("preview {} runs {} {}{stopped}", s(preview, "id")?, s(preview, "app")?, s(preview, "branch")?));
        }
    }
    let runbook = files::config_dir().join("runbooks").join(format!("{}.md", id.to_lowercase()));
    if runbook.exists() {
        facts.push(format!("its runbook is {}", runbook.display()));
    }
    Ok(facts.join("; ") + ".")
}

/// A session to talk the ticket over, changing nothing: a fork of `owner` when given, else of the session that worked
/// on it most, else a fresh one.
pub fn talk(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (ticket, prs, data) = (arg(args, 0)?, list(arg(args, 1)?), arg(args, 2)?);
    talk_over(ctx, ticket, prs, data, opt(args, 3).as_str())
}

pub fn talk_over(ctx: &mut Ctx, ticket: &Value, prs: &[Value], data: &Value, owner: Option<&str>) -> Res {
    let brief = ticket_brief(ticket, prs, data)?;
    let hands_off = " Change nothing (no edits, commits, pushes or ticket updates) unless I ask.";
    let id = s(ticket, "id")?;
    let owned = owner.filter(|o| !o.is_empty()).and_then(|o| session_cwd(o).map(|cwd| (o.to_string(), cwd)));
    let label = format!("talk {}", id.to_lowercase());
    if let Some((session, cwd)) = owned.or_else(|| ticket_session(&id)) {
        let prompt = format!(
            "This is a fork of your session on {id}, opened to talk it over. Per the inbox now: {brief} Catch up on what changed since (the ticket's comments, the PRs' reviews and CI), sum it up in a few lines, then wait for my questions.{hands_off}"
        );
        start_action(ctx, &cwd, &label, &prompt, &proc::strings(&["--resume", &session, "--fork-session"]));
        return Ok(());
    }
    let prompt = format!(
        "Let's talk {id} over. Per the inbox: {brief} Read the ticket with its comments and the PRs with their reviews and CI, sum up where it stands in a few lines, then wait for my questions.{hands_off}"
    );
    let repos = ctx.repos();
    start_action(ctx, &repos, &label, &prompt, &[]);
    Ok(())
}

/// Files a request the mod carries out, and the row's pending entry the app follows to its outcome.
fn request(ctx: &mut Ctx, payload: Value, row: &str, working: &str) -> Res {
    let name = format!("{}-{}.json", now_ms(), s(&payload, "type")?);
    files::write(&requests_dir().join(&name), &payload).map_err(|e| e.to_string())?;
    ctx.pending.insert(row.into(), json!({"file": name, "working": working, "label": payload["label"], "at": now()}));
    Ok(())
}

pub fn rerun(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (pr, ci_run) = (arg(args, 0)?, arg(args, 1)?);
    if ctx.confirmed(&format!("rerun-{}", s(ci_run, "workflowId")?)) {
        let label = format!("rerun {}#{}", s(pr, "repo")?, s(pr, "number")?);
        request(ctx, json!({"type": "rerun", "workflowId": key(ci_run, "workflowId")?, "label": label}), &s(pr, "url")?, "rerunning failed jobs…")?;
    }
    Ok(())
}

pub fn redeploy(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (preview, row) = (arg(args, 0)?, text(arg(args, 1)?));
    let (id, app) = (s(preview, "id")?, s(preview, "app")?);
    if ctx.confirmed(&format!("redeploy-{id}-{app}")) {
        let label = format!("redeploy {app} on {id}");
        request(ctx, json!({"type": "redeploy", "preview": key(preview, "id")?, "app": key(preview, "app")?, "label": label}), &row, "redeploying…")?;
    }
    Ok(())
}

pub fn create_preview(ctx: &mut Ctx, args: &[Value]) -> Res {
    let pr = arg(args, 0)?;
    let (repo, branch) = (s(pr, "repo")?, text(get(pr, "branch")));
    if ctx.confirmed(&format!("preview-{repo}-{branch}")) {
        let payload = json!({"type": "create-preview", "app": key(pr, "repo")?, "branch": get(pr, "branch"), "label": format!("preview for {repo} {branch}")});
        request(ctx, payload, &s(pr, "url")?, "creating…")?;
    }
    Ok(())
}

pub fn stop_preview(ctx: &mut Ctx, args: &[Value]) -> Res {
    let preview = arg(args, 0)?;
    let id = text(preview);
    if ctx.confirmed(&format!("stop-{id}")) {
        request(ctx, json!({"type": "stop-preview", "preview": preview, "label": format!("stop {id}")}), &format!("preview-{id}"), "stopping…")?;
    }
    Ok(())
}

pub fn extend(ctx: &mut Ctx, args: &[Value]) -> Res {
    let preview = arg(args, 0)?;
    let days = if opt(args, 1).is_null() { json!(2) } else { opt(args, 1).clone() };
    let id = text(preview);
    if ctx.confirmed(&format!("extend-{id}")) {
        request(ctx, json!({"type": "extend", "preview": preview, "days": days, "label": format!("extend {id}")}), &format!("preview-{id}"), "extending…")?;
    }
    Ok(())
}

pub fn mark_done(ctx: &mut Ctx, args: &[Value]) -> Res {
    let ticket = arg(args, 0)?;
    let id = s(ticket, "id")?;
    if ctx.confirmed(&format!("done-{id}")) {
        request(ctx, json!({"type": "release", "ticket": key(ticket, "id")?, "label": id}), &id, "moving to Done…")?;
    }
    Ok(())
}

pub fn set_muted(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (repo, is_muted) = (text(arg(args, 0)?), truthy(arg(args, 1)?));
    let mut muted: Vec<String> = files::load(&muted_path()).map(|m| list(&m).iter().map(text).collect()).unwrap_or_default();
    muted.retain(|r| r != &repo);
    if is_muted {
        muted.push(repo.clone());
    }
    muted.sort();
    muted.dedup();
    files::write(&muted_path(), &json!(muted)).map_err(|e| e.to_string())?;
    ctx.note(format!("{} {repo}", if is_muted { "muted" } else { "unmuted" }));
    Ok(())
}

/// Hides today's brief; tomorrow's shows again.
pub fn dismiss_brief(ctx: &mut Ctx, _: &[Value]) -> Res {
    let path = files::cache().join("brief.json");
    let mut brief = files::load(&path).filter(Value::is_object).unwrap_or_else(|| json!({}));
    brief["dismissed"] = json!(true);
    files::write(&path, &brief).map_err(|e| e.to_string())?;
    ctx.note("brief dismissed until tomorrow");
    Ok(())
}

pub fn request_refresh(ctx: &mut Ctx, _: &[Value]) -> Res {
    std::fs::create_dir_all(files::cache()).map_err(|e| e.to_string())?;
    std::fs::write(files::cache().join("refresh-request"), now().to_string()).map_err(|e| e.to_string())?;
    ctx.note("refresh requested, it lands within a minute");
    Ok(())
}

/// Closes the space; a worktree's space goes with its checkout, unless the checkout holds changes.
pub fn close_space(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (space, label, is_worktree) = (text(arg(args, 0)?), text(arg(args, 1)?), truthy(arg(args, 2)?));
    if !ctx.confirmed(&format!("close-{space}")) {
        return Ok(());
    }
    if let Some(tab) = space.strip_prefix("tab:") {
        let result = herdr::run(&["tab", "close", tab]);
        ctx.note(if result.ok() { format!("closed {label}") } else { format!("herdr tab close failed: {}", result.stderr.trim()) });
        return Ok(());
    }
    if is_worktree {
        let result = herdr::run(&["worktree", "remove", "--workspace", &space]);
        ctx.note(if result.ok() {
            format!("removed the worktree of {label}")
        } else {
            format!("the worktree of {label} was not removed (uncommitted changes?): {}", cut(result.stderr.trim(), 120))
        });
        return Ok(());
    }
    let result = herdr::run(&["workspace", "close", &space]);
    ctx.note(if result.ok() { format!("closed {label}") } else { format!("herdr workspace close failed: {}", result.stderr.trim()) });
    Ok(())
}

/// A command the row hands over as argv, such as herdr's focus of a space: run in the background, no note.
pub fn run(_: &mut Ctx, args: &[Value]) -> Res {
    let argv: Vec<String> = list(arg(args, 0)?).iter().map(text).collect();
    proc::spawn(&argv, None, None, |_| {}).map(|_| ()).map_err(|e| e.to_string())
}
