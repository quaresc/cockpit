//! The runbook's actions: a step handed to the runbook's own agent session, marked done by hand, a background refresh
//! by a headless agent, a talk about its ticket, and the notification a finished refresh sends.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use cockpit_core::agent::{self, Headless};
use cockpit_core::{files, transcripts};
use serde_json::{Value, json};

use super::book::{handoffs_dir, read_books, read_handoffs, runbooks_dir};
use super::{Ctx, arg, inbox, opt};
use crate::config;
use crate::herdr;
use crate::proc;
use crate::value::{Res, get, key, list, now, now_ms, s, text, truthy};
use cockpit_core::frontend;

const REPORT_TOOL: &str = "mcp__cockpit__report_step";
/// A handoff no session took after this many seconds no longer counts as in flight.
const SENT_WAIT: f64 = 900.0;
const OPEN: [&str; 4] = ["⬜", "⏳", "🟡", "❌"];

/// The refreshes this hub started, by slug, while they run.
static REFRESHING: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

fn owner_file(book: &Value) -> Res<Value> {
    Ok(files::load(&files::cache().join("runbook-owners").join(format!("{}.json", s(book, "slug")?))).unwrap_or(Value::Null))
}

fn owner_of(book: &Value) -> Res<(String, String)> {
    let owner = owner_file(book)?;
    // A null owner field reads as empty, a missing one as "-".
    let field = |name: &str| owner.get(name).map(|v| if v.is_null() { String::new() } else { text(v) }).unwrap_or_else(|| "-".into());
    Ok((field("pane"), field("session")))
}

fn handoff_id(book: &Value, step: &Value) -> Res<String> {
    Ok(format!("{}-{}", s(book, "slug")?, s(step, "n")?))
}

fn write_handoff(handoff: &Value) -> Res {
    files::write(&handoffs_dir().join(format!("{}.json", s(handoff, "id")?)), handoff).map_err(|e| e.to_string())
}

fn in_flight(handoff: &Value) -> bool {
    match handoff["state"].as_str() {
        Some("sent") => now() - handoff["sentAt"].as_f64().unwrap_or(0.0) / 1000.0 <= SENT_WAIT,
        Some("running" | "waiting") => true,
        _ => false,
    }
}

/// Hands the step to the runbook's own agent conversation, resumed if it is closed; that session owes its outcome.
pub fn run_step(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (book, step) = (arg(args, 0)?, arg(args, 1)?);
    let (slug, n) = (s(book, "slug")?, s(step, "n")?);
    if !ctx.confirmed(&format!("run-{slug}-{n}")) {
        ctx.note(format!("click run again to hand step {n} to the {} session", slug.to_uppercase()));
        return Ok(());
    }
    let (pane, session) = owner_of(book)?;
    let handoff = json!({"id": handoff_id(book, step)?, "slug": slug, "step": key(step, "n")?, "title": key(step, "step")?, "path": key(book, "path")?,
                         "state": "sent", "sentAt": now_ms()});
    write_handoff(&handoff)?;
    let command = if truthy(key(step, "command")?) { format!(", command `{}`", s(step, "command")?) } else { String::new() };
    let prompt = format!(
        "Run step {n} of the {} runbook ({}): {}, on {}{command}. Check its preconditions first. For anything that touches production, show me exactly what it will do and wait for my go before running it. Handoff {}: report how it went with the {REPORT_TOOL} tool, done, failed or blocked once it has run, or waiting while it needs my go. It sets the step's status in the runbook; without that tool, set the status yourself.",
        slug.to_uppercase(),
        s(book, "path")?,
        s(step, "step")?,
        s(step, "where")?,
        s(&handoff, "id")?
    );
    let script = config::script("ask-session.sh").to_string_lossy().into_owned();
    let dash = |v: &str| {
        if v.is_empty() { "-".to_string() } else { v.to_string() }
    };
    // The session is Claude Code's unless the runbook's owner file names its agent.
    let agent = owner_file(book)?["agent"].as_str().unwrap_or("claude").to_string();
    proc::spawn(&[String::from("sh"), script, dash(&pane), dash(&session), prompt, agent], None, None, |_| {}).map_err(|e| e.to_string())?;
    ctx.note(format!("step {n} sent to its session{}", if session != "-" { " (resuming it)" } else { "" }));
    Ok(())
}

