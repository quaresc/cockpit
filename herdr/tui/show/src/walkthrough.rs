//! The guided walkthrough of a diff: the hunks read by `claude -p` in the background, grouped into steps in reading
//! order with what each changes, then laid over the diff as it is now (a step whose code moved is outdated, a hunk no
//! step covers is listed). The prompt, schema and checks follow dmazlum/herdr-walkthrough (MIT).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cockpit_core::files;
use serde_json::{Value, json};

use crate::gui::{fnv, hunk_ids};
use crate::patch::{Kind, parse_patch};

/// Part of every key: another prompt writes another walkthrough.
const PROMPT_VERSION: u32 = 1;
/// A digest past this many characters (about 175k tokens) is refused rather than cut.
const MAX_DIGEST: usize = 600_000;
const MODELS: [&str; 3] = ["haiku", "sonnet", "opus"];
const LIMIT_PARTS: usize = 6;
const LIMIT_STEPS: usize = 16;
const LIMIT_HUNKS: usize = 14;
const LIMIT_NOTES: usize = 6;
const WEIGHTS: [&str; 3] = ["key", "normal", "skim"];
const NOTE_KINDS: [&str; 5] = ["choice", "upkeep", "gap", "divergence", "untested"];

const SYSTEM: &str = "You are guiding an engineer through a code change they did not write. It may come from a teammate, a contributor or a coding agent. Help them read it in the right order and understand what it does and what they accept by taking it. You are not grading it.

Produce three things.

1. An order. A diff lists files alphabetically, which is rarely how a change is best understood. Gather the hunks that serve one purpose, across files, into a step, and order the steps so each one can be followed with only what came before it: types, schemas and entry points before the code that uses them. A test goes with the code it exercises, unless the change is mostly tests. Group the steps into a few parts.

