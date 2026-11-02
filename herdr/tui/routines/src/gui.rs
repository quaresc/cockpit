//! The routines for the Herdr app's cockpit mod: the view model the hub pushes (tiles for the week, the run inbox, each
//! routine with its state, next runs, history and runs) with the actions each row offers, the same ones the TUI's rows
//! have, ready to send; and `routines-when`, what a schedule typed in the form means and when it runs next.

use std::path::{Path, PathBuf};

use chrono::{Datelike, Duration, Local, NaiveDateTime, TimeZone};
use cockpit_core::files::{self, Density};
use cockpit_core::routines::{TRIGGERS, When};
use serde_json::{Value, json};

use crate::app::{App, Cmd};
use crate::data::{Clock, Doc, Routine, Run, State, duration, estimate};
use crate::rows::{Item, ordered};

const HISTORY: usize = 20;
const DAY_MS: u64 = 86_400_000;

/// The view reads only the store's `routines` document; the time you last looked is its own action's.
pub fn dirs() -> Vec<PathBuf> {
    Vec::new()
}

fn seen_path() -> PathBuf {
    files::cache().join("seen").join("routines.json")
}

pub fn view(_args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let value = doc("routines");
    let value = if value.is_null() { files::load(&files::cache().join("routines.json")).unwrap_or(Value::Null) } else { value };
    let since = files::load(&seen_path()).and_then(|v| v["at"].as_u64());
    model(&value, Clock::system(), since, &files::cache().join("routines"))
}

/// `looked []`: now is when you last looked at the routines, as the TUI keeps it when its pane loses the focus.
pub fn act(action: &str, _args: &[Value]) -> Option<Result<String, String>> {
    match action {
        "looked" => Some(files::write(&seen_path(), &json!({"at": Clock::system().now})).map(|_| String::new()).map_err(|e| e.to_string())),
        _ => None,
    }
}

/// `routines-when {when}`: what the schedule means, its next three runs, or why it does not read.
pub fn when_view(args: &Value, _doc: &dyn Fn(&str) -> Value) -> Value {
    preview(args["when"].as_str().unwrap_or_default(), Clock::system())
}

pub fn no_act(_action: &str, _args: &[Value]) -> Option<Result<String, String>> {
    None
}

fn ms_of(clock: &Clock, at: NaiveDateTime) -> Option<u64> {
    match clock.zone {
        Some(zone) => zone.from_local_datetime(&at).earliest().map(|t| t.timestamp_millis() as u64),
        None => Local.from_local_datetime(&at).earliest().map(|t| t.timestamp_millis() as u64),
    }
}

pub fn preview(text: &str, clock: Clock) -> Value {
    match When::parse(text) {
        Ok(when) => {
            let next: Vec<Value> = std::iter::successors(when.next_after(clock.local(clock.now)), |at| when.next_after(*at))
                .take(3)
                .filter_map(|at| ms_of(&clock, at))
                .map(|at| json!({"at": at, "text": clock.until(at)}))
                .collect();
            let kind = match when {
                When::Manual => "manual",
                When::Every(_) => "every",
                When::Cron(_) => "schedule",
                When::On { .. } => "event",
            };
            json!({"ok": true, "when": text, "kind": kind, "describe": when.describe(text), "next": next})
        }
        Err(error) => json!({"ok": false, "when": text, "error": error}),
    }
}

fn state_name(state: State) -> &'static str {
    match state {
        State::Running => "running",
        State::Waiting => "waiting",
        State::Broken => "broken",
        State::Over => "over",
        State::Paused => "paused",
        State::Failed => "failed",
        State::Missed => "missed",
        State::Ok => "ok",
        State::Never => "never",
    }
}