/// Marks a step done by hand: its status cell becomes "✅ done <day>", and a handoff still in flight says so.
pub fn mark_done(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (book, step) = (arg(args, 0)?, arg(args, 1)?);
    let (path, n, slug) = (s(book, "path")?, s(step, "n")?, s(book, "slug")?);
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?.replace("\r\n", "\n");
    let mut lines: Vec<String> = content.split('\n').map(str::to_string).collect();
    let found = lines.iter().position(|line| {
        let cells: Vec<&str> = line.split('|').collect();
        line.trim().starts_with('|') && cells.len() > 5 && cells[1].trim() == n
    });
    let Some(at) = found else {
        ctx.note(format!("step {n} not found in {slug}"));
        return Ok(());
    };
    let mut cells: Vec<String> = lines[at].split('|').map(str::to_string).collect();
    cells[4] = format!(" ✅ done {} ", chrono::Local::now().format("%d %b"));
    lines[at] = cells.join("|");
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, lines.join("\n")).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    if let Some(handoff) = read_handoffs().remove(&handoff_id(book, step)?).filter(in_flight) {
        let mut done = handoff;
        done["state"] = json!("done");
        done["message"] = json!("marked done by hand");
        done["reportedAt"] = json!(now_ms());
        write_handoff(&done)?;
    }
    ctx.note(format!("{slug} step {n} marked done"));
    Ok(())
}

pub fn open_pr(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (repo, number) = (text(arg(args, 0)?), arg(args, 1)?);
    let Some(url) = config::pr_url(&repo, number, &ctx.cfg, &config::read_inbox()) else {
        ctx.note(format!("no URL known for {repo}#{}: set github.org in the cockpit config", text(number)));
        return Ok(());
    };
    config::open_url(&url);
    ctx.note(format!("opened {url}"));
    Ok(())
}

/// Brings the tab of the session running the step into view, with its space.
pub fn focus_session(ctx: &mut Ctx, args: &[Value]) -> Res {
    let handoff = arg(args, 0)?;
    if handoff.is_null() {
        return Err("'NoneType' object has no attribute 'get'".into());
    }
    if !truthy(get(handoff, "pane")) {
        ctx.note("its session has not picked the step up yet");
        return Ok(());
    }
    let pane = s(handoff, "pane")?;
    let Some(tab) = frontend::tab_of(&pane) else {
        // The session is gone with the step: the row offers run again.
        let mut gone = handoff.clone();
        gone["state"] = json!("unreported");
        write_handoff(&gone)?;
        ctx.note("its session's pane is gone: run the step again");
        return Ok(());
    };
    let focused = match pane.starts_with(frontend::APP) {
        true => frontend::focus(&pane),
        false => proc::run(&[herdr::bin(), "tab".into(), "focus".into(), tab], Duration::from_secs(5), None).started,
    };
    if !focused {
        ctx.note("herdr did not answer: try again");
    }
    Ok(())
}

fn memory_dir(ctx: &Ctx) -> PathBuf {
    let flat: String = ctx.repos().chars().map(|c| if c == '/' || c == '.' { '-' } else { c }).collect();
    transcripts::claude_home().join("projects").join(flat).join("memory")
}

fn tools(ctx: &Ctx) -> Vec<String> {
    let mut tools = proc::strings(&["Bash(gh:*)", "Bash(git:*)", "Bash(jq:*)", "Read", "Write", "Edit", "Grep", "Glob", "mcp__cockpit__cockpit_read"]);
    for source in ["tickets", "ci", "platform"] {
        let conf = &ctx.cfg.0[source];
        if truthy(&conf["enabled"]) && truthy(&conf["server"]) {
            tools.push(config::tool_prefix(&text(&conf["server"])));
        }
    }
    tools.extend(list(&ctx.cfg.0["runbookTools"]).iter().map(text));
    tools
}