2. An explanation. For each step, two to four plain sentences on how behaviour differs after the change and why these hunks belong together. The reader sees the code beside your text, so do not narrate the edits (\"adds a parameter\", \"renames foo to bar\", \"updates imports\"). Describe, never instruct: no \"make sure\", \"verify\", \"double-check\", \"consider\", \"be careful\".

3. Notes, only where they earn their place. A note records something the reader signs up for by accepting the change, which the explanation alone would let slip by. Each note has exactly one kind:
- \"choice\": a product or design decision that could reasonably have gone another way.
- \"upkeep\": something the project maintains from now on, such as a new dependency, a build step, or a second copy of logic that already exists.
- \"gap\": something users still cannot do, or will see fail, after the change.
- \"divergence\": two paths for the same thing that now behave differently for users.
- \"untested\": the main thing the change promises is exercised by no test or check.
A note is one or two sentences about a concrete consequence, resting on what that step's own hunks show. It is never a suspected bug, a remark on style, a suggestion, or an instruction. Most steps have no notes.

The summary says what the change means for the people who use the software: what they can do now, or no longer run into. When nothing changes for users, say what it changes for the people working on the code.

Rules:
- Point at hunks only by the aliases in square brackets in the digest (h1, h2, ...), exactly as written. Never invent one.
- Use each hunk at most once, in the step where it matters most.
- Leave purely mechanical hunks out rather than padding a step with them.
- skipped: for every file with hunks you left out, one entry {path, note}, the path exactly as in the digest, the note one short line on what changed there.
- weight: \"key\" only for the one or two steps the rest depends on; \"skim\" for supporting changes that are safe to skim; \"normal\" otherwise.
- Titles name what a step is about, never the act of reading it.
- Write plain sentences everywhere: no markdown, no lists, no code fences.

Answer with a single JSON object and nothing else.";

fn schema() -> Value {
    let s = json!({"type": "string"});
    let note = json!({"type": "object", "additionalProperties": false, "required": ["kind", "text"], "properties": {"kind": {"type": "string", "enum": NOTE_KINDS}, "text": s}});
    let step = json!({"type": "object", "additionalProperties": false, "required": ["title", "hunks", "weight", "text", "notes"], "properties": {
        "title": s, "hunks": {"type": "array", "items": s}, "weight": {"type": "string", "enum": WEIGHTS}, "text": s, "notes": {"type": "array", "items": note}}});
    let part = json!({"type": "object", "additionalProperties": false, "required": ["title", "blurb", "steps"], "properties": {"title": s, "blurb": s, "steps": {"type": "array", "items": step}}});
    let skipped = json!({"type": "object", "additionalProperties": false, "required": ["path", "note"], "properties": {"path": s, "note": s}});
    json!({"type": "object", "additionalProperties": false, "required": ["title", "summary", "parts", "skipped"], "properties": {
        "title": s, "summary": s, "parts": {"type": "array", "items": part}, "skipped": {"type": "array", "items": skipped}}})
}

fn root() -> PathBuf {
    files::cache().join("walkthrough")
}

/// Lockfiles and bundles: listed, never sent.
fn generated(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(name, "Cargo.lock" | "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml" | "mix.lock" | "Gemfile.lock" | "poetry.lock" | "go.sum" | "composer.lock")
        || name.ends_with(".min.js")
        || name.ends_with(".snap")
        || path.contains("/dist/")
}

/// What the model reads: every hunk under an alias, and the aliases' hunk ids.
pub struct Digest {
    pub text: String,
    pub aliases: HashMap<String, String>,
    pub files: Vec<(String, Vec<String>)>,
    pub hunks: usize,
}

pub fn digest(patch: &str) -> Digest {
    let (mut blocks, mut aliases, mut listed, mut n) = (Vec::new(), HashMap::new(), Vec::new(), 0);
    for file in parse_patch(patch) {
        if generated(&file.path) {
            continue;
        }
        let mut body = vec![format!("=== {}", file.path)];
        let mut mine = Vec::new();
        for (start, end, id) in hunk_ids(&file) {
            n += 1;
            let alias = format!("h{n}");
            body.push(format!("[{alias}] {}", file.lines[start].text));
            body.extend(file.lines[start + 1..end].iter().map(|l| {
                let mark = match l.kind {
                    Kind::Add => "+",
                    Kind::Del => "-",
                    Kind::Note => "",
                    _ => " ",
                };
                format!("{mark}{}", l.text)
            }));
            aliases.insert(alias.clone(), id);
            mine.push(alias);
        }
        blocks.push(body.join("\n"));
        listed.push((file.path.clone(), mine));
    }
    Digest { text: blocks.join("\n\n"), aliases, files: listed, hunks: n }
}

fn prompt(d: &Digest, what: &str) -> String {
    let steps = ((d.hunks as f64 / 2.5).round() as usize).clamp(1, LIMIT_STEPS);
    let parts = if d.hunks <= 4 { 1 } else { steps.div_ceil(3).clamp(1, LIMIT_PARTS) };
    let key = if steps <= 3 { 1 } else if steps <= 9 { 2 } else { 3 };
    let notes = steps.div_ceil(3).clamp(1, LIMIT_NOTES);
    format!(
        "The change: {what}\n\nIt has {} file(s) and {} hunk(s) to read. Aim for about {steps} step(s) in about {parts} part(s); never more than {LIMIT_STEPS} steps, {LIMIT_PARTS} parts or {LIMIT_HUNKS} hunks in one step. Fewer, fuller steps beat many thin ones.\nMark at most {key} step(s) \"key\". Write at most {notes} note(s) in the whole walkthrough; none is a fine answer.\nPart titles show in a narrow column: one to three words, at most 28 characters.\n\nThe digest: each file starts with \"=== path\", each hunk with its alias in square brackets and its @@ header, then its lines.\n\n{}",
        d.files.len(),
        d.hunks,
        d.text
    )
}

fn place(repo: &str, scope: &str) -> String {
    format!("{:016x}", fnv(&format!("{repo}\n{scope}")))
}

fn key_of(d: &Digest, model: &str) -> String {
    format!("{:016x}", fnv(&format!("{PROMPT_VERSION}\n{model}\n{}", d.text)))
}

/// The script a job runs: claude with no tools, MCP servers, settings or skills, stopped after `$2` seconds.
const RUN: &str = r#"#!/bin/sh
cd "$(dirname "$0")" || exit 1
claude -p --output-format json --tools '' --no-session-persistence --strict-mcp-config --setting-sources '' --disable-slash-commands --model "$1" --json-schema "$(cat schema.json)" --system-prompt "$(cat system.txt)" < prompt.txt > out.json 2> err.txt &
pid=$!
echo $pid > pid
( waited=0
  while kill -0 $pid 2>/dev/null; do
    if [ $waited -ge "${2:-900}" ]; then echo "Stopped after $2 seconds without an answer." >> err.txt; kill $pid; exit 0; fi
    sleep 3; waited=$((waited + 3))
  done ) &
wait $pid
echo $? > code.tmp && mv code.tmp code
"#;

/// Starts a walkthrough of the patch in the background, unless one of the same changes and model is there or running.
pub fn start(repo: &str, scope: &str, model: &str, patch: &str, about: &Value) -> Result<String, String> {
    start_in(&root(), repo, scope, model, patch, about)
}

pub fn start_in(root: &Path, repo: &str, scope: &str, model: &str, patch: &str, about: &Value) -> Result<String, String> {
    let model = if MODELS.contains(&model) { model } else { "haiku" };
    let d = digest(patch);
    if d.hunks == 0 {
        return Err("no changes to walk through".into());
    }
    if d.text.len() > MAX_DIGEST {
        return Err("these changes are too large for one walkthrough: narrow the scope".into());
    }
    let key = key_of(&d, model);
    let pointer = json!({"key": key, "model": model, "at": now_ms()});
    let err = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(root.join("pointers")).map_err(err)?;
    files::write(&root.join("pointers").join(format!("{}.json", place(repo, scope))), &pointer).map_err(err)?;
    if root.join("entries").join(format!("{key}.json")).exists() {
        return Ok("this walkthrough is already there".into());
    }
    let job = root.join("jobs").join(&key);
    if job.exists() && !job.join("code").exists() {
        return Ok("the walkthrough is already being written".into());
    }
    let _ = std::fs::remove_dir_all(&job);
    std::fs::create_dir_all(&job).map_err(err)?;
    let what = match scope {
        "branch" => format!("the branch {} against {}", about["branch"].as_str().unwrap_or("?"), about["base"].as_str().unwrap_or("its base")),
        "pr" => format!("pull request #{} \"{}\"", about["pr"]["number"], about["pr"]["title"].as_str().unwrap_or_default()),
        _ => "uncommitted changes in a working tree".into(),
    };
    std::fs::write(job.join("system.txt"), SYSTEM).map_err(err)?;
    std::fs::write(job.join("prompt.txt"), prompt(&d, &what)).map_err(err)?;
    std::fs::write(job.join("schema.json"), schema().to_string()).map_err(err)?;
    std::fs::write(job.join("run.sh"), RUN).map_err(err)?;
    let aliases: Value = d.aliases.iter().map(|(a, id)| (a.clone(), json!(id))).collect::<serde_json::Map<_, _>>().into();
    let listed: Vec<Value> = d.files.iter().map(|(p, a)| json!({"path": p, "aliases": a})).collect();
    files::write(&job.join("meta.json"), &json!({"aliases": aliases, "files": listed, "model": model, "repo": repo, "scope": scope, "at": now_ms()})).map_err(err)?;
    let mut command = Command::new("sh");
    command.arg(job.join("run.sh")).args([model, "900"]).current_dir(&job).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // A nested claude must not think it runs inside the session that asked.
    for var in ["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT"] {
        command.env_remove(var);
    }
    let mut child = command.spawn().map_err(|e| format!("the walkthrough did not start: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(format!("writing the walkthrough with {model}"))
}

/// Stops the scope's running walkthrough.
pub fn stop(repo: &str, scope: &str) -> Result<String, String> {
    let root = root();
    let pointer = files::load(&root.join("pointers").join(format!("{}.json", place(repo, scope)))).ok_or("no walkthrough here")?;
    let job = root.join("jobs").join(pointer["key"].as_str().unwrap_or_default());
    let pid = std::fs::read_to_string(job.join("pid")).map_err(|_| "it is not running")?;
    let _ = Command::new("kill").arg(pid.trim()).status();
    let _ = std::fs::remove_dir_all(&job);
    Ok("walkthrough stopped".into())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The scope's walkthrough over the patch: none, running, failed, or its steps laid over the current hunks.
pub fn state(repo: &str, scope: &str, patch: &str) -> Value {
    state_in(&root(), repo, scope, patch)
}

pub fn state_in(root: &Path, repo: &str, scope: &str, patch: &str) -> Value {
    let Some(pointer) = files::load(&root.join("pointers").join(format!("{}.json", place(repo, scope)))) else { return json!({"status": "none"}) };
    let key = pointer["key"].as_str().unwrap_or_default();
    let model = pointer["model"].as_str().unwrap_or("haiku");
    let fresh = key_of(&digest(patch), model) == key;
    let entry_path = root.join("entries").join(format!("{key}.json"));
    let entry = match files::load(&entry_path) {
        Some(entry) => entry,
        None => {
            let job = root.join("jobs").join(key);
            if !job.join("code").exists() {
                return if job.exists() { json!({"status": "running", "model": model, "since": pointer["at"]}) } else { json!({"status": "none"}) };
            }
            match finish(&job) {
                Ok(entry) => {
                    let _ = files::write(&entry_path, &entry);
                    let _ = std::fs::remove_dir_all(&job);
                    entry
                }
                Err(error) => return json!({"status": "failed", "error": error, "model": model}),
            }
        }
    };
    let mut out = overlay(&entry, patch);
    out["status"] = json!("done");
    out["fresh"] = json!(fresh);
    out["model"] = json!(model);
    out
}

/// A finished job's answer, checked: every step pointing at hunks that were in the digest.
fn finish(job: &Path) -> Result<Value, String> {
    let read = |name: &str| std::fs::read_to_string(job.join(name)).unwrap_or_default();
    let answer: Value = serde_json::from_str(&read("out.json")).map_err(|_| {
        let why: Vec<String> = read("err.txt").lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect();
        if why.is_empty() { format!("claude exited with {}", read("code").trim()) } else { why[why.len().saturating_sub(3)..].join(" ") }
    })?;
    if answer["is_error"] == true || answer["subtype"].as_str().is_some_and(|s| s != "success") {
        return Err(answer["result"].as_str().filter(|r| !r.trim().is_empty()).unwrap_or("claude reported an error").trim().to_string());
    }
    let raw = if answer["structured_output"].is_object() { answer["structured_output"].clone() } else { parse_json(answer["result"].as_str().unwrap_or_default())? };
    let meta = files::load(&job.join("meta.json")).unwrap_or_default();
    let mut entry = normalize(&raw, &meta)?;
    entry["cost"] = answer["total_cost_usd"].clone();
    entry["at"] = json!(now_ms());
    Ok(entry)
}

/// The JSON object in a text answer, also when fenced.
fn parse_json(text: &str) -> Result<Value, String> {
    let t = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    if let Ok(v) = serde_json::from_str(t) {
        return Ok(v);
    }
    let start = t.find('{').ok_or("the model's answer was not JSON")?;
    let end = t.rfind('}').ok_or("the model's answer was not JSON")?;
    serde_json::from_str(&t[start..=end]).map_err(|_| "the model's answer was not JSON".into())
}

fn clip(value: &Value, max: usize) -> String {
    let t = value.as_str().unwrap_or_default().trim();
    if max > 0 && t.chars().count() > max { t.chars().take(max - 1).collect::<String>() + "…" } else { t.to_string() }
}

/// The model's answer made safe: aliases it invented are dropped, each hunk is told once, the limits hold.
pub fn normalize(raw: &Value, meta: &Value) -> Result<Value, String> {
    let alias = |a: &Value| meta["aliases"][a.as_str().unwrap_or_default().trim().trim_matches(['[', ']'])].as_str().map(str::to_string);
    let (mut used, mut steps, mut notes, mut parts) = (HashSet::new(), 0, 0, Vec::new());
    for part in raw["parts"].as_array().into_iter().flatten() {
        if parts.len() >= LIMIT_PARTS {
            break;
        }
        let mut mine = Vec::new();
        for step in part["steps"].as_array().into_iter().flatten() {
            if steps >= LIMIT_STEPS {
                break;
            }
            let mut hunks: Vec<String> = Vec::new();
            for id in step["hunks"].as_array().into_iter().flatten().filter_map(alias) {
                if !used.contains(&id) && !hunks.contains(&id) && hunks.len() < LIMIT_HUNKS {
                    hunks.push(id);
                }
            }
            let text = clip(&step["text"], 0);
            if hunks.is_empty() || text.is_empty() {
                continue;
            }
            used.extend(hunks.iter().cloned());
            let mut kept = Vec::new();
            for note in step["notes"].as_array().into_iter().flatten() {
                let (kind, t) = (note["kind"].as_str().unwrap_or_default(), clip(&note["text"], 0));
                if kept.len() < 2 && notes < LIMIT_NOTES && NOTE_KINDS.contains(&kind) && !t.is_empty() {
                    kept.push(json!({"kind": kind, "text": t}));
                    notes += 1;
                }
            }
            steps += 1;
            let weight = step["weight"].as_str().filter(|w| WEIGHTS.contains(w)).unwrap_or("normal");
            let title = Some(clip(&step["title"], 120)).filter(|t| !t.is_empty()).unwrap_or_else(|| format!("Step {steps}"));
            mine.push(json!({"id": format!("s{steps}"), "title": title, "weight": weight, "text": text, "notes": kept, "hunks": hunks}));
        }
        if !mine.is_empty() {
            let title = Some(clip(&part["title"], 28)).filter(|t| !t.is_empty()).unwrap_or_else(|| format!("Part {}", parts.len() + 1));
            parts.push(json!({"title": title, "blurb": clip(&part["blurb"], 0), "steps": mine}));
        }
    }
    if parts.is_empty() {
        return Err("the model's answer had no usable steps for these changes".into());
    }
    let left_out: HashSet<&str> = meta["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["aliases"].as_array().into_iter().flatten().any(|a| !alias(a).is_some_and(|id| used.contains(&id))))
        .filter_map(|f| f["path"].as_str())
        .collect();
    let mut noted = HashSet::new();
    let skipped: Vec<Value> = raw["skipped"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let (path, note) = (clip(&s["path"], 0), clip(&s["note"], 200));
            (!note.is_empty() && left_out.contains(path.as_str()) && noted.insert(path.clone())).then(|| json!({"path": path, "note": note}))
        })
        .collect();
    Ok(json!({"title": Some(clip(&raw["title"], 140)).filter(|t| !t.is_empty()).unwrap_or_else(|| "Walkthrough".into()), "summary": clip(&raw["summary"], 0), "parts": parts, "skipped": skipped}))
}

/// The walkthrough over the diff as it is now: each step's hunks still found, outdated when one is gone, and every
/// hunk no step covers, file by file, with the model's note for its file.
pub fn overlay(entry: &Value, patch: &str) -> Value {
    let files = parse_patch(patch);
    let ids: HashSet<String> = files.iter().flat_map(|f| hunk_ids(f).into_iter().map(|(_, _, id)| id)).collect();
    let mut covered = HashSet::new();
    let mut outdated = 0;
    let parts: Vec<Value> = entry["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|part| {
            let steps: Vec<Value> = part["steps"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|step| {
                    let (found, missing): (Vec<&Value>, Vec<&Value>) = step["hunks"].as_array().into_iter().flatten().partition(|h| h.as_str().is_some_and(|id| ids.contains(id)));
                    covered.extend(found.iter().filter_map(|h| h.as_str()).map(str::to_string));
                    outdated += usize::from(!missing.is_empty());
                    let mut out = step.clone();
                    out["hunks"] = json!(found);
                    out["missing"] = json!(missing.len());
                    out["outdated"] = json!(!missing.is_empty());
                    out
                })
                .collect();
            json!({"title": part["title"], "blurb": part["blurb"], "steps": steps})
        })
        .collect();
    let notes: HashMap<&str, &str> = entry["skipped"].as_array().into_iter().flatten().filter_map(|s| Some((s["path"].as_str()?, s["note"].as_str()?))).collect();
    let uncovered: Vec<Value> = files
        .iter()
        .filter_map(|f| {
            let hunks: Vec<String> = hunk_ids(f).into_iter().map(|(_, _, id)| id).filter(|id| !covered.contains(id)).collect();
            let note = notes.get(f.path.as_str()).map(|n| n.to_string()).unwrap_or_else(|| if generated(&f.path) { "Generated file, not sent to the model.".into() } else { String::new() });
            (!hunks.is_empty()).then(|| json!({"path": f.path, "hunks": hunks, "note": note}))
        })
        .collect();
    json!({"title": entry["title"], "summary": entry["summary"], "parts": parts, "uncovered": uncovered, "outdated": outdated, "cost": entry["cost"], "at": entry["at"]})
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n fn a() {}\n-let x = 1;\n+let x = 2;\n@@ -10,1 +10,1 @@\n-old\n+new\ndiff --git a/Cargo.lock b/Cargo.lock\n--- a/Cargo.lock\n+++ b/Cargo.lock\n@@ -1 +1 @@\n-v1\n+v2\n";

    #[test]
    fn the_digest_names_hunks_and_leaves_lockfiles_out() {
        let d = digest(PATCH);
        assert_eq!(d.hunks, 2);
        assert!(d.text.contains("[h1] @@ -1,2 +1,2 @@\n fn a() {}\n-let x = 1;\n+let x = 2;"));
        assert!(!d.text.contains("Cargo.lock"));
        assert!(d.aliases["h2"].starts_with("a.rs:"));
    }

    #[test]
    fn answers_are_checked_and_laid_over_the_diff() {
        let d = digest(PATCH);
        let aliases: serde_json::Map<String, Value> = d.aliases.iter().map(|(a, id)| (a.clone(), json!(id))).collect();
        let meta = json!({"aliases": aliases, "files": [{"path": "a.rs", "aliases": ["h1", "h2"]}]});
        let raw = json!({"title": "T", "summary": "S", "skipped": [{"path": "a.rs", "note": "a rename"}, {"path": "zzz", "note": "x"}], "parts": [{"title": "One", "blurb": "", "steps": [
            {"title": "Bump", "hunks": ["h1", "h9", "h1"], "weight": "key", "text": "x goes to 2.", "notes": [{"kind": "choice", "text": "2, not 3."}, {"kind": "bogus", "text": "no"}]},
            {"title": "Empty", "hunks": ["h7"], "weight": "normal", "text": "nothing", "notes": []}]}]});
        let entry = normalize(&raw, &meta).unwrap();
        assert_eq!(entry["parts"][0]["steps"].as_array().unwrap().len(), 1);
        assert_eq!(entry["parts"][0]["steps"][0]["hunks"], json!([d.aliases["h1"]]));
        assert_eq!(entry["parts"][0]["steps"][0]["notes"].as_array().unwrap().len(), 1);
        assert_eq!(entry["skipped"], json!([{"path": "a.rs", "note": "a rename"}]));
        let laid = overlay(&entry, PATCH);
        assert_eq!(laid["outdated"], 0);
        assert_eq!(laid["uncovered"][0]["path"], "a.rs");
        assert_eq!(laid["uncovered"][0]["note"], "a rename");
        assert_eq!(laid["uncovered"][1]["path"], "Cargo.lock");
        let edited = PATCH.replace("+let x = 2;", "+let x = 3;");
        assert_eq!(overlay(&entry, &edited)["outdated"], 1);
        assert!(normalize(&json!({"parts": []}), &meta).is_err());
    }

    #[test]
    fn a_finished_job_becomes_an_entry_and_its_state_follows() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let d = digest(PATCH);
        let key = key_of(&d, "haiku");
        std::fs::create_dir_all(r.join("pointers")).unwrap();
        files::write(&r.join("pointers").join(format!("{}.json", place("/repo", "uncommitted"))), &json!({"key": key, "model": "haiku", "at": 1})).unwrap();
        let job = r.join("jobs").join(&key);
        std::fs::create_dir_all(&job).unwrap();
        assert_eq!(state_in(r, "/repo", "uncommitted", PATCH)["status"], "running");
        let aliases: serde_json::Map<String, Value> = d.aliases.iter().map(|(a, id)| (a.clone(), json!(id))).collect();
        files::write(&job.join("meta.json"), &json!({"aliases": aliases, "files": []})).unwrap();
        let raw = json!({"title": "T", "summary": "S", "skipped": [], "parts": [{"title": "P", "blurb": "", "steps": [{"title": "A", "hunks": ["h1", "h2"], "weight": "key", "text": "t", "notes": []}]}]});
        std::fs::write(job.join("out.json"), json!({"subtype": "success", "structured_output": raw, "total_cost_usd": 0.002}).to_string()).unwrap();
        std::fs::write(job.join("code"), "0").unwrap();
        let done = state_in(r, "/repo", "uncommitted", PATCH);
        assert_eq!((done["status"].as_str(), done["fresh"].as_bool()), (Some("done"), Some(true)));
        assert!(r.join("entries").join(format!("{key}.json")).exists());
        assert!(!job.exists());
        assert_eq!(state_in(r, "/elsewhere", "uncommitted", PATCH)["status"], "none");
    }
}
