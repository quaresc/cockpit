//! The routines space's actions: add or change one, run, stop, pause or delete it, edit its file or read a run's output
//! in a tab of the actions space, talk an agent's run over by resuming its session there, mark runs read, and look at or
//! clean up the worktrees runs left.

use std::path::Path;

use cockpit_core::routines::{self, When};
use serde_json::{Value, json};

use super::inbox::{Launch, start_action};
use super::space;
use super::{Ctx, arg, opt};
use crate::routines::{self as runner, HandRun, Setup, now_ms};
use crate::store::{self, Store};
use crate::value::{Res, cut, quote, text, truthy};
use cockpit_core::{agent, frontend};

fn slug(args: &[Value]) -> Res<String> {
    let slug = text(arg(args, 0)?);
    if slug.is_empty() || !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("not a routine: {slug:?}"));
    }
    Ok(slug)
}

/// The routine as its file says now.
fn routine(setup: &Setup, slug: &str) -> Res<routines::Routine> {
    let path = setup.routines.join(format!("{slug}.md"));
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no routine {slug}"))?;
    Ok(routines::Routine::parse(slug, &path, &text))
}

/// A new routine from the app's form: name, when, prompt (or script), folder, "shell" for a script else the agent's id
/// (empty for the default), its model, whether each run gets a worktree, when it notifies (always, failed, never), the
/// precheck that runs first, the time zone its times are in, and the tools and permission mode a template gives it.
pub fn add(ctx: &mut Ctx, args: &[Value]) -> Res {
    let note = create(&Setup::live(), args)?;
    ctx.note(note);
    runner::poke();
    Ok(())
}

/// The permission modes a routine runs in, as Claude Code's `--permission-mode` names them.
const MODES: [&str; 6] = ["default", "acceptEdits", "plan", "bypassPermissions", "auto", "dontAsk"];

fn create(setup: &Setup, args: &[Value]) -> Res<String> {
    let field = |n: usize| opt(args, n).as_str().unwrap_or_default().trim().to_string();
    let (when, body, cwd) = (field(1), field(2), field(3));
    if body.is_empty() {
        return Err("a routine needs a prompt".into());
    }
    let mode = field(11);
    if !mode.is_empty() && !MODES.contains(&mode.as_str()) {
        return Err(format!("no mode {mode:?}: one of {}", MODES.join(", ")));
    }
    When::parse(&when)?;
    let name = Some(field(0)).filter(|n| !n.is_empty()).unwrap_or_else(|| cut(&body, 40));
    std::fs::create_dir_all(&setup.routines).map_err(|e| e.to_string())?;
    let slug = routines::slug_for(&setup.routines, &name);
    let mut file = routines::new_file(&name, &when, &cwd, &body);
    let kind = field(4);
    if kind == "shell" {
        file = routines::set_field(&file, "run", "shell");
    } else {
        let agent = agent_field(setup, &kind)?;
        if !agent.is_empty() {
            file = routines::set_field(&file, "agent", &agent);
        }
        if !field(5).is_empty() {
            file = routines::set_field(&file, "model", &model(setup, &agent, &field(5))?);
        }
    }
    if truthy(opt(args, 6)) {
        file = routines::set_field(&file, "worktree", "true");
    }
    if let notify @ ("failed" | "never") = field(7).as_str() {
        file = routines::set_field(&file, "notify", notify);
    }
    if !field(8).is_empty() {
        file = routines::set_field(&file, "precheck", &field(8));
    }
    let tz = time_zone(&field(9))?;
    if !tz.is_empty() {
        file = routines::set_field(&file, "tz", &tz);
    }
    for (n, key) in [(10, "tools"), (11, "mode")] {
        if !field(n).is_empty() && kind != "shell" {
            file = routines::set_field(&file, key, &field(n));
        }
    }
    let path = setup.routines.join(format!("{slug}.md"));
    std::fs::write(&path, file).map_err(|e| e.to_string())?;
    Ok(format!("added {name}: e edits its file"))
}

/// A routine changed from the app's form: the slug, then the fields `add` takes; the file's other fields stay, and so do
/// its precheck and time zone when the form sends none.
pub fn update(ctx: &mut Ctx, args: &[Value]) -> Res {
    let note = rewrite(&Setup::live(), &slug(args)?, &args[1..])?;
    ctx.note(note);
    runner::poke();
    Ok(())
}