/// The hub's action and arguments for a command; None for what only moves the view.
fn call(cmd: &Cmd) -> Option<(&'static str, Vec<Value>)> {
    let run = |id: &Option<String>| id.as_ref().map_or(Value::Null, |id| json!(id));
    Some(match cmd {
        Cmd::Run(slug) => ("run", vec![json!(slug)]),
        Cmd::Stop(slug) => ("stop", vec![json!(slug)]),
        Cmd::Pause(slug, paused) => ("pause", vec![json!(slug), json!(paused)]),
        Cmd::Model(slug, model) => ("model", vec![json!(slug), json!(model)]),
        Cmd::Delete(slug) => ("delete", vec![json!(slug)]),
        Cmd::Edit(slug) => ("edit", vec![json!(slug)]),
        Cmd::Output(slug, id) => ("output", vec![json!(slug), run(id)]),
        Cmd::Talk(slug, id) => ("talk", vec![json!(slug), run(id)]),
        Cmd::Worktree(slug, id) => ("open_worktree", vec![json!(slug), json!(id)]),
        Cmd::Refresh => ("request_refresh", Vec::new()),
        Cmd::ReadAll => ("read_all", Vec::new()),
        Cmd::Cleanup => ("cleanup", Vec::new()),
        Cmd::Add | Cmd::Fold(_) | Cmd::GoTo(_) => return None,
    })
}

fn acts(tui: &App, item: &Item) -> Vec<Value> {
    tui.acts_for(item)
        .into_iter()
        .filter_map(|(act, cmd)| {
            let (action, args) = call(&cmd)?;
            let question = act.confirm.as_ref().map(|_| tui.question(&cmd));
            Some(json!({
                "id": act.id, "label": act.label, "key": act.key.map(String::from), "main": act.main, "confirm": act.confirm,
                "question": question, "action": action, "args": args,
            }))
        })
        .collect()
}

fn home_short(path: &str) -> String {
    let home = files::home().to_string_lossy().into_owned();
    if !home.is_empty() && path.starts_with(&home) { path.replacen(&home, "~", 1) } else { path.to_string() }
}

fn run_model(tui: &App, doc: &Doc, clock: &Clock, r: &Routine, run: &Run, runs_dir: &Path) -> Value {
    let worktree = run.worktree.as_ref().map(|w| json!({"path": home_short(&w.path), "full": w.path, "branch": w.branch, "kept": w.kept(), "describe": w.describe()}));
    json!({
        "key": format!("{}/run/{}", r.slug, run.id), "slug": r.slug, "id": run.id, "name": r.name, "started": run.started, "ended": run.ended,
        "when": clock.when(run.started), "ago": clock.ago(run.ended.max(run.started)), "took": if run.missed { String::new() } else { run.took() },
        "cost": run.cost.map(estimate), "costValue": run.cost, "ok": run.ok, "word": run.word(), "why": run.why, "summary": run.summary,
        "code": run.code, "session": run.session(), "late": run.late, "missed": run.missed, "timedOut": run.timed_out, "unread": doc.is_unread(run),
        "worktree": worktree, "output": runs_dir.join(&r.slug).join(format!("{}.txt", run.id)),
        "acts": acts(tui, &Item::Run(r.slug.clone(), run.id.clone())),
    })
}

