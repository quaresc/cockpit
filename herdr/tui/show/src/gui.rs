//! The show views for the Herdr app's cockpit mod: `show` (one view by name, or a terminal's views), `show-block` (the
//! data of a block that runs something: a diff read, a log, a command's rows) and the answers written back for the
//! plugin to hand to the session, the same `<name>.<block>.answer` files the TUI app writes.

use std::path::{Path, PathBuf};

use cockpit_core::files;
use serde_json::{Value, json};

use crate::blueprint;
use crate::log::run;
use crate::mermaid;
use crate::patch::{File, Kind, git_diff, prepare};
use crate::value::{at, axes, cell, number, rows, rows_of, text};

/// Past this many lines a diff's rest is counted, not sent: the mod draws what it gets in one go.
const SENT_LINES: usize = 6000;
/// How many views of a terminal the panel lists.
const LISTED: usize = 8;
/// How many log lines are kept when the block does not say.
const KEPT: usize = 500;

fn dir() -> PathBuf {
    files::cache().join("show")
}

/// The folders the view reads: the views' and the cache's (a CI block follows `ci.json` and `inbox.json`).
pub fn dirs() -> Vec<PathBuf> {
    vec![dir(), files::cache()]
}

fn valid(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `{name}`: that view; `{terminal}`: the views that terminal's session showed, newest first, with the newest (or
/// `{pick}`) in full; nothing: every recent view the same way.
pub fn view(args: &Value, _doc: &dyn Fn(&str) -> Value) -> Value {
    let caches = Caches::read();
    if let Some(name) = args["name"].as_str() {
        if !valid(name) {
            return json!({"error": format!("no view named {name:?}")});
        }
        let spec = files::load(&dir().join(format!("{name}.json")));
        return json!({"views": [], "view": spec.map(|s| model(name, &s, &caches))});
    }
    let listed = listed(&dir(), args["terminal"].as_str());
    let pick = args["pick"].as_str().filter(|p| listed.iter().any(|(n, _)| n == p)).map(str::to_string).or_else(|| listed.first().map(|(n, _)| n.clone()));
    let views: Vec<Value> = listed.iter().map(|(name, spec)| summary(name, spec)).collect();
    let view = pick.and_then(|p| listed.iter().find(|(n, _)| *n == p).map(|(n, s)| model(n, s, &caches)));
    json!({"views": views, "view": view})
}

/// The views in the folder, newest first: the terminal's own when it showed any, else every one.
fn listed(dir: &Path, terminal: Option<&str>) -> Vec<(String, Value)> {
    let mut all: Vec<(String, Value)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().strip_suffix(".json")?.to_string();
            valid(&name).then(|| files::load(&e.path()).map(|s| (name, s))).flatten()
        })
        .filter(|(_, s)| s["placement"] != "browser")
        .collect();
    all.sort_by(|a, b| b.1["at"].as_f64().unwrap_or(0.0).total_cmp(&a.1["at"].as_f64().unwrap_or(0.0)));
    if let Some(terminal) = terminal.filter(|t| !t.is_empty()) {
        let mine: Vec<(String, Value)> = all.iter().filter(|(_, s)| s["terminal"].as_str() == Some(terminal)).cloned().collect();
        if !mine.is_empty() {
            all = mine;
        }
    }
    all.truncate(LISTED);
    all
}

fn summary(name: &str, spec: &Value) -> Value {
    let blocks = spec["blocks"].as_array().map_or(0, Vec::len);
    let asks = spec["blocks"].as_array().into_iter().flatten().filter(|b| matches!(b["type"].as_str(), Some("choice" | "confirm")) && b["answered"].is_null()).count();
    json!({"name": name, "title": spec["title"], "at": spec["at"], "blocks": blocks, "asks": asks, "terminal": spec["terminal"]})
}

/// The caches a CI block is drawn from.
pub struct Caches {
    runs: Value,
    inbox: Value,
}

impl Caches {
    fn read() -> Self {
        let load = |name: &str| files::load(&files::cache().join(name)).unwrap_or_default();
        Self { runs: load("ci.json"), inbox: load("inbox.json") }
    }
}