fn rewrite(setup: &Setup, slug: &str, fields: &[Value]) -> Res<String> {
    let r = routine(setup, slug)?;
    let field = |n: usize| opt(fields, n).as_str().unwrap_or_default().trim().to_string();
    let (name, when, body, cwd, shell) = (field(0), field(1), field(2), field(3), field(4) == "shell");
    if body.is_empty() {
        return Err("a routine needs a prompt".into());
    }
    When::parse(&when)?;
    let agent = if shell { String::new() } else { agent_field(setup, &field(4))? };
    let model = if shell { String::new() } else { model(setup, &agent, &field(5))? };
    let notify = match field(7).as_str() {
        n @ ("failed" | "never") => n.to_string(),
        _ => "always".to_string(),
    };
    let mut text = std::fs::read_to_string(&r.path).map_err(|e| e.to_string())?;
    let name = if name.is_empty() { r.name.clone() } else { name };
    let worktree = if truthy(opt(fields, 6)) { "true" } else { "false" };
    let precheck = opt(fields, 8).as_str().map_or(r.precheck.clone(), |p| p.trim().to_string());
    let tz = match opt(fields, 9).as_str() {
        Some(tz) => time_zone(tz)?,
        None => r.tz.clone(),
    };
    let fields =
        [("name", name.as_str()), ("when", &when), ("cwd", &cwd), ("run", if shell { "shell" } else { "agent" }), ("agent", &agent), ("model", &model)];
    for (key, value) in fields.into_iter().chain([("worktree", worktree), ("notify", notify.as_str()), ("precheck", &precheck), ("tz", &tz)]) {
        text = routines::set_field(&text, key, value);
    }
    let (head, _) = routines::front_matter(&text);
    let unset = |k: &str, v: &str| ["agent", "precheck", "tz"].contains(&k) && v.is_empty();
    let head: Vec<String> = head.iter().filter(|(k, v)| !unset(k, v)).map(|(k, v)| format!("{k}: {v}")).collect();
    std::fs::write(&r.path, format!("---\n{}\n---\n\n{}\n", head.join("\n"), body)).map_err(|e| e.to_string())?;
    Ok(format!("saved {name}"))
}

/// The `tz` field for the form's time zone: empty for the machine's own, else a zone's name, refused when it is none.
fn time_zone(text: &str) -> Res<String> {
    Ok(routines::Zone::parse(text)?.name())
}

/// The `agent` field for the form's choice: empty for the default, else a known agent's id.
fn agent_field(setup: &Setup, id: &str) -> Res<String> {
    let id = id.trim().to_lowercase();
    if id.is_empty() || id == "agent" {
        return Ok(String::new());
    }
    setup.agents.iter().find(|a| a.id == id || a.aliases.contains(&id)).map(|a| a.id.clone()).ok_or_else(|| format!("no agent {id:?}"))
}

/// A model the routine's agent takes: one of those its table names (and a routine's own, as Claude Code's lightest), or
/// a plain model name for an agent that lists its own or none.
fn model(setup: &Setup, agent: &str, name: &str) -> Res<String> {
    let name = name.trim().to_string();
    let id = if agent.is_empty() { setup.default_agent.as_str() } else { agent };
    if let Some(named) = setup.agents.iter().find(|a| a.id == id && a.model_listing().is_none() && !a.offered.is_empty()) {
        let (name, known) = (name.to_lowercase(), cockpit_core::models::for_routine(named, || {}));
        return if known.contains(&name) { Ok(name) } else { Err(format!("no model {name:?}: one of {}", known[1..].join(", "))) };
    }
    // Typed after its switch as one word: never one an agent would read as a switch of its own.
    if name.is_empty() || cockpit_core::agent::model_id(&name) { Ok(name) } else { Err(format!("no model {name:?}")) }
}

/// The routine's model: one its agent takes (`model`), or empty for the agent's default.
pub fn set_model(ctx: &mut Ctx, args: &[Value]) -> Res {
    let setup = Setup::live();
    let r = routine(&setup, &slug(args)?)?;
    let name = model(&setup, &r.agent, opt(args, 1).as_str().unwrap_or_default())?;
    let text = std::fs::read_to_string(&r.path).map_err(|e| e.to_string())?;
    std::fs::write(&r.path, routines::set_field(&text, "model", &name)).map_err(|e| e.to_string())?;
    let agent = setup.agent_of(&r).map_or("the agent".to_string(), |a| a.name.clone());
    ctx.note(format!("{} runs on {}", r.name, if name.is_empty() { format!("{agent}'s default model") } else { name }));
    runner::poke();
    Ok(())
}

pub fn run(ctx: &mut Ctx, args: &[Value]) -> Res {
    let slug = slug(args)?;
    let setup = Setup::live();
    let note = run_by_hand(&mut store::global(), &setup, &slug, now_ms())?;
    ctx.note(note);
    Ok(())
}

/// A routine run by hand, refused when its file cannot run: what happened, in words.
fn run_by_hand(store: &mut Store, setup: &Setup, slug: &str, now: u64) -> Res<String> {
    let r = routine(setup, slug)?;
    if let Some(why) = r.broken() {
        return Err(format!("{} cannot run: {why}", r.name));
    }
    match runner::run_now(store, setup, slug, now, &mut runner::post) {
        HandRun::Started => Ok(format!("started {}", r.name)),
        HandRun::Queued => Ok(format!("{} waits for a free slot", r.name)),
        HandRun::Busy => Ok(format!("{} is already running", r.name)),
        HandRun::OverBudget(why) | HandRun::Failed(why) => Err(format!("{}: {why}", r.name)),
    }
}

