//! The monitor for the Herdr app's cockpit mod: the view model the hub pushes (verdict tiles, each app with its meters,
//! history strips, alerting monitors, new errors and releases judged before and after, the timeline) and the actions
//! each row offers, the same ones the TUI's rows have, ready to send.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use cockpit_core::acts::Act;
use cockpit_core::files::{self, Density};
use serde_json::{Value, json};

use crate::app::{App, Cmd};
use crate::data::{self, Clock, Doc, State, Window, count, duration, id_text};
use crate::rows::{Item, ordered, window_url};

/// The view reads only store documents; the time you last looked is its own action's.
pub fn dirs() -> Vec<PathBuf> {
    Vec::new()
}

fn seen_path() -> PathBuf {
    files::cache().join("seen").join("monitor.json")
}

pub fn view(_args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let value = doc("monitor");
    let value = if value.is_null() { files::load(&files::cache().join("monitor.json")).unwrap_or(Value::Null) } else { value };
    let since = files::load(&seen_path()).and_then(|v| v["at"].as_u64());
    model(&value, Clock::system(), since)
}

/// `looked []`: now is when you last looked at the monitor, as the TUI keeps it when its pane loses the focus.
pub fn act(action: &str, _args: &[Value]) -> Option<Result<String, String>> {
    match action {
        "looked" => Some(files::write(&seen_path(), &json!({"at": Utc::now().timestamp_millis()})).map(|_| String::new()).map_err(|e| e.to_string())),
        _ => None,
    }
}

fn state_name(state: State) -> &'static str {
    match state {
        State::Watching => "watching",
        State::Ok => "ok",
        State::Moved => "moved",
        State::Failed => "failed",
        State::Unknown => "unknown",
    }
}

/// How bad, for the worst of several: failed, moved, alert, watching, ok, idle.
fn rank(state: &str) -> u8 {
    match state {
        "failed" => 5,
        "moved" => 4,
        "alert" => 3,
        "watching" => 2,
        "ok" => 1,
        _ => 0,
    }
}

/// The hub's action and arguments for a command; the app goes as `{"$monitor": id}`, which the hub reads from the
/// monitor document with its environment. None for what only moves the view.
fn call(cmd: &Cmd) -> Option<(&'static str, Vec<Value>)> {
    let app = |id: &str| json!({"$monitor": id});
    Some(match cmd {
        Cmd::OpenUrl(url) => ("open_url", vec![json!(url)]),
        Cmd::Investigate(id, target) => ("investigate", vec![app(id), target.clone()]),
        Cmd::Fix(id, issue) => ("fix", vec![app(id), json!(issue)]),
        Cmd::Bug(id, target) => ("bug", vec![app(id), target.clone()]),
        Cmd::RollBack(id, release, previous) => ("roll_back", vec![app(id), json!(release), json!(previous)]),
        Cmd::Talk(id, release) => ("talk", vec![app(id), json!(release)]),
        Cmd::Pin(id, pin) => ("pin", vec![json!(id), json!(pin)]),
        Cmd::Refresh => ("request_refresh", Vec::new()),
        Cmd::Fold(_) | Cmd::View | Cmd::GoTo(_) => return None,
    })
}

/// A row's actions as the mod draws and sends them: label, key, main, the confirmation and its question.
fn acts(tui: &App, item: &Item) -> Vec<Value> {
    tui.acts_for(item)
        .into_iter()
        .filter_map(|(act, cmd): (Act, Cmd)| {
            let (action, args) = call(&cmd)?;
            let question = act.confirm.as_ref().map(|_| tui.question(&cmd));
            Some(json!({
                "id": act.id, "label": act.label, "key": act.key.map(String::from), "main": act.main, "confirm": act.confirm,
                "question": question, "action": action, "args": args,
            }))
        })
        .collect()
}

/// A window before or after a release: its error rate in percent, p95 in ms and the counts.
fn window(w: &Option<Window>) -> Value {
    match w {
        Some(w) => {
            let rate = if w.hits > 0.0 { w.errors / w.hits * 100.0 } else { 0.0 };
            json!({"errors": w.errors, "hits": w.hits, "rate": rate, "rateText": format!("{rate:.2}%"), "p95": w.p95 * 1000.0, "p95Text": duration(w.p95)})
        }
        None => Value::Null,
    }
}

/// After over before, 1 when either is missing or zero.
fn ratio(after: f64, before: f64) -> f64 {
    if before > 0.0 && after >= 0.0 { after / before } else { 1.0 }
}

/// A series as its strip draws it: each value with its level against the series' usual (its median), 0 usual, 1 up
/// by half, 2 doubled; the last value as text.
fn strip(values: &[f64], text: impl Fn(f64) -> String) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let usual = sorted[sorted.len() / 2];
    let max = sorted.last().copied().unwrap_or(0.0);
    let level = |v: f64| if usual <= 0.0 { u8::from(v > 0.0) } else if v >= usual * 2.0 { 2 } else if v >= usual * 1.5 { 1 } else { 0 };
    let last = values.last().copied().unwrap_or(0.0);
    json!({"values": values, "levels": values.iter().map(|v| level(*v)).collect::<Vec<_>>(), "max": max, "usual": usual, "last": last, "text": text(last), "level": level(last)})
}