/// A view as the mod draws it: its header facts and each block's model.
pub fn model(name: &str, spec: &Value, caches: &Caches) -> Value {
    model_in(&dir(), name, spec, caches)
}

fn model_in(dir: &Path, name: &str, spec: &Value, caches: &Caches) -> Value {
    let blocks: Vec<Value> =
        spec["blocks"].as_array().into_iter().flatten().enumerate().map(|(n, b)| block(n, &blueprint::living(dir, name, b), caches)).collect();
    json!({
        "name": name,
        "title": spec["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(name),
        "at": spec["at"],
        "url": spec["url"],
        "cwd": spec["cwd"],
        "terminal": spec["terminal"],
        "blocks": blocks,
    })
}

/// One block's model: what every block carries (n, type, title, refreshed_at, error), then its own fields.
pub fn block(n: usize, spec: &Value, caches: &Caches) -> Value {
    let kind = spec["type"].as_str().unwrap_or_default();
    let mut out = json!({
        "n": n,
        "type": kind,
        "title": spec["title"].as_str().filter(|t| !t.is_empty()),
        "id": spec["id"],
        "refreshed_at": spec["refreshed_at"],
        "error": spec["error"].as_str().filter(|e| !e.is_empty()),
    });
    let own = match kind {
        "markdown" => json!({"text": spec["text"]}),
        "stats" => json!({"items": spec["items"]}),
        "table" | "chart" if spec["command"].as_str().is_some_and(|c| !c.is_empty()) => {
            json!({"live": true, "command": spec["command"], "every": spec["every"].as_f64().unwrap_or(30.0).max(1.0), "kind": spec["kind"]})
        }
        "table" => table(spec),
        "chart" => chart(spec),
        "flow" => json!({"direction": if spec["direction"] == "right" { "right" } else { "down" }, "nodes": spec["nodes"], "edges": spec["edges"]}),
        "mermaid" => {
            let source = spec["text"].as_str().unwrap_or_default();
            if mermaid::is_flowchart(source) {
                json!({"graph": "flowchart", "text": source})
            } else {
                match mermaid::sequence_of(source) {
                    Ok(seq) => json!({"graph": "sequence", "actors": actors(&seq), "messages": seq["messages"]}),
                    Err(err) => json!({"graph": "unknown", "text": source, "error": err}),
                }
            }
        }
        "sequence" => json!({"actors": actors(spec), "messages": spec["messages"]}),
        "blueprint" => json!({"graph": "flowchart", "text": spec["text"], "file": spec["file"], "updated": spec["updated"]}),
        "image" => json!({"src": picture(spec["path"].as_str().or(spec["url"].as_str())), "caption": spec["caption"]}),
        "compare" => json!({"before": picture(spec["before"].as_str()), "after": picture(spec["after"].as_str()), "caption": spec["caption"]}),
        "diff" => json!({"live": true, "repo": spec["repo"], "base": spec["base"], "patch": spec["patch"].is_string()}),
        "log" => json!({"live": true, "command": spec["command"], "every": spec["every"].as_f64().unwrap_or(5.0).max(1.0)}),
        "ci" => ci(spec, caches),
        "choice" => json!({
            "prompt": spec["prompt"],
            "answered": spec["answered"],
            "options": spec["options"].as_array().into_iter().flatten().map(|o| json!({"label": o["label"], "preview": o["preview"]})).collect::<Vec<_>>(),
        }),
        "confirm" => json!({"summary": spec["summary"], "consequence": spec["consequence"], "confirm_label": spec["confirm_label"].as_str().filter(|l| !l.is_empty()).unwrap_or("Confirm"), "answered": spec["answered"]}),
        other => json!({"unknown": other}),
    };
    if let (Some(out), Some(own)) = (out.as_object_mut(), own.as_object()) {
        out.extend(own.clone());
    }
    out
}

/// Actors as {id, label}: given ones first, then whoever a message names.
fn actors(spec: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = spec["actors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| match a.as_str() {
            Some(name) => json!({"id": name, "label": name}),
            None => json!({"id": a["id"], "label": a["label"].as_str().or(a["id"].as_str())}),
        })
        .collect();
    for m in spec["messages"].as_array().into_iter().flatten() {
        for end in [&m["from"], &m["to"], &m["over"]] {
            if let Some(id) = end.as_str()
                && !out.iter().any(|a| a["id"] == id)
            {
                out.push(json!({"id": id, "label": id}));
            }
        }
    }
    out
}