pub fn stop(ctx: &mut Ctx, args: &[Value]) -> Res {
    let slug = slug(args)?;
    let mut stopped = false;
    runner::change(&mut store::global(), now_ms(), |doc| {
        stopped = runner::stop(doc, &slug, now_ms());
        if let Some(queued) = doc["queued"].as_array_mut() {
            let before = queued.len();
            queued.retain(|q| q["slug"] != slug.as_str());
            stopped |= queued.len() < before;
        }
    });
    runner::poke();
    ctx.note(if stopped { format!("stopping {slug}") } else { format!("{slug} is not running") });
    Ok(())
}

/// Pauses a routine, or resumes it from now on: what it missed while paused does not run.
pub fn pause(ctx: &mut Ctx, args: &[Value]) -> Res {
    let slug = slug(args)?;
    let paused = opt(args, 1).as_bool().unwrap_or(true);
    let setup = Setup::live();
    let note = set_paused(&mut store::global(), &setup, &slug, paused, now_ms())?;
    ctx.note(note);
    runner::poke();
    Ok(())
}

fn set_paused(store: &mut Store, setup: &Setup, slug: &str, paused: bool, now: u64) -> Res<String> {
    let r = routine(setup, slug)?;
    let text = std::fs::read_to_string(&r.path).map_err(|e| e.to_string())?;
    std::fs::write(&r.path, routines::set_field(&text, "paused", if paused { "true" } else { "false" })).map_err(|e| e.to_string())?;
    if !paused {
        runner::change(store, now, |doc| doc["marks"][slug]["fired"] = json!(now));
    }
    Ok(format!("{} {}", if paused { "paused" } else { "resumed" }, r.name))
}

pub fn delete(ctx: &mut Ctx, args: &[Value]) -> Res {
    let slug = slug(args)?;
    if !ctx.confirmed(&format!("delete-routine-{slug}")) {
        ctx.note(format!("press delete again to remove the routine {slug}"));
        return Ok(());
    }
    let setup = Setup::live();
    remove(&mut store::global(), &setup, &slug, now_ms())?;
    ctx.note(format!("deleted {slug}"));
    Ok(())
}

fn remove(store: &mut Store, setup: &Setup, slug: &str, now: u64) -> Res {
    let r = routine(setup, slug)?;
    std::fs::remove_file(&r.path).map_err(|e| e.to_string())?;
    runner::change(store, now, |doc| {
        runner::stop(doc, slug, now);
        runner::forget(doc, setup, slug);
    });
    runner::round(store, setup, now, &mut runner::post);
    Ok(())
}

/// A command in a new tab of the actions space, else in a new space.
fn in_tab(ctx: &mut Ctx, cwd: &str, label: &str, line: &str) -> Res {
    if let Some(pane) = space::pane_of(label)
        && space::focus(&pane)
    {
        ctx.note(format!("\"{label}\" is already open: focused its tab"));
        return Ok(());
    }
    let pane = space::open_tab(&ctx.cfg, cwd, label, Some(label)).or_else(|| frontend::new_space(cwd, label));
    let Some(pane) = pane else { return Err("herdr opened no tab".into()) };
    frontend::run_in(&pane, line);
    Ok(())
}

/// The routine's file in $EDITOR, in a tab of the actions space.
pub fn edit(ctx: &mut Ctx, args: &[Value]) -> Res {
    let r = routine(&Setup::live(), &slug(args)?)?;
    let path = r.path.to_string_lossy().into_owned();
    in_tab(ctx, &routines::dir().to_string_lossy(), &format!("✎ {}", cut(&r.name, 28)), &format!("${{EDITOR:-vi}} {}", quote(&path)))?;
    ctx.note(format!("editing {}", r.name));
    Ok(())
}

/// A run's whole output in the pager, in a tab of the actions space: the given run, else the last one.
pub fn output(ctx: &mut Ctx, args: &[Value]) -> Res {
    let slug = slug(args)?;
    let setup = Setup::live();
    let run = run_of(&store::global().get(runner::DOC), &slug, opt(args, 1))?;
    mark_read(&mut store::global(), Some(&slug), run["id"].as_str(), now_ms());
    let path = runner::output(&setup, &slug, run["id"].as_str().unwrap_or_default());
    if !Path::new(&path).is_file() {
        return Err(format!("{slug}'s run left no output"));
    }
    let shown = path.to_string_lossy().into_owned();
    in_tab(ctx, &setup.runs.to_string_lossy(), &format!("⟳ {slug} output"), &format!("${{PAGER:-less}} {}", quote(&shown)))?;
    Ok(())
}

/// The run of this id, else the last one.
fn run_of(doc: &Value, slug: &str, id: &Value) -> Res<Value> {
    let runs = doc["runs"][slug].as_array().cloned().unwrap_or_default();
    let found = match id.as_str() {
        Some(id) => runs.into_iter().find(|r| r["id"] == id),
        None => runs.into_iter().next(),
    };
    found.ok_or_else(|| format!("{slug} has not run yet"))
}

