//! The inbox for the Herdr app's cockpit mod: the TUI's own rows (`model::model`) as JSON, with flat lists of my PRs,
//! tickets and worktrees for the home screen, and the outcomes of the requests the rows filed.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use cockpit_core::files;
use cockpit_core::theme::{DIM, GREEN, ORANGE, RED};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use serde_json::{Map, Value, json};

use crate::facts::{self, Config, ago, has_ticket, is_failing, list, needs_me, s};
use crate::icons;
use crate::inputs::Inputs;
use crate::model::{self, Ctx};
use crate::row::{Body, Call, Kind, Row};
use crate::sessions::Local;

fn results_dir() -> PathBuf {
    files::cache().join("requests").join("results")
}

/// The folders the view reads: the config's (muted.json), Claude's costs, and the requests' outcomes.
pub fn dirs() -> Vec<PathBuf> {
    vec![files::config_dir(), files::cache().join("costs"), results_dir()]
}

/// The view for `{group: "feature" | "status"}`, from the hub's documents (the cache files before the hub has them).
pub fn view(args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let pick = |name: &str| {
        let value = doc(name);
        if value.is_null() { files::load(&files::cache().join(format!("{name}.json"))) } else { Some(value) }
    };
    let inputs = Inputs::of(pick("inbox"), pick("ci"), pick("brief"));
    static LOCAL: Mutex<Option<Local>> = Mutex::new(None);
    let mut local = LOCAL.lock().unwrap_or_else(|e| e.into_inner());
    let local = local.get_or_insert_with(Local::default);
    model_of(&inputs, &Config::load(), args["group"] == "status", local, results())
}

/// The outcomes the mod wrote for requests, by file name.
fn results() -> Value {
    let mut out = Map::new();
    for entry in std::fs::read_dir(results_dir()).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json")
            && let Some(outcome) = files::load(&entry.path())
        {
            out.insert(name, outcome);
        }
    }
    Value::Object(out)
}

/// Each ticket's cost history, oldest first, from the hub's history store.
fn series(ids: &[String]) -> HashMap<String, Vec<f64>> {
    let kept = std::fs::read_to_string(files::cache().join("history").join("ticket_cost.jsonl")).unwrap_or_default();
    let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut out: HashMap<String, Vec<f64>> = HashMap::new();
    for point in kept.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        if let (Some(key), Some(value)) = (point["key"].as_str().filter(|k| wanted.contains(k)), point["value"].as_f64()) {
            out.entry(key.to_string()).or_default().push(value);
        }
    }
    out
}