/// A picture's source and the name its copy takes in the app's folder (the app draws pictures only from there).
fn picture(src: Option<&str>) -> Value {
    let Some(src) = src.filter(|s| !s.is_empty()) else { return Value::Null };
    let ext = src.rsplit('/').next().and_then(|n| n.rsplit_once('.')).map(|(_, e)| e.to_lowercase()).filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()));
    let stamp = if src.starts_with('/') { std::fs::metadata(src).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs()) } else { 0 };
    let key = format!("{:016x}.{}", fnv(&format!("{src}@{stamp}")), ext.unwrap_or_else(|| "png".into()));
    json!({"src": src, "remote": !src.starts_with('/'), "key": key})
}

/// Columns, each cell's text and its value for sorting, and which columns hold numbers.
fn table(spec: &Value) -> Value {
    let columns: Vec<String> = spec["columns"].as_array().into_iter().flatten().map(text).collect();
    let rows = rows(spec);
    let numeric: Vec<bool> = (0..columns.len()).map(|i| rows.iter().any(|r| number(at(r, i)).is_some())).collect();
    let cells: Vec<Vec<String>> = rows.iter().map(|r| (0..columns.len()).map(|i| cell(at(r, i))).collect()).collect();
    json!({"columns": columns, "numeric": numeric, "cells": cells, "rows": rows})
}

/// The x labels and each drawn column's values (null where a row has none).
fn chart(spec: &Value) -> Value {
    let (xi, ys, names) = axes(spec);
    let rows = rows(spec);
    let labels: Vec<String> = rows.iter().map(|r| cell(at(r, xi))).collect();
    let series: Vec<Value> = ys.iter().zip(&names).map(|(&i, name)| json!({"name": name, "values": rows.iter().map(|r| number(at(r, i))).collect::<Vec<_>>()})).collect();
    let x = spec["columns"].as_array().and_then(|c| c.get(xi)).map(text).unwrap_or_default();
    let kind = spec["kind"].as_str().filter(|k| matches!(*k, "bar" | "area" | "donut")).unwrap_or("line");
    let on = |key: &str| spec[key].as_bool().unwrap_or(false);
    json!({
        "kind": kind, "x": x, "labels": labels, "series": series,
        "horizontal": on("horizontal"), "stacked": on("stacked") || on("percent"), "percent": on("percent"), "show_values": on("show_values"),
    })
}

/// A branch's CI run and PR from the caches the inbox keeps.
fn ci(spec: &Value, caches: &Caches) -> Value {
    let (repo, branch) = (&spec["repo"], &spec["branch"]);
    let same = |v: &&Value| v["repo"] == *repo && v["branch"] == *branch;
    let run = caches.runs["runs"].as_array().and_then(|r| r.iter().find(same)).cloned();
    let pr = caches.inbox["mine"].as_array().and_then(|p| p.iter().find(same)).map(|p| json!({"number": p["number"], "title": p["title"], "url": p["url"]}));
    let run = run.map(|run| {
        let outcome = run["outcome"].as_str().unwrap_or_default();
        let status = if run["isRunning"].as_bool().unwrap_or(false) {
            "running"
        } else if matches!(outcome, "success" | "succeeded") {
            "passed"
        } else {
            "failed"
        };
        json!({
            "status": status,
            "outcome": outcome,
            "total": run["total"].as_u64().unwrap_or(1).max(1),
            "done": run["done"].as_u64().unwrap_or(0),
            "failed": run["failed"],
            "url": run["url"],
        })
    });
    json!({"repo": repo, "branch": branch, "pr": pr, "run": run})
}