/// An agent run's session resumed in a tab of the actions space, to ask it what it did or carry on: the given run, else
/// the last one.
pub fn talk(ctx: &mut Ctx, args: &[Value]) -> Res {
    let slug = slug(args)?;
    let setup = Setup::live();
    let doc = store::global().get(runner::DOC);
    let run = run_of(&doc, &slug, opt(args, 1))?;
    mark_read(&mut store::global(), Some(&slug), run["id"].as_str(), now_ms());
    let Some(session) = run["session"].as_str().filter(|s| !s.is_empty()) else {
        ctx.note(format!("{slug}'s run left no agent session to talk to"));
        return Ok(());
    };
    // A run from before runs named their agent was Claude Code's.
    let Some(agent) = agent::get(run["agent"].as_str().unwrap_or("claude")) else {
        return Err(format!("{slug} ran an agent cockpit no longer knows"));
    };
    if agent.resume.is_empty() {
        ctx.note(format!("{} cannot resume a session: talk to it yourself", agent.name));
        return Ok(());
    }
    let r = routine(&setup, &slug).ok();
    let name = r.as_ref().map_or(slug.clone(), |r| r.name.clone());
    let cwd = r.map(|r| r.cwd).filter(|c| !c.is_empty()).unwrap_or_else(|| ctx.repos());
    let prompt = "Where did this run get to? Sum it up in a few lines, then wait for me.";
    start_action(ctx, &cwd, &format!("⟳ {}", cut(&name, 28)), prompt, &Launch::resume(agent, session));
    Ok(())
}

/// Marks runs read, as the run inbox has them: this routine's (only this run of it, given its id).
pub fn read(ctx: &mut Ctx, args: &[Value]) -> Res {
    let slug = slug(args)?;
    mark_read(&mut store::global(), Some(&slug), opt(args, 1).as_str(), now_ms());
    ctx.note("");
    Ok(())
}

/// Every run read: the inbox empties.
pub fn read_all(ctx: &mut Ctx, _: &[Value]) -> Res {
    let n = mark_read(&mut store::global(), None, None, now_ms());
    ctx.note(if n == 1 { "1 run marked read".to_string() } else { format!("{n} runs marked read") });
    Ok(())
}

/// The runs read, of every routine or one, every run of it or one: how many were not yet.
fn mark_read(store: &mut Store, slug: Option<&str>, id: Option<&str>, now: u64) -> usize {
    let mut marked = 0;
    runner::change(store, now, |doc| {
        for (s, runs) in doc["runs"].as_object_mut().into_iter().flatten() {
            if slug.is_some_and(|want| want != s) {
                continue;
            }
            for run in runs.as_array_mut().into_iter().flatten() {
                if id.is_some_and(|want| run["id"] != want) || run["read"] == true {
                    continue;
                }
                run["read"] = json!(true);
                marked += 1;
            }
        }
    });
    marked
}

/// A run's worktree in a tab of the actions space, its commits and changes listed.
pub fn open_worktree(ctx: &mut Ctx, args: &[Value]) -> Res {
    let slug = slug(args)?;
    let run = run_of(&store::global().get(runner::DOC), &slug, opt(args, 1))?;
    let wt = &run["worktree"];
    let path = wt["path"].as_str().filter(|p| Path::new(p).is_dir()).ok_or_else(|| format!("{slug}'s run left no worktree"))?;
    let line = format!("git log --oneline --stat {}..HEAD; git status --short", wt["base"].as_str().unwrap_or("HEAD"));
    in_tab(ctx, path, &format!("⎇ {}", wt["branch"].as_str().unwrap_or(&slug)), &line)?;
    Ok(())
}

/// Removes run worktrees, never by force: given a run, its own; else every one nothing keeps (`cockpit_core::cleanup`'s
/// rules), the note naming what keeps the others.
pub fn cleanup(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (only, id) = (opt(args, 0).as_str().map(str::to_string), opt(args, 1).as_str().map(str::to_string));
    // Irreversible: asked first, as the inbox's removal is (`remove-run-<slug>-<id>`, or `cleanup-runs` for them all).
    let armed = match (&only, &id) {
        (Some(slug), Some(id)) => format!("remove-run-{slug}-{id}"),
        _ => "cleanup-runs".to_string(),
    };
    if !ctx.confirmed(&armed) {
        return Ok(());
    }
    let note = clean(&mut store::global(), only.as_deref(), id.as_deref(), now_ms())?;
    ctx.note(note);
    Ok(())
}