pub fn model_of(inputs: &Inputs, config: &Config, by_status: bool, local: &mut Local, results: Value) -> Value {
    let data = model::usable(inputs).cloned().unwrap_or(Value::Null);
    let ids: Vec<String> = list(&data, "tickets").iter().map(|t| s(t, "id").to_string()).collect();
    let series = series(&ids);
    let ctx = Ctx { config, by_status, compact: false, series: &series };
    let (head, top) = model::model(inputs, &ctx, local);
    let state = match (&inputs.data, model::usable(inputs)) {
        (None, _) => "empty",
        (Some(_), None) => "stale",
        _ => "ok",
    };
    let message = if state == "ok" { String::new() } else { head.first().map(plain).unwrap_or_default() };
    let errors: Vec<String> = if state == "ok" { head.iter().skip(1).map(plain).collect() } else { Vec::new() };
    let sections: Vec<Value> = top.iter().map(|row| row_json(row, "")).collect();
    let alerts = top.iter().find(|r| r.key == "alerts").map_or(0, |r| r.children.len());
    let (prs, tickets, worktrees) = (prs(&data, &inputs.ci), tickets(&data, inputs), worktrees(&data, inputs));
    let direct = list(&data, "direct").len();
    let counts = json!({
        "needs_you": prs.iter().filter(|p| p["needs_you"] == true).count() + direct,
        "reviews": direct + list(&data, "team").len(),
        "direct": direct,
        "tickets": tickets.len(),
        "in_progress": tickets.iter().filter(|t| t["statusType"] == "started").count(),
        "prs": prs.len(),
        "ready": prs.iter().filter(|p| p["ready"]["state"] == "ready").count(),
        "failing": prs.iter().filter(|p| p["ci"] == "FAILURE" || p["ci"] == "ERROR").count(),
        "alerts": alerts,
        "worktrees": worktrees.len(),
        "cleanup": worktrees.iter().filter(|w| w["merged"] == true && w["changes"] == 0).count(),
    });
    // When it was fetched goes out by the minute the app shows it in, not the millisecond: each refresh would move the
    // whole view otherwise.
    json!({
        "updated": if data["at"].is_null() { String::new() } else { facts::clock(&data["at"]) },
        "state": state,
        "message": message,
        "errors": errors,
        "group": if by_status { "status" } else { "feature" },
        "counts": counts,
        "sections": sections,
        "prs": prs,
        "tickets": tickets,
        "worktrees": worktrees,
        "results": results,
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Flat lists

fn run_of<'a>(ci: &'a Value, pr: &Value) -> Option<&'a Value> {
    list(ci, "runs").iter().find(|r| r["repo"] == pr["repo"] && r["branch"] == pr["branch"])
}

fn prs(data: &Value, ci: &Value) -> Vec<Value> {
    list(data, "mine")
        .iter()
        .map(|pr| {
            let run = run_of(ci, pr);
            let (state, text, tone) = facts::readiness(pr, run);
            let ticket = list(data, "tickets").iter().map(|t| s(t, "id")).find(|id| has_ticket(s(pr, "branch"), id));
            let running = run.filter(|r| facts::flag(r, "isRunning")).map(|r| json!({"done": r["done"], "total": r["total"]}));
            json!({
                "url": pr["url"], "repo": pr["repo"], "number": pr["number"], "title": pr["title"], "branch": pr["branch"],
                "base": pr["base"], "draft": facts::flag(pr, "isDraft"), "ci": pr["ci"], "review": pr["review"],
                "updated": pr["updatedAt"], "age": ago(s(pr, "updatedAt")), "ticket": ticket, "failing": pr["failedChecks"],
                "running": running, "needs_you": needs_me(pr) || run.is_some_and(|r| s(r, "outcome") == "failed" && !facts::flag(r, "isRunning")) && !is_failing(pr),
                "ready": {"state": state, "text": text, "tone": tone},
            })
        })
        .collect()
}

fn tickets(data: &Value, inputs: &Inputs) -> Vec<Value> {
    list(data, "tickets")
        .iter()
        .map(|t| {
            let id = s(t, "id");
            let prs: Vec<&Value> = list(data, "mine").iter().filter(|pr| has_ticket(s(pr, "branch"), id)).map(|pr| &pr["url"]).collect();
            json!({
                "id": id, "title": t["title"], "url": t["url"], "status": t["status"], "statusType": t["statusType"],
                "project": t["project"], "parentId": t["parentId"], "prs": prs,
                "cost": inputs.costs.get(id).copied().filter(|c| *c != 0.0), "secs": inputs.secs.get(&id.to_uppercase()),
            })
        })
        .collect()
}

fn worktrees(data: &Value, inputs: &Inputs) -> Vec<Value> {
    let merged: HashSet<(&str, &str)> = list(data, "merged").iter().map(|m| (s(m, "repo"), s(m, "branch"))).collect();
    inputs
        .worktrees
        .iter()
        .map(|w| {
            let key = (s(w, "repo"), s(w, "branch"));
            let pr = list(data, "mine").iter().find(|pr| (s(pr, "repo"), s(pr, "branch")) == key).map(|pr| pr["url"].clone());
            json!({"path": w["path"], "repo": w["repo"], "branch": w["branch"], "changes": w["changes"], "merged": merged.contains(&key) && pr.is_none(), "pr": pr})
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// Rows

fn plain(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim().to_string()
}

fn tone(style: Style) -> &'static str {
    match style.fg {
        Some(c) if c == RED => "bad",
        Some(c) if c == ORANGE => "warn",
        Some(c) if c == GREEN => "good",
        Some(c) if c == DIM => "dim",
        _ => "",
    }
}

/// A Nerd Font glyph (the private use areas): drawn as an icon in the app.
fn is_glyph(text: &str) -> bool {
    let mut chars = text.trim().chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if ('\u{e000}'..='\u{f8ff}').contains(&c) || c >= '\u{f0000}')
}

fn is_glyph_char(c: char) -> bool {
    ('\u{e000}'..='\u{f8ff}').contains(&c) || c >= '\u{f0000}'
}

/// A line's spans as text and icons: each Nerd Font glyph in the text becomes an icon of its own.
fn spans(line: &Line) -> Vec<Value> {
    let mut out = Vec::new();
    for span in line.spans.iter().filter(|span| !span.content.trim().is_empty()) {
        let style = line.style.patch(span.style);
        let (tone, bold) = (tone(style), style.add_modifier.contains(Modifier::BOLD));
        let mut text = String::new();
        let flush = |text: &mut String, out: &mut Vec<Value>| {
            if !text.trim().is_empty() {
                out.push(json!({"text": text.trim(), "tone": tone, "bold": bold}));
            }
            text.clear();
        };
        for c in span.content.chars() {
            if is_glyph_char(c) {
                flush(&mut text, &mut out);
                out.push(json!({"icon": icons::lucide(&c.to_string()), "tone": tone}));
            } else {
                text.push(c);
            }
        }
        flush(&mut text, &mut out);
    }
    out
}

fn call_json(call: &Call) -> Value {
    json!({"fn": call.function, "args": call.args})
}

fn kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Plain => "plain",
        Kind::Pr => "pr",
        Kind::Review => "review",
        Kind::Ticket => "ticket",
        Kind::Alert => "alert",
        Kind::Brief => "brief",
        Kind::Muted => "muted",
        Kind::Worktree => "worktree",
    }
}