/// FNV-1a: a stable 64-bit hash for names on disk and hunk ids, the same in every build.
pub fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
}

// ---------------------------------------------------------------------------------------------------------------------
// show-block: the blocks that run something, asked for by the mod on their own clock

/// `{name, n, since}`: a diff's files (`{same: true}` when its patch hashes to `since`), a log's lines, a command's rows.
pub fn block_data(args: &Value, _doc: &dyn Fn(&str) -> Value) -> Value {
    let name = args["name"].as_str().unwrap_or_default();
    if !valid(name) {
        return json!({"error": "no such view"});
    }
    let Some(spec) = files::load(&dir().join(format!("{name}.json"))) else { return json!({"error": "the view is gone"}) };
    let n = args["n"].as_u64().unwrap_or(0) as usize;
    let Some(block) = spec["blocks"].get(n) else { return json!({"error": "no such block"}) };
    let cwd = spec["cwd"].as_str().map_or_else(files::home, PathBuf::from);
    let cwd = if cwd.is_dir() { cwd } else { files::home() };
    match block["type"].as_str() {
        Some("diff") => {
            let patch = match block["patch"].as_str() {
                Some(patch) => Ok(patch.to_string()),
                None => git_diff(block["repo"].as_str().unwrap_or_default(), block["base"].as_str()),
            };
            match patch {
                Ok(patch) => diff_model(&patch, args["since"].as_str()),
                Err(error) => json!({"error": error}),
            }
        }
        Some("log") => {
            let (output, code) = run(block["command"].as_str().unwrap_or_default(), &cwd);
            let keep = block["lines"].as_u64().map_or(KEPT, |n| n as usize);
            json!({"lines": log_lines(&output, keep), "code": code, "at": now_ms()})
        }
        Some("table" | "chart") => {
            let (output, code) = run(block["command"].as_str().unwrap_or_default(), &cwd);
            match rows_of(&output) {
                Ok((columns, rows)) => {
                    let mut fed = block.clone();
                    (fed["columns"], fed["rows"]) = (columns, rows);
                    fed.as_object_mut().map(|o| o.remove("command"));
                    let mut out = self::block(n, &fed, &Caches { runs: Value::Null, inbox: Value::Null });
                    out["at"] = json!(now_ms());
                    out
                }
                Err(err) => {
                    let shown: String = output.trim().chars().take(160).collect();
                    json!({"error": format!("exit {code}, {err}: {shown}")})
                }
            }
        }
        _ => json!({"error": "this block runs nothing"}),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// A command's output as lines without terminal colours, tabs as spaces, the last `keep` of them, each with its level.
pub fn log_lines(output: &str, keep: usize) -> Vec<Value> {
    static ANSI: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07").unwrap());
    static ERROR: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\b(error|fail(ed|ure)?|exception|fatal)\b").unwrap());
    static WARN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bwarn(ing)?\b").unwrap());
    let clean = ANSI.replace_all(output, "");
    let lines: Vec<&str> = clean.trim_end_matches('\n').lines().collect();
    lines[lines.len().saturating_sub(keep)..]
        .iter()
        .map(|l| {
            let level = if ERROR.is_match(l) { "error" } else if WARN.is_match(l) { "warn" } else { "" };
            json!([l.replace('\t', "    "), level])
        })
        .collect()
}

/// A patch as the mod draws it: per file its path, counts, hunks (stable ids) and lines `[kind, old, new, pieces]`, a
/// piece `[text, class, changed]` with class a syntax role (`k`eyword, `s`tring, `f`unction, `t`ype, `c`omment, `p`aram).
pub fn diff_model(patch: &str, since: Option<&str>) -> Value {
    let hash = format!("{:016x}", fnv(patch));
    if since == Some(hash.as_str()) {
        return json!({"same": true, "hash": hash});
    }
    let files = prepare(patch);
    let (mut sent, mut out) = (0usize, Vec::new());
    for file in &files {
        out.push(file_model(file, &mut sent));
    }
    let (added, removed) = files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
    json!({"hash": hash, "files": out, "added": added, "removed": removed})
}

fn kind_mark(kind: Kind) -> &'static str {
    match kind {
        Kind::Hunk => "@",
        Kind::Add => "+",
        Kind::Del => "-",
        Kind::Same => " ",
        Kind::Note => "!",
    }
}

/// A hunk's id: its file and a hash of its lines (not of where they are), so a hunk keeps its id when lines above it
/// move; byte-identical hunks in one file are told apart by occurrence.
pub fn hunk_ids(file: &File) -> Vec<(usize, usize, String)> {
    let mut out: Vec<(usize, usize, String)> = Vec::new();
    let starts: Vec<usize> = file.lines.iter().enumerate().filter(|(_, l)| l.kind == Kind::Hunk).map(|(i, _)| i).collect();
    for (k, &start) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(file.lines.len());
        let body: String = file.lines[start + 1..end].iter().map(|l| format!("{}{}\n", kind_mark(l.kind), l.text)).collect();
        let digest = format!("{:08x}", fnv(&body) & 0xffff_ffff);
        let id = format!("{}:{digest}", file.path);
        let seen = out.iter().filter(|(_, _, i)| i.starts_with(&id)).count();
        out.push((start, end, if seen > 0 { format!("{id}-{}", seen + 1) } else { id }));
    }
    out
}