fn clean(store: &mut Store, only: Option<&str>, id: Option<&str>, now: u64) -> Res<String> {
    let doc = store.get(runner::DOC);
    let (mut removed, mut kept, mut errors) = (Vec::new(), Vec::new(), Vec::new());
    let facts = cockpit_core::cleanup::Facts::now();
    for (slug, runs) in doc["runs"].as_object().into_iter().flatten() {
        for run in runs.as_array().into_iter().flatten() {
            let wt = &run["worktree"];
            let picked = only.is_none_or(|s| s == slug) && id.is_none_or(|i| run["id"] == i);
            if wt.is_null() || wt["removed"] == true || !picked || doc["running"][slug]["id"] == run["id"] {
                continue;
            }
            // A run picked by hand is held by the same rules as all the others.
            let blockers = runner::kept_by(wt, &facts);
            if !blockers.is_empty() {
                kept.push(format!("{} ({})", text(&wt["branch"]), cockpit_core::cleanup::said(&blockers)));
                continue;
            }
            match runner::remove_worktree(wt) {
                Ok(()) => removed.push((slug.clone(), run["id"].clone())),
                Err(err) if !Path::new(wt["path"].as_str().unwrap_or_default()).is_dir() => {
                    removed.push((slug.clone(), run["id"].clone()));
                    errors.push(format!("branch {} kept: {err}", text(&wt["branch"])));
                }
                Err(err) => errors.push(err),
            }
        }
    }
    runner::change(store, now, |doc| {
        for (slug, id) in &removed {
            for run in doc["runs"][slug].as_array_mut().into_iter().flatten().filter(|r| &r["id"] == id) {
                run["worktree"]["removed"] = json!(true);
            }
        }
    });
    if id.is_some() && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    if id.is_some() && !kept.is_empty() {
        return Err(format!("kept {}", kept.join("; ")));
    }
    let mut note = match removed.len() {
        0 => "no run worktree to remove".to_string(),
        1 => "removed 1 run worktree".to_string(),
        n => format!("removed {n} run worktrees"),
    };
    if !kept.is_empty() {
        note.push_str(&format!(" · {} kept: {}", kept.len(), kept.join("; ")));
    }
    Ok(note)
}