fn refresh_prompt(ctx: &Ctx, book: &Value) -> Res<String> {
    let platform = &ctx.cfg.0["platform"];
    let deploy_facts = if truthy(&platform["enabled"]) && platform["tools"].get("deployStatus").is_some() {
        format!(
            "the platform's {} tool, then gh compare of the merge commit against the deployed tag",
            s(key(key(platform, "tools")?, "deployStatus")?, "tool")?
        )
    } else {
        "from the deploy tooling the connected tools reach, then gh compare of the merge commit against the deployed ref".into()
    };
    Ok([
        format!("Update the runbook at {}: read it first, keep its step order and numbering, refresh every status from live facts, add the steps that appeared and mark the ones finished.", s(book, "path")?),
        format!("Gather the facts, never from memory alone: every PR across repos (gh: state, review, CI), whether merged ones are live in production ({deploy_facts}), the ticket status, and deploy order, migrations and post-deploy commands from the PRs, the memory notes and the code."),
        "Keep the file's format: `# <TICKET> · <feature>`, `updated: <YYYY-MM-DD HH:MM>`, one `| # | Step | Where | Status | Command |` table, statuses from ✅ done, 🚀 live, 🟡 in progress, ⏳ waiting, ⬜ to do, ❌ blocked, optional `## Notes` of three bullets.".into(),
        "cockpit's cockpit_read tool reads tickets, CI runs and deploys through cockpit's own servers (source tickets, ci or platform; verbs such as details, listRuns, deployStatus) when your own tools do not reach them.".into(),
        "Do not run any production command or deploy. Write the file, nothing else.".into(),
        format!("It is now {} local time: use it for `updated:` and any \"as of\" in a status.", chrono::Local::now().format("%Y-%m-%d %H:%M")),
    ]
    .join("\n"))
}

/// Refreshes the runbook in the background: a headless run of the agent that wrote it, forked from that conversation,
/// else of the default agent; its exit status lands in runbook-refresh-<slug>.exit for an app that only knows the pid.
pub fn update(ctx: &mut Ctx, args: &[Value]) -> Res {
    let book = arg(args, 0)?;
    let slug = s(book, "slug")?;
    if REFRESHING.lock().map_err(|e| e.to_string())?.get_or_insert_default().contains_key(&slug) {
        ctx.note(format!("{slug} is already refreshing"));
        return Ok(());
    }
    let (_, session) = owner_of(book)?;
    // The writer's session is Claude Code's unless its owner file names another agent.
    let writer = agent::get(owner_file(book)?["agent"].as_str().unwrap_or("claude")).filter(|a| a.installed() && !a.run_fork.is_empty());
    let fork = writer.as_ref().and(Some(session.as_str())).filter(|s| !s.is_empty() && *s != "-");
    let agent = writer.filter(|_| fork.is_some()).unwrap_or_else(agent::default);
    let memory = memory_dir(ctx);
    let mut dirs = vec![runbooks_dir().to_string_lossy().into_owned()];
    if memory.is_dir() {
        dirs.push(memory.to_string_lossy().into_owned());
    }
    let how = Headless { model: Some("sonnet"), fork, edits: true, dirs, system: None };
    let Some(mut command) = agent.headless(&refresh_prompt(ctx, book)?, &how) else {
        return Err(format!("{} has no headless mode: it cannot refresh a runbook", agent.name));
    };
    if agent.id == "claude" {
        command.push("--allowedTools".into());
        command.extend(tools(ctx));
    }
    let logs = files::cache();
    let status = logs.join(format!("runbook-refresh-{slug}.exit"));
    let _ = std::fs::remove_file(&status);
    let mut argv = proc::strings(&["sh", "-c", "\"$@\"; code=$?; echo \"$code\" > \"$0\"; exit \"$code\""]);
    argv.push(status.to_string_lossy().into_owned());
    argv.extend(command);
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    let log = std::fs::File::create(logs.join(format!("runbook-refresh-{slug}.log"))).map_err(|e| e.to_string())?;
    let ended = slug.clone();
    let pid = proc::spawn(&argv, Some(std::path::Path::new(&ctx.repos())), Some(log), move |_| {
        if let Ok(mut running) = REFRESHING.lock() {
            running.get_or_insert_default().remove(&ended);
        }
    })
    .map_err(|e| e.to_string())?;
    if let Ok(mut running) = REFRESHING.lock() {
        running.get_or_insert_default().insert(slug.clone(), pid);
    }
    ctx.refreshing.insert(slug.clone(), json!(pid));
    ctx.note(format!("refreshing {slug} in the background"));
    Ok(())
}