fn file_model(file: &File, sent: &mut usize) -> Value {
    let room = SENT_LINES.saturating_sub(*sent);
    let shown = file.lines.len().min(room);
    *sent += shown;
    let lines: Vec<Value> = file.lines[..shown]
        .iter()
        .map(|l| {
            let pieces: Vec<Value> = match l.kind {
                Kind::Hunk | Kind::Note => vec![json!([l.text, "", false])],
                _ if l.paint.is_empty() => vec![json!([l.text.replace('\t', "    "), "", false])],
                _ => l.paint.iter().map(|(t, style, changed)| json!([t, crate::highlight::class_of(style), changed])).collect(),
            };
            json!([kind_mark(l.kind), l.old, l.new, pieces])
        })
        .collect();
    let hunks: Vec<Value> = hunk_ids(file).into_iter().map(|(start, end, id)| json!({"id": id, "at": start, "end": end, "header": file.lines[start].text})).collect();
    json!({"path": file.path, "added": file.added, "removed": file.removed, "hunks": hunks, "lines": lines, "more": file.lines.len() - shown})
}

// ---------------------------------------------------------------------------------------------------------------------
// Actions

/// `answer [name, n, type, value, label]` writes a choice, confirm, comment or review answer for the session.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    act_in(&dir(), action, args)
}

pub fn act_in(dir: &Path, action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let name = args.first().and_then(Value::as_str).unwrap_or_default();
    (action == "answer").then(|| answer(dir, name, args))
}