pub fn request_refresh(ctx: &mut Ctx, _: &[Value]) -> Res {
    runner::poke();
    ctx.note("reading the routines again");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::*;

    fn world() -> (tempfile::TempDir, Store, Setup) {
        let dir = tempfile::tempdir().unwrap();
        let setup = Setup {
            routines: dir.path().join("routines"),
            runs: dir.path().join("runs"),
            agents: agent::builtin(),
            default_agent: "claude".into(),
            repos: dir.path().to_string_lossy().into_owned(),
            worktrees: dir.path().join("worktrees"),
            settling: false,
            slept_ms: 0,
        };
        let store = Store::open(&dir.path().join("cache"), Duration::ZERO);
        (dir, store, setup)
    }

    #[test]
    fn a_discovered_model_id_keeps_its_case_and_version() {
        let (_dir, _store, setup) = world();
        assert_eq!(model(&setup, "opencode", "Acme/Large@stable").unwrap(), "Acme/Large@stable");
        assert!(model(&setup, "opencode", "acme/model;touch x").is_err());
        for switch in ["-p", "--dangerously-skip-permissions", &"m".repeat(200)] {
            assert!(model(&setup, "opencode", switch).is_err(), "{switch:?} is no model");
        }
        assert_eq!(model(&setup, "opencode", " ").unwrap(), "", "empty: the agent's default");
    }

    #[test]
    fn a_routine_added_from_the_form_is_a_file_named_after_it() {
        let (_dir, _, setup) = world();
        let args = [json!("Morning sweep"), json!("weekdays 09:00"), json!("Sum up my PRs."), json!("~/work"), json!(""), json!("lightest")];
        assert_eq!(create(&setup, &args).unwrap(), "added Morning sweep: e edits its file");
        let text = std::fs::read_to_string(setup.routines.join("morning-sweep.md")).unwrap();
        assert_eq!(text, "---\nname: Morning sweep\nwhen: weekdays 09:00\ncwd: ~/work\nmodel: lightest\n---\n\nSum up my PRs.\n");
        assert!(create(&setup, &[json!("x"), json!("hourly"), json!("y"), json!(""), json!(""), json!("gpt")]).unwrap_err().starts_with("no model"));
        let script = [json!(""), json!("hourly"), json!("git -C ~/notes pull"), json!(""), json!("shell")];
        create(&setup, &script).unwrap();
        let r = routine(&setup, "git-c-notes-pull").unwrap();
        assert_eq!((r.run, r.name.as_str()), (routines::Run::Shell, "git -C ~/notes pull"));
        assert_eq!(create(&setup, &[json!("x"), json!("sometimes"), json!("y")]).unwrap_err().split('?').next(), Some("when"));
        assert_eq!(create(&setup, &[json!("x"), json!("hourly"), json!(" ")]).unwrap_err(), "a routine needs a prompt");
        create(&setup, &[json!("On codex"), json!("hourly"), json!("Go."), json!(""), json!("codex"), json!("gpt-5-mini")]).unwrap();
        let r = routine(&setup, "on-codex").unwrap();
        assert_eq!((r.run, r.agent.as_str(), r.model.as_str()), (routines::Run::Agent, "codex", "gpt-5-mini"));
        assert_eq!(create(&setup, &[json!("x"), json!("hourly"), json!("y"), json!(""), json!("nobody")]).unwrap_err(), "no agent \"nobody\"");
        let moded = |mode: &str| {
            [json!("Moded"), json!("hourly"), json!("Go."), json!(""), json!(""), json!(""), json!(""), json!(""), json!(""), json!(""), json!(""), json!(mode)]
        };
        assert!(create(&setup, &moded("yolo --dangerously-skip-permissions")).unwrap_err().starts_with("no mode"), "only a mode the agent takes is stored");
        create(&setup, &moded("plan")).unwrap();
        assert_eq!(routine(&setup, "moded").unwrap().mode, "plan");
    }

    #[test]
    fn resuming_a_routine_skips_what_it_missed_and_deleting_forgets_it() {
        let (_dir, mut store, setup) = world();
        create(&setup, &[json!("tick"), json!("every 30m"), json!("true"), json!(""), json!("shell")]).unwrap();
        runner::round(&mut store, &setup, 1_000, &mut |_, _, _, _| {});
        assert_eq!(set_paused(&mut store, &setup, "tick", true, 2_000).unwrap(), "paused tick");
        assert!(routine(&setup, "tick").unwrap().paused);
        set_paused(&mut store, &setup, "tick", false, 50_000_000).unwrap();
        runner::round(&mut store, &setup, 50_000_001, &mut |_, _, _, _| {});
        let doc = store.get(runner::DOC);
        assert_eq!((&doc["routines"][0]["next"], doc["runs"]["tick"].is_null()), (&json!(50_000_000 + 1_800_000), true));
        remove(&mut store, &setup, "tick", 50_000_002).unwrap();
        let doc = store.get(runner::DOC);
        assert!(doc["routines"].as_array().unwrap().is_empty() && doc["marks"]["tick"].is_null());
        assert!(!setup.routines.join("tick.md").exists());
    }

    #[test]
    fn a_routine_changed_from_the_form_keeps_its_other_fields() {
        let (_dir, _, setup) = world();
        create(&setup, &[json!("Sweep"), json!("hourly"), json!("Sum up."), json!(""), json!(""), json!("haiku"), json!(true), json!("failed")]).unwrap();
        let path = setup.routines.join("sweep.md");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("worktree: true\nnotify: failed\n"), "{text}");
        std::fs::write(&path, routines::set_field(&text, "tools", "Read")).unwrap();
        let fields = [json!("Sweep PRs"), json!("weekdays 09:00"), json!("Sum up my PRs."), json!("~/w"), json!(""), json!("sonnet"), json!(false), json!("")];
        assert_eq!(rewrite(&setup, "sweep", &fields).unwrap(), "saved Sweep PRs");
        let r = routine(&setup, "sweep").unwrap();
        assert_eq!((r.name.as_str(), r.when_text.as_str(), r.body.as_str(), r.model.as_str()), ("Sweep PRs", "weekdays 09:00", "Sum up my PRs.", "sonnet"));
        assert_eq!((r.worktree, r.notify, r.tools.clone()), (false, routines::Notify::Always, vec!["Read".to_string()]));
        assert!(rewrite(&setup, "sweep", &[json!("x"), json!("when?"), json!("y")]).is_err());
        assert!(!std::fs::read_to_string(&path).unwrap().contains("agent:"), "the default agent writes no field");
        let fields = [json!("Sweep PRs"), json!("weekdays 09:00"), json!("Sum up my PRs."), json!("~/w"), json!("codex"), json!(""), json!(false), json!("")];
        rewrite(&setup, "sweep", &fields).unwrap();
        assert_eq!(routine(&setup, "sweep").unwrap().agent, "codex");
    }

    #[test]
    fn a_precheck_and_a_time_zone_go_in_the_file_and_stay_when_the_form_sends_none() {
        let (_dir, _, setup) = world();
        let fields = |tz: &str| {
            [
                json!("Reviews"),
                json!("weekdays 09:00"),
                json!("Sum up."),
                json!(""),
                json!(""),
                json!(""),
                json!(false),
                json!(""),
                json!("test -s ~/q"),
                json!(tz),
            ]
        };
        create(&setup, &fields("Europe/Paris")).unwrap();
        let r = routine(&setup, "reviews").unwrap();
        assert_eq!((r.precheck.as_str(), r.tz.as_str()), ("test -s ~/q", "Europe/Paris"));
        assert_eq!(create(&setup, &fields("Paris")).unwrap_err(), "\"Paris\" is not a time zone: a name like Europe/Paris, or local");
        // An older form sends eight fields: the precheck and the time zone stay as they were.
        rewrite(&setup, "reviews", &fields("Europe/Paris")[..8]).unwrap();
        let r = routine(&setup, "reviews").unwrap();
        assert_eq!((r.precheck.as_str(), r.tz.as_str()), ("test -s ~/q", "Europe/Paris"));
        let mut cleared = fields("local").to_vec();
        cleared[8] = json!("");
        rewrite(&setup, "reviews", &cleared).unwrap();
        let text = std::fs::read_to_string(setup.routines.join("reviews.md")).unwrap();
        assert!(!text.contains("precheck:") && !text.contains("tz:"), "{text}");
    }

    #[test]
    fn a_template_saved_from_the_form_keeps_its_tools_and_precheck() {
        let (_dir, _, setup) = world();
        let t = routines::TEMPLATES[0];
        let fields = [t.name, t.when, t.body, "", "", t.model, "", t.notify, t.precheck, "", t.tools, t.mode].map(|f| json!(f));
        create(&setup, &fields).unwrap();
        let r = routine(&setup, "morning-sweep").unwrap();
        assert_eq!((r.tools.len(), r.precheck.as_str(), r.model.as_str()), (3, t.precheck, "sonnet"));
        assert_eq!(r.when_shown(), "weekdays at 09:00");
    }

    #[test]
    fn the_cleanup_template_removes_only_merged_idle_worktrees_and_says_when_it_could_not() {
        let (dir, _, _) = world();
        let repos = dir.path().join("repos");
        let repo = repos.join("api");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |at: &Path, args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(at).args(["-c", "user.email=a@b", "-c", "user.name=a"]).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        let t = routines::TEMPLATES.iter().find(|t| t.id == "worktree-cleanup").unwrap();
        // gh answers as the test says, never with anyone's account: the commit a merged pull request had, else nothing.
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let gh = bin.join("gh");
        let says = |oid: &str| {
            std::fs::write(&gh, format!("#!/bin/sh\necho '{oid}'\n")).unwrap();
            assert!(Command::new("chmod").arg("+x").arg(&gh).status().unwrap().success());
        };
        says("");
        let path_var = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        let clean = || {
            let out = Command::new("sh").args(["-c", t.body]).current_dir(&repos).env("PATH", &path_var).output().unwrap();
            (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned())
        };
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join(".gitignore"), "*.env\n").unwrap();
        git(&repo, &["add", ".gitignore"]);
        git(&repo, &["commit", "-q", "-m", "start"]);
        git(&repo, &["remote", "add", "origin", "https://example.com/acme/api.git"]);
        let (ok, said) = clean();
        assert!(!ok && said.contains("its default branch is unknown"), "nothing is known merged without it: {said}");
        git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        git(&repo, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
        let path = |name: &str| dir.path().join(name);
        let add = |branch: &str, name: &str, work: bool| {
            git(&repo, &["worktree", "add", "-q", "-b", branch, &path(name).to_string_lossy()]);
            // Each its own message: two empty commits on one parent in one second would be the same commit.
            if work {
                git(&path(name), &["commit", "-q", "--allow-empty", "-m", branch]);
            }
        };
        add("merged", "wt merged", true);
        git(&repo, &["merge", "-q", "--ff-only", "merged"]);
        add("fresh", "wt-fresh", false);
        add("open", "wt-open", true);
        add("dirty", "wt-dirty", true);
        git(&repo, &["merge", "-q", "--ff-only", "dirty"]);
        std::fs::write(path("wt-dirty").join(".gitignore"), "changed\n").unwrap();
        add("squashed", "wt-squashed", true);
        git(&repo, &["config", "branch.squashed.remote", "origin"]);
        git(&repo, &["config", "branch.squashed.merge", "refs/heads/squashed"]);
        add("ignored", "wt-ignored", true);
        git(&repo, &["merge", "-q", "--ff-only", "ignored"]);
        std::fs::write(path("wt-ignored").join("secrets.env"), "token\n").unwrap();
        add("recent", "wt-recent", true);
        git(&repo, &["merge", "-q", "--ff-only", "recent"]);
        git(&repo, &["update-ref", "refs/remotes/origin/main", "main"]);
        for name in ["wt merged", "wt-fresh", "wt-open", "wt-dirty", "wt-squashed", "wt-ignored"] {
            assert!(Command::new("find").arg(path(name)).args(["-exec", "touch", "-t", "202601010000", "{}", "+"]).status().unwrap().success());
        }
        let (ok, said) = clean();
        assert!(!ok, "merged work kept unsaved needs a look: {said}");
        assert!(said.contains("wt-dirty: 1 uncommitted change"), "said as cockpit's cleanup says it: {said}");
        assert!(!path("wt merged").exists(), "merged: {said}");
        assert!(path("wt-squashed").exists(), "a branch merely gone is no merge: {said}");
        for kept in ["wt-fresh", "wt-open", "wt-dirty", "wt-ignored", "wt-recent"] {
            assert!(path(kept).exists(), "{kept} stays: {said}");
        }
        assert!(said.contains("wt-ignored: 1 ignored file other than builds") && said.contains("wt-recent: changed in the last 3 days"), "{said}");
        assert!(repo.join(".git").exists(), "the checkout itself stays");
        // GitHub says the squashed one merged at the commit it has.
        let tip = Command::new("git").arg("-C").arg(path("wt-squashed")).args(["rev-parse", "HEAD"]).output().unwrap();
        says(String::from_utf8_lossy(&tip.stdout).trim());
        let (_, said) = clean();
        assert!(!path("wt-squashed").exists(), "merged on GitHub at its own commit: {said}");
        // The agent the hub's agents document has in a worktree is named as the agent table names it, whatever order the
        // row's fields come in.
        add("by-codex", "wt-codex", true);
        git(&repo, &["merge", "-q", "--ff-only", "by-codex"]);
        git(&repo, &["update-ref", "refs/remotes/origin/main", "main"]);
        assert!(Command::new("find").arg(path("wt-codex")).args(["-exec", "touch", "-t", "202601010000", "{}", "+"]).status().unwrap().success());
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".cache/cockpit")).unwrap();
        let cwd = std::fs::canonicalize(path("wt-codex")).unwrap();
        let row = json!({"app:t9": {"at": now_ms(), "agent": "codex", "cwd": cwd}, "w2:p1": {"cwd": "/w/elsewhere", "at": now_ms()}});
        std::fs::write(home.join(".cache/cockpit/agents.json"), row.to_string()).unwrap();
        let out = Command::new("sh").args(["-c", t.body]).current_dir(&repos).env("PATH", &path_var).env("HOME", &home).output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(said.contains("wt-codex: Codex still runs in it"), "{said}");
    }

    #[test]
    fn a_run_worktree_picked_by_hand_is_asked_first_and_held_by_the_same_rules() {
        let (dir, mut store, _) = world();
        let repo = dir.path().join("api");
        let wt = dir.path().join("wt-run");
        let git = |at: &Path, args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(at).args(["-c", "user.email=a@b", "-c", "user.name=a"]).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "start"]);
        git(&repo, &["worktree", "add", "-q", "-b", "auto/r-1", &wt.to_string_lossy()]);
        std::fs::write(wt.join("draft.md"), "draft").unwrap();
        let tree = json!({"path": wt, "repo": repo, "branch": "auto/r-1", "base": "main"});
        store.update(runner::DOC, |_| json!({"version": 1, "runs": {"r": [{"id": "1", "worktree": tree}]}}));
        // Not asked first, nothing is done.
        let mut ctx = Ctx::new(None, std::collections::HashMap::new());
        cleanup(&mut ctx, &[json!("r"), json!("1")]).unwrap();
        assert!(ctx.note.is_empty() && wt.exists());
        let refused = clean(&mut store, Some("r"), Some("1"), 5).unwrap_err();
        assert!(refused.starts_with("kept auto/r-1 (1 uncommitted change"), "{refused}");
        assert!(wt.join("draft.md").is_file(), "never forced");
        std::fs::remove_file(wt.join("draft.md")).unwrap();
        assert_eq!(clean(&mut store, Some("r"), Some("1"), 6).unwrap(), "removed 1 run worktree");
        assert!(!wt.exists());
    }

    #[test]
    fn a_routine_that_cannot_run_is_refused_by_hand_too() {
        let (_dir, mut store, setup) = world();
        create(&setup, &[json!("Nightly"), json!("daily 02:00"), json!("true"), json!(""), json!("shell")]).unwrap();
        let path = setup.routines.join("nightly.md");
        std::fs::write(&path, routines::set_field(&std::fs::read_to_string(&path).unwrap(), "tz", "Mars/Olympus")).unwrap();
        let err = run_by_hand(&mut store, &setup, "nightly", 1_000).unwrap_err();
        assert_eq!(err, "Nightly cannot run: tz: \"Mars/Olympus\" is not a time zone: a name like Europe/Paris, or local");
        assert!(store.get(runner::DOC)["queued"].as_array().is_none_or(Vec::is_empty), "nothing queued");
    }

    #[test]
    fn runs_are_marked_read_one_routine_or_all_at_once() {
        let (_dir, mut store, _) = world();
        store.update(runner::DOC, |_| json!({"version": 1, "runs": {"a": [{"id": "2"}, {"id": "1", "read": true}], "b": [{"id": "3"}]}}));
        assert_eq!(mark_read(&mut store, Some("a"), Some("2"), 5), 1);
        assert_eq!(mark_read(&mut store, None, None, 6), 1);
        let doc = store.get(runner::DOC);
        assert!([&doc["runs"]["a"][0]["read"], &doc["runs"]["b"][0]["read"]].iter().all(|r| **r == json!(true)));
        assert_eq!(clean(&mut store, None, None, 7).unwrap(), "no run worktree to remove");
    }

    #[test]
    fn a_run_is_found_by_id_else_the_last() {
        let doc = json!({"runs": {"s": [{"id": "2"}, {"id": "1"}]}});
        assert_eq!(run_of(&doc, "s", &Value::Null).unwrap()["id"], "2");
        assert_eq!(run_of(&doc, "s", &json!("1")).unwrap()["id"], "1");
        assert_eq!(run_of(&doc, "t", &Value::Null).unwrap_err(), "t has not run yet");
    }
}