fn ago(clock: &Clock, at: Option<DateTime<Utc>>) -> Value {
    at.map_or(Value::Null, |at| json!(clock.ago(at)))
}

fn ms(at: Option<DateTime<Utc>>) -> Value {
    at.map_or(Value::Null, |at| json!(at.timestamp_millis()))
}

fn release(tui: &App, clock: &Clock, app: &data::App, r: &data::Release) -> Value {
    let id = id_text(&r.id);
    let v = &r.verdict;
    let (before, after) = (v.before, v.after);
    let change = match (before, after) {
        (Some(b), Some(a)) => {
            let rate = |w: Window| if w.hits > 0.0 { w.errors / w.hits } else { 0.0 };
            json!({"errors": ratio(rate(a), rate(b)), "p95": ratio(a.p95, b.p95)})
        }
        _ => Value::Null,
    };
    let state = state_name(v.state);
    json!({
        "key": format!("{}/release/{id}", app.id), "id": id, "app": app.id, "appName": app.name, "tag": r.tag, "who": r.who(), "mine": r.is_mine,
        "phase": r.phase, "at": ms(r.at()), "hm": r.at().map(|t| clock.hm(t)), "ago": ago(clock, r.at()), "today": r.at().is_some_and(|t| clock.is_today(t)),
        "state": state, "summary": v.summary, "before": window(&before), "after": window(&after), "change": change, "until": ms(v.until),
        "watchingFor": v.until.filter(|u| *u > clock.now).map(|u| (u - clock.now).num_minutes()),
        "newIssues": v.new_issues, "tickets": r.tickets.iter().map(|t| json!({"id": t.id, "url": t.url})).collect::<Vec<_>>(),
        "prs": r.prs.iter().map(|p| json!({"name": format!("{}#{}", p.repo, p.number), "url": p.url})).collect::<Vec<_>>(),
        "windowUrl": window_url(app, r), "previous": app.previous_tag(&id),
        "acts": acts(tui, &Item::Release(app.id.clone(), id.clone())),
    })
}

fn app_model(tui: &App, clock: &Clock, app: &data::App) -> Value {
    let releases: Vec<Value> = app.releases.iter().map(|r| release(tui, clock, app, r)).collect();
    let worst = releases.iter().filter_map(|r| r["state"].as_str()).max_by_key(|s| rank(s));
    let state = match (worst, app.alerting().is_empty()) {
        (Some(s @ ("failed" | "moved")), _) => s,
        (_, false) => "alert",
        (Some(s), true) => s,
        (None, true) if app.watched => "ok",
        _ => "idle",
    };
    let monitors = app.monitors.as_ref().map(|m| json!({"ok": m.ok, "warn": m.warn, "alert": m.alert, "nodata": m.nodata, "total": m.ok + m.warn + m.alert + m.nodata}));
    let series = app.series.as_ref();
    let alerting: Vec<Value> = app
        .alerting()
        .iter()
        .map(|m| {
            let id = id_text(&m.id);
            json!({"key": format!("{}/monitor/{id}", app.id), "id": id, "name": m.name, "status": m.status, "url": m.url,
                   "acts": acts(tui, &Item::Monitor(app.id.clone(), id.clone()))})
        })
        .collect();
    let issues: Vec<Value> = app
        .issues
        .iter()
        .map(|i| {
            let release = app.releases.iter().find(|r| r.verdict.new_issues.contains(&i.id)).map(|r| r.tag.clone());
            json!({"key": format!("{}/issue/{}", app.id, i.id), "id": i.id, "kind": i.kind, "message": i.message, "count": i.count,
                   "firstSeen": ms(i.first_seen), "firstSeenAgo": ago(clock, i.first_seen), "firstVersion": i.first_version, "url": i.url,
                   "isNew": i.is_new, "release": release, "acts": if i.is_new { acts(tui, &Item::Issue(app.id.clone(), i.id.clone())) } else { Vec::new() }})
        })
        .collect();
    json!({
        "key": app.id, "id": app.id, "name": app.name, "service": app.service, "repo": app.repo, "tag": app.tag, "health": app.health,
        "deployed": ms(app.deployed_at), "deployedAgo": ago(clock, app.deployed_at), "behind": app.behind, "watched": app.watched, "pinned": app.pinned,
        "troubled": app.is_troubled(), "state": state, "monitors": monitors, "counts": app.counts(),
        "errors": series.map_or(Value::Null, |s| strip(&s.errors, |v| format!("{} err", count(v)))),
        "p95": series.map_or(Value::Null, |s| strip(&s.p95.iter().map(|v| v * 1000.0).collect::<Vec<_>>(), |v| duration(v / 1000.0))),
        "alerting": alerting, "issues": issues, "releases": releases, "datadogUrl": app.datadog_url, "argoUrl": app.argo_url,
        "acts": acts(tui, &Item::App(app.id.clone())), "help": tui.row_help(&Item::App(app.id.clone())),
    })
}