fn answer(dir: &Path, name: &str, args: &[Value]) -> Result<String, String> {
    if !valid(name) {
        return Err(format!("no view named {name:?}"));
    }
    let n = args.get(1).and_then(Value::as_u64).ok_or("which block?")? as usize;
    let kind = args.get(2).and_then(Value::as_str).unwrap_or_default();
    if !matches!(kind, "choice" | "confirm" | "comment" | "review" | "blueprint") {
        return Err(format!("not an answer: {kind}"));
    }
    let spec = files::load(&dir.join(format!("{name}.json"))).ok_or("the view is gone")?;
    let block = spec["blocks"].get(n).ok_or("no such block")?;
    if matches!(kind, "choice" | "confirm") && block["answered"].is_string() {
        return Err("already answered".into());
    }
    let path = dir.join(format!("{name}.{n}.answer"));
    if path.exists() {
        return Err("the session has not taken the last answer yet; try again in a moment".into());
    }
    let label = args.get(4).and_then(Value::as_str).unwrap_or_default();
    let value = args.get(3).cloned().unwrap_or(Value::Null);
    let at = now_ms() as f64 / 1000.0;
    let mut answer = json!({"block": n, "type": kind, "value": value, "label": label, "at": at});
    if matches!(kind, "choice" | "confirm") {
        answer["id"] = block.get("id").cloned().unwrap_or(Value::Null);
    }
    files::write(&path, &answer).map_err(|e| format!("the answer was not written: {e}"))?;
    Ok(match kind {
        "comment" => format!("comment on {label} sent to the session"),
        "review" => "review sent to the session".into(),
        "blueprint" => format!("change request about «{label}» sent to the session"),
        _ => format!("answered: {label}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        files::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.json"))).unwrap()
    }

    fn no_caches() -> Caches {
        Caches { runs: Value::Null, inbox: Value::Null }
    }

    #[test]
    fn every_fixture_block_has_a_model() {
        for name in ["chart_bar", "chart_line", "choice", "code", "compare", "confirm", "diff", "flow", "image", "log", "markdown", "mermaid", "mixed", "sequence", "table"] {
            let m = model(name, &fixture(name), &no_caches());
            for b in m["blocks"].as_array().unwrap() {
                assert!(b["unknown"].is_null(), "{name}: {b}");
                assert!(b["type"].is_string());
            }
        }
    }

    #[test]
    fn tables_and_charts_carry_their_cells_and_series() {
        let t = block(0, &json!({"type": "table", "columns": ["plan", "orgs"], "rows": [["free", 1234], ["pro", 12.5]]}), &no_caches());
        assert_eq!(t["numeric"], json!([false, true]));
        assert_eq!(t["cells"][0][1], "1,234");
        let c = block(0, &json!({"type": "chart", "kind": "line", "columns": ["week", "a", "b"], "rows": [["w1", 1, null], ["w2", 3, 4]]}), &no_caches());
        assert_eq!(c["labels"], json!(["w1", "w2"]));
        assert_eq!(c["series"][1], json!({"name": "b", "values": [null, 4.0]}));
    }

    #[test]
    fn a_chart_keeps_its_shape_and_figures_their_items() {
        let rows = json!([["fr", 9332, 1359], ["en", 7758, 932]]);
        let c = block(0, &json!({"type": "chart", "kind": "bar", "horizontal": true, "percent": true, "columns": ["lang", "ok", "ko"], "rows": rows}), &no_caches());
        assert_eq!((c["kind"].as_str(), c["horizontal"].as_bool(), c["stacked"].as_bool(), c["percent"].as_bool()), (Some("bar"), Some(true), Some(true), Some(true)));
        let d = block(0, &json!({"type": "chart", "kind": "donut", "columns": ["lang", "n"], "rows": [["fr", 3], ["en", 1]]}), &no_caches());
        assert_eq!(d["kind"], "donut");
        let odd = block(0, &json!({"type": "chart", "kind": "pie", "columns": ["a", "b"], "rows": [["x", 1]]}), &no_caches());
        assert_eq!(odd["kind"], "line");
        let s = block(0, &json!({"type": "stats", "items": [{"label": "Sign-ups", "value": 21234, "delta": "-2%", "good": false}]}), &no_caches());
        assert_eq!(s["items"][0]["value"], 21234);
    }

    #[test]
    fn mermaid_sequences_become_actors_and_messages() {
        let b = block(0, &json!({"type": "mermaid", "text": "sequenceDiagram\n  A->>B: hi\n  B-->>A: ok"}), &no_caches());
        assert_eq!(b["graph"], "sequence");
        assert_eq!(b["actors"].as_array().unwrap().len(), 2);
        let f = block(0, &json!({"type": "mermaid", "text": "flowchart LR\n A-->B"}), &no_caches());
        assert_eq!(f["graph"], "flowchart");
    }

    #[test]
    fn a_ci_block_reads_the_caches() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let caches = Caches { runs: files::load(&dir.join("ci-cache.json")).unwrap(), inbox: files::load(&dir.join("inbox-cache.json")).unwrap() };
        let b = block(0, &fixture("ci")["blocks"][0], &caches);
        assert_eq!(b["run"]["status"], "failed");
        assert_eq!(b["run"]["failed"]["job"], "test");
        assert_eq!(b["pr"]["number"], 2198);
    }

    #[test]
    fn a_diff_comes_in_files_with_stable_hunk_ids() {
        let patch = "diff --git a/q.ts b/q.ts\n--- a/q.ts\n+++ b/q.ts\n@@ -1,3 +1,3 @@\n /** The quota. */\n-const limit = 100;\n+const limit = 200;\n";
        let m = diff_model(patch, None);
        let file = &m["files"][0];
        assert_eq!(file["path"], "q.ts");
        assert_eq!(file["lines"][2][0], "-");
        assert!(file["lines"][3][3].as_array().unwrap().iter().any(|p| p[2] == true));
        let moved = patch.replace("@@ -1,3 +1,3 @@", "@@ -10,3 +10,3 @@");
        assert_eq!(diff_model(&moved, None)["files"][0]["hunks"][0]["id"], file["hunks"][0]["id"]);
        assert_eq!(diff_model(patch, m["hash"].as_str()), json!({"same": true, "hash": m["hash"]}));
    }

    #[test]
    fn logs_lose_their_colours_and_keep_the_last_lines() {
        let lines = log_lines("\x1b[31merror: x\x1b[0m\nok\nwarning: y\n", 2);
        assert_eq!(lines, vec![json!(["ok", ""]), json!(["warning: y", "warn"])]);
    }

    #[test]
    fn answers_are_written_once_and_not_twice_at_a_time() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        files::write(&dir.join("q.json"), &json!({"title": "Q", "blocks": [{"type": "choice", "options": [{"label": "a"}, {"label": "b"}]}]})).unwrap();
        let ok = act_in(dir, "answer", &[json!("q"), json!(0), json!("choice"), json!("a"), json!("a")]).unwrap();
        assert_eq!(ok, Ok("answered: a".into()));
        assert_eq!(files::load(&dir.join("q.0.answer")).unwrap()["value"], "a");
        assert!(act_in(dir, "answer", &[json!("q"), json!(0), json!("choice"), json!("b"), json!("b")]).unwrap().is_err());
        assert!(act_in(dir, "answer", &[json!("../x"), json!(0), json!("choice")]).unwrap().is_err());
        assert!(act_in(dir, "nope", &[]).is_none());
    }

    #[test]
    fn a_blueprint_draws_its_file_and_takes_change_requests() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        let spec = json!({"title": "Plan", "at": 1, "blocks": [{"type": "blueprint", "text": "flowchart TD\n A --> B"}]});
        files::write(&dir.join("plan.json"), &spec).unwrap();
        std::fs::write(dir.join("plan.mmd"), "flowchart TD\n A[Read] --> B[Write]\n class A done").unwrap();
        let b = &model_in(dir, "plan", &spec, &no_caches())["blocks"][0];
        assert_eq!(b["graph"], "flowchart");
        assert!(b["text"].as_str().unwrap().contains("class A done"));
        assert!(b["updated"].is_u64());
        let ask = |text: &str| act_in(dir, "answer", &[json!("plan"), json!(0), json!("blueprint"), json!({"node": "A", "label": "Read", "text": text}), json!("Read")]).unwrap();
        assert_eq!(ask("split it"), Ok("change request about «Read» sent to the session".into()));
        std::fs::remove_file(dir.join("plan.0.answer")).unwrap();
        assert!(ask("and test it").is_ok());
    }

    #[test]
    fn a_terminal_lists_its_own_views_newest_first() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        files::write(&dir.join("a.json"), &json!({"title": "A", "at": 1, "terminal": "t1", "blocks": []})).unwrap();
        files::write(&dir.join("b.json"), &json!({"title": "B", "at": 2, "terminal": "t2", "blocks": []})).unwrap();
        files::write(&dir.join("c.json"), &json!({"title": "C", "at": 3, "terminal": "t1", "blocks": []})).unwrap();
        let names = |t: Option<&str>| listed(dir, t).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(Some("t1")), ["c", "a"]);
        assert_eq!(names(Some("t9")), ["c", "b", "a"]);
    }
}