fn routine_model(tui: &App, doc: &Doc, clock: &Clock, r: &Routine, runs_dir: &Path) -> Value {
    let runs = doc.runs(&r.slug);
    let running = doc.running.get(&r.slug).map(|run| {
        json!({"id": run.id, "started": run.started, "for": duration(clock.now.saturating_sub(run.started)), "why": run.why, "stopping": run.stopping.is_some()})
    });
    let history: Vec<Value> = runs.iter().take(HISTORY).rev().map(|run| json!({"id": run.id, "word": run.word(), "when": clock.when(run.started), "summary": run.summary})).collect();
    let next_runs: Vec<Value> = r.next_runs.iter().map(|at| json!({"at": at, "text": clock.until(*at)})).collect();
    json!({
        "key": r.slug, "slug": r.slug, "name": r.name, "state": state_name(doc.state(r)), "when": r.when, "whenText": r.when_text, "trigger": r.trigger,
        "error": r.error, "problems": r.problems, "run": r.run, "model": r.model, "mode": r.mode, "tools": r.tools, "cwd": home_short(&r.cwd),
        "budget": r.budget, "spentWeek": r.spent_week, "spent": estimate(r.spent_week), "lastCost": r.last_cost.map(estimate), "over": r.over,
        "paused": r.paused, "prompt": r.prompt, "path": r.path, "notify": r.notify, "worktree": r.worktree, "catchUp": r.catch_up,
        "next": r.next, "nextText": r.next.filter(|_| !r.paused).map(|at| clock.until(at)), "nextRuns": next_runs,
        "settings": crate::view::settings(r), "running": running, "queued": doc.queued.contains(&r.slug), "history": history,
        "runs": runs.iter().map(|run| run_model(tui, doc, clock, r, run, runs_dir)).collect::<Vec<_>>(),
        "unread": runs.iter().filter(|run| doc.is_unread(run)).count(),
        "acts": acts(tui, &Item::Routine(r.slug.clone())),
    })
}

/// The week's runs a day at a time, oldest day first: how many ended ok, failed or were missed, and what they cost.
fn days(doc: &Doc, clock: &Clock) -> Vec<Value> {
    let today = clock.local(clock.now).date();
    (0..7)
        .rev()
        .map(|back| {
            let day = today - Duration::days(back);
            let runs: Vec<&Run> = doc.runs.values().flatten().filter(|run| clock.local(run.started).date() == day).collect();
            let of = |word: &str| runs.iter().filter(|run| run.word() == word).count();
            json!({
                "label": day.format("%a").to_string(), "date": day.format("%-d %b").to_string(), "today": back == 0, "ok": of("ok"),
                "failed": of("failed") + of("stopped"), "missed": of("missed"), "cost": runs.iter().filter_map(|r| r.cost).sum::<f64>(), "weekday": day.weekday().num_days_from_monday(),
            })
        })
        .collect()
}

