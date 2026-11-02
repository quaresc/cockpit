//! The routines space's actions: add or change one, run, stop, pause or delete it, edit its file or read a run's output
//! in a tab of the actions space, talk a Claude run over by resuming its session there, mark runs read, and look at or
//! clean up the worktrees runs left.

use std::path::Path;

use cockpit_core::routines::{self, When};
use serde_json::{Value, json};

use super::inbox::start_action;
use super::space;
use super::{Ctx, arg, opt};
use cockpit_core::frontend;
use crate::routines::{self as runner, Setup, now_ms};
use crate::store::{self, Store};
use crate::value::{Res, cut, quote, text, truthy};

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

/// A new routine from the app's form: name, when, prompt (or script), folder, "shell" for a script, its model, whether
/// each run gets a worktree, and when it notifies (always, failed, never).
pub fn add(ctx: &mut Ctx, args: &[Value]) -> Res {
    let note = create(&Setup::live(), args)?;
    ctx.note(note);
    runner::poke();
    Ok(())
}

fn create(setup: &Setup, args: &[Value]) -> Res<String> {
    let field = |n: usize| opt(args, n).as_str().unwrap_or_default().trim().to_string();
    let (when, body, cwd) = (field(1), field(2), field(3));
    if body.is_empty() {
        return Err("a routine needs a prompt".into());
    }
    When::parse(&when)?;
    let name = Some(field(0)).filter(|n| !n.is_empty()).unwrap_or_else(|| cut(&body, 40));
    std::fs::create_dir_all(&setup.routines).map_err(|e| e.to_string())?;
    let slug = routines::slug_for(&setup.routines, &name);
    let mut file = routines::new_file(&name, &when, &cwd, &body);
    if field(4) == "shell" {
        file = routines::set_field(&file, "run", "shell");
    } else if !field(5).is_empty() {
        file = routines::set_field(&file, "model", &model(&field(5))?);
    }
    if truthy(opt(args, 6)) {
        file = routines::set_field(&file, "worktree", "true");
    }
    if let notify @ ("failed" | "never") = field(7).as_str() {
        file = routines::set_field(&file, "notify", notify);
    }
    let path = setup.routines.join(format!("{slug}.md"));
    std::fs::write(&path, file).map_err(|e| e.to_string())?;
    Ok(format!("added {name}: e edits its file"))
}

/// A routine changed from the app's form: the slug, then the fields `add` takes; the file's other fields stay.
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
    let model = if shell { String::new() } else { model(&field(5))? };
    let notify = match field(7).as_str() {
        n @ ("failed" | "never") => n.to_string(),
        _ => "always".to_string(),
    };
    let mut text = std::fs::read_to_string(&r.path).map_err(|e| e.to_string())?;
    let name = if name.is_empty() { r.name.clone() } else { name };
    let worktree = if truthy(opt(fields, 6)) { "true" } else { "false" };
    for (key, value) in [("name", name.as_str()), ("when", &when), ("cwd", &cwd), ("run", if shell { "shell" } else { "claude" }), ("model", &model), ("worktree", worktree), ("notify", &notify)] {
        text = routines::set_field(&text, key, value);
    }
    let (head, _) = routines::front_matter(&text);
    let head: Vec<String> = head.iter().map(|(k, v)| format!("{k}: {v}")).collect();
    std::fs::write(&r.path, format!("---\n{}\n---\n\n{}\n", head.join("\n"), body)).map_err(|e| e.to_string())?;
    Ok(format!("saved {name}"))
}

/// One of the models a routine picks from.
fn model(name: &str) -> Res<String> {
    let name = name.trim().to_lowercase();
    if routines::MODELS.contains(&name.as_str()) { Ok(name) } else { Err(format!("no model {name:?}: one of lightest, haiku, sonnet, opus")) }
}

/// The routine's model: lightest, haiku, sonnet, opus, or empty for Claude's default.
pub fn set_model(ctx: &mut Ctx, args: &[Value]) -> Res {
    let r = routine(&Setup::live(), &slug(args)?)?;
    let name = model(opt(args, 1).as_str().unwrap_or_default())?;
    let text = std::fs::read_to_string(&r.path).map_err(|e| e.to_string())?;
    std::fs::write(&r.path, routines::set_field(&text, "model", &name)).map_err(|e| e.to_string())?;
    ctx.note(format!("{} runs on {}", r.name, if name.is_empty() { "Claude's default model" } else { &name }));
    runner::poke();
    Ok(())
}