/// Talks the runbook's ticket over in the actions space, forking the session that wrote the runbook.
pub fn talk(ctx: &mut Ctx, args: &[Value]) -> Res {
    let book = arg(args, 0)?;
    let slug = s(book, "slug")?;
    let data = config::read_inbox();
    static TICKET: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^[a-z]+-\d+$").expect("a valid pattern"));
    let is_ticket = TICKET.is_match(&slug);
    let id = if is_ticket { slug.to_uppercase() } else { slug.clone() };
    let ticket = match list(&data["tickets"]).iter().find(|t| t["id"].as_str() == Some(&id)) {
        Some(ticket) => ticket.clone(),
        None => {
            let url = if id != slug { config::ticket_url(&id, &ctx.cfg, Some(&data)).map(Value::from).unwrap_or(Value::Null) } else { json!("") };
            json!({"id": id, "title": key(book, "title")?, "url": url})
        }
    };
    let prs: Vec<Value> = list(&data["mine"]).iter().filter(|pr| config::has_ticket(pr["branch"].as_str().unwrap_or_default(), &id)).cloned().collect();
    let (_, session) = owner_of(book)?;
    let owner = (session != "-").then_some(session);
    inbox::talk_over(ctx, &ticket, &prs, &data, owner.as_deref(), "")
}

pub fn delete(ctx: &mut Ctx, args: &[Value]) -> Res {
    let book = arg(args, 0)?;
    let (path, slug) = (s(book, "path")?, s(book, "slug")?);
    if ctx.confirmed(&format!("delete-{path}")) {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        ctx.note(format!("deleted {slug}"));
    } else {
        ctx.note(format!("click delete again to remove the {slug} runbook"));
    }
    Ok(())
}

/// Whether the runbook space is the one herdr shows now.
fn is_in_view() -> bool {
    let out = proc::run(&[herdr::bin(), "workspace".into(), "list".into()], Duration::from_secs(5), None);
    herdr::result_of(&out)
        .is_some_and(|r| list(&r["workspaces"]).iter().any(|w| truthy(&w["focused"]) && w["label"].as_str().is_some_and(|l| l.ends_with("  runbook"))))
}

/// A finished background refresh: a notification whose click opens the runbook and plays its new done steps. Beside
/// the slug, the app may pass the steps the refresh turned done and the reason it was asked for.
pub fn announce(_: &mut Ctx, args: &[Value]) -> Res {
    let slug = text(arg(args, 0)?);
    let Some(book) = read_books(&runbooks_dir()).into_iter().find(|b| b["slug"].as_str() == Some(&slug)) else {
        return Ok(());
    };
    if is_in_view() {
        return Ok(());
    }
    let newly: Vec<String> = list(opt(args, 1)).iter().map(text).collect();
    let reason = opt(args, 2).as_str().unwrap_or_default();
    let todo = list(&book["steps"]).iter().filter(|s| s["status"].as_str().is_some_and(|st| OPEN.iter().any(|o| st.starts_with(o)))).count();
    let mut body = if newly.is_empty() {
        format!("no change · {todo} to do")
    } else {
        format!("{} step{} done · {todo} to do", newly.len(), if newly.len() != 1 { "s" } else { "" })
    };
    if !reason.is_empty() {
        body = format!("{reason} · {body}");
    }
    let action = if newly.is_empty() { format!("runbook:{slug}") } else { format!("runbook:{slug}:{}", newly.join(",")) };
    let title = book["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(&slug).to_string();
    std::thread::spawn(move || cockpit_core::notify::post(&format!("📋 {title}"), &body, &action, &[]));
    Ok(())
}

pub fn copy(ctx: &mut Ctx, args: &[Value]) -> Res {
    let command = text(arg(args, 0)?);
    if !config::copy(&command) {
        ctx.note("no clipboard tool: install wl-copy or xclip");
        return Ok(());
    }
    ctx.note(format!("copied: {command}"));
    Ok(())
}