/// The whole view: the headline and tiles, the apps (troubled first), the releases newest first.
pub fn model(value: &Value, clock: Clock, since: Option<u64>) -> Value {
    let Some(doc) = Doc::parse(value).filter(|_| !value.is_null()) else {
        return json!({"ready": false, "since": since});
    };
    let mut tui = App::with(Some(clock), Density::Comfortable, false);
    tui.set_doc(value);
    let apps: Vec<Value> = ordered(&doc).into_iter().map(|a| app_model(&tui, &clock, a)).collect();
    let mut timeline: Vec<&Value> = apps.iter().flat_map(|a| a["releases"].as_array().into_iter().flatten()).collect();
    timeline.sort_by_key(|r| std::cmp::Reverse(r["at"].as_i64().unwrap_or(0)));
    let today: Vec<&&Value> = timeline.iter().filter(|r| r["today"] == true).collect();
    let of = |state: &str| today.iter().filter(|r| r["state"] == state).count();
    let alerts: u32 = doc.apps.iter().filter_map(|a| a.monitors.as_ref()).map(|m| m.alert).sum();
    let warns: u32 = doc.apps.iter().filter_map(|a| a.monitors.as_ref()).map(|m| m.warn).sum();
    let monitors: u32 = doc.apps.iter().filter_map(|a| a.monitors.as_ref()).map(|m| m.ok + m.warn + m.alert + m.nodata).sum();
    let new_errors: Vec<&data::Issue> = doc.apps.iter().flat_map(|a| a.new_issues()).collect();
    let verdict = apps.iter().filter_map(|a| a["state"].as_str()).max_by_key(|s| rank(s)).unwrap_or("idle");
    let (headline, bad) = crate::pinned::sidebar_text(&doc);
    let headline = headline.trim_start_matches(|c: char| !c.is_ascii_alphanumeric()).trim().to_string();
    let at = doc.at.and_then(DateTime::<Utc>::from_timestamp_millis);
    json!({
        "ready": true, "env": doc.env, "at": doc.at, "ago": ago(&clock, at), "since": since, "errors": doc.error_lines(),
        "headline": headline, "bad": bad, "verdict": verdict,
        "tiles": {
            "today": today.len(), "ok": of("ok"), "moved": of("moved"), "failed": of("failed"), "watching": of("watching"),
            "alerts": alerts, "warns": warns, "monitors": monitors, "newErrors": new_errors.len(),
            "newErrorEvents": new_errors.iter().map(|i| i.count).sum::<u64>(), "apps": doc.apps.len(), "watched": doc.apps.iter().filter(|a| a.watched).count(),
        },
        "apps": apps,
        "timeline": timeline.iter().map(|r| r["key"].clone()).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/monitor.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn clock() -> Clock {
        let now = DateTime::parse_from_rfc3339("2026-10-28T17:56:51+01:00").unwrap();
        Clock { now: now.with_timezone(&Utc), zone: Some(*now.offset()) }
    }

    #[test]
    fn the_view_judges_apps_and_carries_each_rows_actions() {
        let view = model(&fixture(), clock(), Some(5));
        assert_eq!((view["ready"].as_bool(), view["since"].as_u64()), (Some(true), Some(5)));
        let apps = view["apps"].as_array().unwrap();
        let first = &apps[0];
        assert_eq!(first["troubled"], true, "troubled apps come first");
        assert!(rank(view["verdict"].as_str().unwrap()) >= rank(first["state"].as_str().unwrap()));
        let release = first["releases"].as_array().unwrap().iter().find(|r| !r["acts"].as_array().unwrap().is_empty()).unwrap();
        let acts = release["acts"].as_array().unwrap();
        let investigate = acts.iter().find(|a| a["id"] == "investigate").unwrap();
        assert_eq!(investigate["args"][0]["$monitor"], first["id"]);
        assert_eq!(investigate["args"][1]["release"].to_string().trim_matches('"'), release["id"].as_str().unwrap());
        if let Some(back) = acts.iter().find(|a| a["id"] == "roll_back") {
            assert!(back["confirm"].as_str().unwrap().starts_with("rollback-"));
            assert!(back["question"].as_str().unwrap().starts_with("roll back"));
        }
        let timeline = view["timeline"].as_array().unwrap();
        assert!(!timeline.is_empty() && timeline.iter().all(|k| k.as_str().unwrap().contains("/release/")));
        assert_eq!(model(&Value::Null, clock(), None)["ready"], false);
    }

    #[test]
    fn a_strip_says_how_far_each_value_is_from_the_usual() {
        let s = strip(&[2.0, 2.0, 2.0, 2.0, 3.0, 5.0, 9.0], |v| format!("{v}"));
        assert_eq!(s["levels"], json!([0, 0, 0, 0, 1, 2, 2]));
        assert_eq!((s["usual"].as_f64(), s["text"].as_str()), (Some(2.0), Some("9")));
    }
}