pub fn run(ctx: &mut Ctx, args: &[Value]) -> Res {
    let slug = slug(args)?;
    let setup = Setup::live();
    let name = routine(&setup, &slug)?.name;
    let started = runner::run_now(&mut store::global(), &setup, &slug, now_ms(), &mut runner::post);
    ctx.note(if started { format!("started {name}") } else { format!("{name} is already running") });
    Ok(())
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

/// A Claude run's session resumed in a tab of the actions space, to ask it what it did or carry on: the given run, else
/// the last one.
pub fn talk(ctx: &mut Ctx, args: &[Value]) -> Res {
    let slug = slug(args)?;
    let setup = Setup::live();
    let doc = store::global().get(runner::DOC);
    let run = run_of(&doc, &slug, opt(args, 1))?;
    mark_read(&mut store::global(), Some(&slug), run["id"].as_str(), now_ms());
    let Some(session) = run["session"].as_str().filter(|s| !s.is_empty()) else {
        ctx.note(format!("{slug} runs a script: no Claude session to talk to"));
        return Ok(());
    };
    let r = routine(&setup, &slug).ok();
    let name = r.as_ref().map_or(slug.clone(), |r| r.name.clone());
    let cwd = r.map(|r| r.cwd).filter(|c| !c.is_empty()).unwrap_or_else(|| ctx.repos());
    let flags = vec!["--resume".to_string(), session.to_string()];
    start_action(ctx, &cwd, &format!("⟳ {}", cut(&name, 28)), "Where did this run get to? Sum it up in a few lines, then wait for me.", &flags);
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

/// Removes run worktrees, never by force: given a run, its own; else every one whose work landed (nothing uncommitted,
/// its branch merged).
pub fn cleanup(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (only, id) = (opt(args, 0).as_str().map(str::to_string), opt(args, 1).as_str().map(str::to_string));
    let note = clean(&mut store::global(), only.as_deref(), id.as_deref(), now_ms())?;
    ctx.note(note);
    Ok(())
}

fn clean(store: &mut Store, only: Option<&str>, id: Option<&str>, now: u64) -> Res<String> {
    let doc = store.get(runner::DOC);
    let (mut removed, mut kept, mut errors) = (Vec::new(), 0, Vec::new());
    for (slug, runs) in doc["runs"].as_object().into_iter().flatten() {
        for run in runs.as_array().into_iter().flatten() {
            let wt = &run["worktree"];
            let picked = only.is_none_or(|s| s == slug) && id.is_none_or(|i| run["id"] == i);
            if wt.is_null() || wt["removed"] == true || !picked || doc["running"][slug]["id"] == run["id"] {
                continue;
            }
            if id.is_none() && !runner::landed(wt) {
                kept += 1;
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
    let mut note = match removed.len() {
        0 => "no run worktree to remove".to_string(),
        1 => "removed 1 run worktree".to_string(),
        n => format!("removed {n} run worktrees"),
    };
    if kept > 0 {
        note.push_str(&format!(" · {kept} kept: their work has not landed"));
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
    use std::time::Duration;

    use super::*;

    fn world() -> (tempfile::TempDir, Store, Setup) {
        let dir = tempfile::tempdir().unwrap();
        let setup =
            Setup {
                routines: dir.path().join("routines"),
                runs: dir.path().join("runs"),
                claude: "claude".into(),
                repos: dir.path().to_string_lossy().into_owned(),
                worktrees: dir.path().join("worktrees"),
            };
        let store = Store::open(&dir.path().join("cache"), Duration::ZERO);
        (dir, store, setup)
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
        let fields = [json!("Sweep PRs"), json!("weekdays 09:00"), json!("Sum up my PRs."), json!("~/w"), json!("claude"), json!("sonnet"), json!(false), json!("")];
        assert_eq!(rewrite(&setup, "sweep", &fields).unwrap(), "saved Sweep PRs");
        let r = routine(&setup, "sweep").unwrap();
        assert_eq!((r.name.as_str(), r.when_text.as_str(), r.body.as_str(), r.model.as_str()), ("Sweep PRs", "weekdays 09:00", "Sum up my PRs.", "sonnet"));
        assert_eq!((r.worktree, r.notify, r.tools.clone()), (false, routines::Notify::Always, vec!["Read".to_string()]));
        assert!(rewrite(&setup, "sweep", &[json!("x"), json!("when?"), json!("y")]).is_err());
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