/// The whole view: the week's tiles, the run inbox (newest first), the routines (running and failing first).
pub fn model(value: &Value, clock: Clock, since: Option<u64>, runs_dir: &Path) -> Value {
    let Some(doc) = Doc::parse(value) else { return json!({"ready": false, "since": since}) };
    let mut tui = App::with(Some(clock), Density::Comfortable, false, runs_dir.to_path_buf());
    tui.set_doc(value);
    let routines: Vec<Value> = ordered(&doc).into_iter().map(|r| routine_model(&tui, &doc, &clock, r, runs_dir)).collect();
    let inbox: Vec<Value> = doc
        .unread()
        .into_iter()
        .filter_map(|(slug, run)| doc.routine(slug).map(|r| run_model(&tui, &doc, &clock, r, run, runs_dir)))
        .collect();
    let week: Vec<&Run> = doc.runs.values().flatten().filter(|run| clock.now.saturating_sub(run.started) < 7 * DAY_MS).collect();
    let count = |word: &str| week.iter().filter(|run| run.word() == word).count();
    let states: Vec<State> = doc.routines.iter().map(|r| doc.state(r)).collect();
    let of = |s: State| states.iter().filter(|x| **x == s).count();
    let next = doc.routines.iter().filter(|r| !r.paused && r.error.is_none()).filter_map(|r| r.next.map(|at| (at, r))).min_by_key(|(at, _)| *at);
    let worktrees = doc.runs.values().flatten().filter(|run| run.worktree.as_ref().is_some_and(|w| w.kept())).count();
    let (spent, budget) = (doc.routines.iter().map(|r| r.spent_week).sum::<f64>(), doc.routines.iter().filter_map(|r| r.budget).sum::<f64>());
    json!({
        "ready": true, "at": doc.at, "since": since, "runsDir": runs_dir,
        "tiles": {
            "routines": doc.routines.len(), "active": doc.routines.iter().filter(|r| !r.paused).count(), "paused": of(State::Paused),
            "running": of(State::Running), "waiting": of(State::Waiting), "failing": of(State::Failed) + of(State::Broken) + of(State::Missed) + of(State::Over),
            "weekRuns": week.len(), "weekOk": count("ok"), "weekFailed": count("failed") + count("stopped"), "weekMissed": count("missed"),
            "weekCost": week.iter().filter_map(|r| r.cost).sum::<f64>(), "spent": estimate(spent),
            "budget": budget, "overBudget": budget > 0.0 && spent > budget,
            "next": next.map(|(at, r)| json!({"slug": r.slug, "name": r.name, "at": at, "text": clock.until(at)})),
            "worktrees": worktrees, "unread": inbox.len(),
        },
        "days": days(&doc, &clock),
        "inbox": inbox,
        "routines": routines,
        "triggers": TRIGGERS.iter().map(|(word, _, what)| json!({"word": word, "what": what})).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn clock() -> Clock {
        let now = DateTime::parse_from_rfc3339("2026-10-29T10:00:00+01:00").unwrap();
        Clock { now: now.timestamp_millis() as u64, zone: Some(*now.offset()) }
    }

    #[test]
    fn the_view_carries_the_inbox_the_week_and_each_rows_actions() {
        let now = clock().now;
        let doc = json!({"version": 1, "inboxFrom": now - 3_600_000, "routines": [
            {"slug": "a", "name": "A", "when": "daily at 09:00", "next": now + 3_600_000, "nextRuns": [now + 3_600_000]},
            {"slug": "b", "name": "B", "paused": true}],
            "running": {}, "queued": [],
            "runs": {"a": [{"id": "2", "started": now - 600_000, "ended": now - 590_000, "ok": false, "missed": true, "summary": "missed Thu 09:50"},
                           {"id": "1", "started": now - 7_200_000, "ended": now - 7_100_000, "ok": true, "summary": "done",
                            "worktree": {"path": "/w", "branch": "auto/a-1", "commits": 2}}]}});
        let view = model(&doc, clock(), Some(1), Path::new("/runs"));
        let inbox = view["inbox"].as_array().unwrap();
        assert_eq!((inbox.len(), inbox[0]["word"].as_str(), inbox[0]["key"].as_str()), (1, Some("missed"), Some("a/run/2")));
        let a = &view["routines"][0];
        assert_eq!((a["state"].as_str(), a["history"].as_array().unwrap().len()), (Some("missed"), 2));
        assert_eq!(a["runs"][1]["worktree"]["describe"], "auto/a-1 · 2 commits");
        assert!(a["runs"][1]["acts"].as_array().unwrap().iter().any(|x| x["action"] == "open_worktree"));
        let delete = a["acts"].as_array().unwrap().iter().find(|x| x["action"] == "delete").unwrap();
        assert_eq!((delete["confirm"].as_str(), delete["args"][0].as_str()), (Some("delete-routine-a"), Some("a")));
        assert_eq!(view["tiles"]["weekMissed"], 1);
        assert_eq!(view["tiles"]["worktrees"], 1);
        assert_eq!(view["days"].as_array().unwrap().len(), 7);
        assert_eq!(view["days"][6]["missed"], 1);
        assert_eq!(model(&Value::Null, clock(), None, Path::new("/runs"))["ready"], false);
    }

    #[test]
    fn a_schedule_reads_back_with_its_next_runs() {
        let p = preview("weekdays 09:00", clock());
        assert_eq!((p["ok"].as_bool(), p["describe"].as_str()), (Some(true), Some("weekdays at 09:00")));
        let next: Vec<&str> = p["next"].as_array().unwrap().iter().map(|n| n["text"].as_str().unwrap()).collect();
        assert_eq!(next, ["Fri 09:00", "Mon 09:00", "Tue 09:00"]);
        assert_eq!(preview("sometimes", clock())["ok"], false);
        assert_eq!(preview("every 30m", clock())["kind"], "every");
    }
}