fn row_json(row: &Row, above: &str) -> Value {
    let path = format!("{above}/{}", row.key);
    let acts: Vec<Value> = row
        .acts
        .iter()
        .map(|a| {
            let choices: Vec<Value> = a.act.choices.iter().zip(&a.picks).map(|(label, call)| json!({"label": label, "call": call_json(call)})).collect();
            json!({"label": a.act.label, "key": a.act.key.map(String::from), "main": a.act.main, "confirm": a.act.confirm, "call": a.call.as_ref().map(call_json), "choices": choices})
        })
        .collect();
    let mut out = json!({
        "key": row.key, "path": path, "kind": kind(row.kind), "section": row.section, "name": row.name, "expand": row.expand,
        "info": row.info.iter().map(spans).collect::<Vec<_>>(), "acts": acts, "primary": row.primary.as_ref().map(call_json),
        "status": row.status, "item": row.item,
        "children": row.children.iter().map(|kid| row_json(kid, &path)).collect::<Vec<_>>(),
    });
    match &row.body {
        Body::Text(line) => {
            out["text"] = Value::from(spans(line));
            // A heading: its icon, title and the counts after it.
            if above.is_empty() && line.spans.len() >= 2 && is_glyph(&line.spans[0].content) {
                out["icon"] = json!(icons::lucide(line.spans[0].content.trim()));
                out["title"] = json!(line.spans[1].content.trim());
                out["extra"] = json!(line.spans[2..].iter().map(|s| s.content.as_ref()).collect::<String>().trim());
            }
        }
        Body::Cells(cells) => {
            let lead: Vec<Value> = cells
                .lead
                .iter()
                .filter(|(g, _)| !g.trim().is_empty())
                .map(|(g, st)| json!({"icon": if is_glyph(g) { icons::lucide(g.trim()) } else { "circle" }, "tone": tone(*st)}))
                .collect();
            out["cells"] = json!({
                "lead": lead, "ref": cells.reference.0.trim(), "title": cells.title.0, "title_tone": tone(cells.title.1),
                "notes": cells.notes.0, "notes_tone": tone(cells.notes.1), "age": cells.age,
            });
        }
    }
    if row.kind == Kind::Ticket {
        out["item"] = json!({"id": row.key});
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// Actions

/// The view's own action: `forget_result [file]` drops a request's outcome once shown. The hub has the rest.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    match action {
        "forget_result" => {
            let file = args.first().and_then(Value::as_str).unwrap_or_default();
            if file.is_empty() || file.contains('/') {
                return Some(Err("no such result".into()));
            }
            let _ = std::fs::remove_file(results_dir().join(file));
            Some(Ok(String::new()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn the_rows_and_flat_lists_the_app_draws() {
        facts::freeze("2026-10-28T10:00:00Z");
        let inputs = Inputs { data: Some(fixture("inbox.json")), ci: fixture("ci.json"), brief: Some(fixture("brief.json")), ..Inputs::default() };
        let mut local = Local::fixed(HashMap::new(), Vec::new());
        let view = model_of(&inputs, &Config::from(json!({})), false, &mut local, json!({}));
        assert_eq!(view["state"], "ok");
        let keys: Vec<&str> = view["sections"].as_array().unwrap().iter().map(|s| s["key"].as_str().unwrap()).collect();
        assert_eq!(keys.last(), Some(&"other"));
        let work = view["sections"].as_array().unwrap().iter().find(|s| s["key"] == "work").unwrap();
        assert_eq!(work["icon"], "briefcase");
        let ticket = &work["children"][0]["children"][0];
        assert_eq!(ticket["kind"], "ticket");
        assert!(ticket["acts"].as_array().unwrap().iter().any(|a| a["label"] == "status" && a["choices"].as_array().unwrap().len() == 4));
        let pr = view["prs"].as_array().unwrap().first().unwrap();
        assert!(["ready", "blocked", "waiting", "draft"].contains(&pr["ready"]["state"].as_str().unwrap()));
        assert_eq!(view["counts"]["prs"], view["prs"].as_array().unwrap().len());
        let empty = model_of(&Inputs::default(), &Config::from(json!({})), true, &mut local, json!({}));
        assert_eq!((empty["state"].as_str(), empty["group"].as_str()), (Some("empty"), Some("status")));
        assert!(empty["message"].as_str().unwrap().starts_with("No data yet"));
    }

    #[test]
    fn a_refresh_within_the_minute_leaves_the_view_as_it_was() {
        facts::freeze("2026-10-28T10:00:00Z");
        let mut local = Local::fixed(HashMap::new(), Vec::new());
        let mut view_at = |ms: u64| {
            let mut data = fixture("inbox.json");
            data["at"] = json!(ms);
            let inputs = Inputs { data: Some(data), ci: fixture("ci.json"), ..Inputs::default() };
            model_of(&inputs, &Config::from(json!({})), false, &mut local, json!({}))
        };
        let minute = 1_761_645_540_000;
        assert_eq!(view_at(minute + 10_000), view_at(minute + 40_000), "the app shows when it was updated by the minute");
    }

    /// Every act label in the model, however deep its row.
    fn labels(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                for a in m.get("acts").and_then(Value::as_array).into_iter().flatten() {
                    out.extend(a["label"].as_str().map(str::to_string));
                }
                m.values().for_each(|x| labels(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| labels(x, out)),
            _ => {}
        }
    }

    #[test]
    fn without_claude_code_only_github_reruns_are_offered() {
        facts::freeze("2026-10-28T10:00:00Z");
        let mut ci = fixture("ci.json");
        let inputs = |ci: &Value| Inputs { data: Some(fixture("inbox.json")), ci: ci.clone(), ..Inputs::default() };
        let offered = |ci: &Value, config: Config| {
            let mut out = Vec::new();
            labels(&model_of(&inputs(ci), &config, false, &mut Local::fixed(HashMap::new(), Vec::new()), json!({})), &mut out);
            out
        };
        assert!(offered(&ci, Config::from(json!({}))).iter().any(|l| l == "rerun"), "the fixture has a failed run to rerun");
        assert!(!offered(&ci, Config::from(json!({})).without_claude()).iter().any(|l| l == "rerun" || l == "redeploy" || l == "done"));
        for run in ci["runs"].as_array_mut().unwrap() {
            run["slug"] = json!(format!("acme/{}", run["repo"].as_str().unwrap()));
        }
        assert!(offered(&ci, Config::from(json!({})).without_claude()).iter().any(|l| l == "rerun"));
    }
}
