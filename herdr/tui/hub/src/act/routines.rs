//! The routines space's actions: add one, run, stop, pause or delete it, edit its file or read a run's output in a tab
//! of the actions space, and talk a Claude run over by resuming its session there.

use std::path::Path;

use cockpit_core::routines::{self, When};
use serde_json::{Value, json};

use super::inbox::start_action;
use super::space;
use super::{Ctx, arg, opt};
use crate::herdr;
use crate::routines::{self as runner, Setup, now_ms};
use crate::store::{self, Store};
use crate::value::{Res, cut, quote, text};

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

/// A new routine from the app's form: name, when, prompt (or script), folder, and "shell" for a script.
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
    }
    let path = setup.routines.join(format!("{slug}.md"));
    std::fs::write(&path, file).map_err(|e| e.to_string())?;
    Ok(format!("added {name}: e edits its file"))
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
    let pane = space::open_tab(&ctx.cfg, cwd, label, Some(label)).or_else(|| {
        let created = herdr::run(&["workspace", "create", "--cwd", cwd, "--label", label, "--focus"]);
        herdr::result_of(&created).and_then(|r| r["root_pane"]["pane_id"].as_str().map(str::to_string))
    });
    let Some(pane) = pane else { return Err("herdr opened no tab".into()) };
    herdr::run(&["pane", "run", &pane, line]);
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
            Setup { routines: dir.path().join("routines"), runs: dir.path().join("runs"), claude: "claude".into(), repos: dir.path().to_string_lossy().into_owned() };
        let store = Store::open(&dir.path().join("cache"), Duration::ZERO);
        (dir, store, setup)
    }

    #[test]
    fn a_routine_added_from_the_form_is_a_file_named_after_it() {
        let (_dir, _, setup) = world();
        let args = [json!("Morning sweep"), json!("weekdays 09:00"), json!("Sum up my PRs."), json!("~/work"), json!("")];
        assert_eq!(create(&setup, &args).unwrap(), "added Morning sweep: e edits its file");
        let text = std::fs::read_to_string(setup.routines.join("morning-sweep.md")).unwrap();
        assert_eq!(text, "---\nname: Morning sweep\nwhen: weekdays 09:00\ncwd: ~/work\n---\n\nSum up my PRs.\n");
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
    fn a_run_is_found_by_id_else_the_last() {
        let doc = json!({"runs": {"s": [{"id": "2"}, {"id": "1"}]}});
        assert_eq!(run_of(&doc, "s", &Value::Null).unwrap()["id"], "2");
        assert_eq!(run_of(&doc, "s", &json!("1")).unwrap()["id"], "1");
        assert_eq!(run_of(&doc, "t", &Value::Null).unwrap_err(), "t has not run yet");
    }
}
