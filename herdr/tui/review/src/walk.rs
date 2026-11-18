//! The walkthrough as the space draws it: none yet (what it does, the model, `r` writes it), being written, failed, or
//! its steps by part with the one in view explained above its hunks, outdated steps and the hunks no step covers
//! marked. It is the state `review::read` reports, the one the Herdr app's review panel draws.

use std::collections::HashSet;

use cockpit_core::text::wrap;
use cockpit_core::theme::{BG, DIM, FG, GREEN, ORANGE, RED, bold, dim, fg};
use cockpit_show::row::{Act, Row};
use ratatui::style::{Modifier, Style};
use serde_json::Value;

/// The models a walkthrough is written with, and what each is good at.
pub const MODELS: [(&str, &str, &str); 3] =
    [("haiku", "Haiku", "fast and cheap"), ("sonnet", "Sonnet", "better on large changes"), ("opus", "Opus", "the most careful")];
const SPINNER: [&str; 8] = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

/// One step to read: a part's step, or the hunks no step covers.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub title: String,
    pub text: String,
    pub weight: String,
    pub notes: Vec<(String, String)>,
    pub hunks: Vec<String>,
    pub part: Option<String>,
    pub outdated: bool,
    pub missing: u64,
    pub uncovered: bool,
}

/// none | running | failed | done.
pub fn status(w: &Value) -> &str {
    w["status"].as_str().unwrap_or("none")
}

/// The steps in reading order, then "Not covered" when some hunks are left out.
pub fn steps(w: &Value) -> Vec<Step> {
    if status(w) != "done" {
        return Vec::new();
    }
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let mut out = Vec::new();
    for part in w["parts"].as_array().into_iter().flatten() {
        for s in part["steps"].as_array().into_iter().flatten() {
            out.push(Step {
                title: text(&s["title"]),
                text: text(&s["text"]),
                weight: text(&s["weight"]),
                notes: s["notes"].as_array().into_iter().flatten().map(|n| (text(&n["kind"]), text(&n["text"]))).collect(),
                hunks: s["hunks"].as_array().into_iter().flatten().filter_map(|h| h.as_str().map(str::to_string)).collect(),
                part: Some(text(&part["title"])),
                outdated: s["outdated"] == true,
                missing: s["missing"].as_u64().unwrap_or(0),
                uncovered: false,
            });
        }
    }
    let uncovered: Vec<String> = w["uncovered"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|u| u["hunks"].as_array().into_iter().flatten())
        .filter_map(|h| h.as_str().map(str::to_string))
        .collect();
    if !uncovered.is_empty() {
        out.push(Step { title: "Not covered".into(), weight: "skim".into(), hunks: uncovered, uncovered: true, ..Step::empty() });
    }
    out
}

impl Step {
    fn empty() -> Self {
        Step {
            title: String::new(),
            text: String::new(),
            weight: String::new(),
            notes: Vec::new(),
            hunks: Vec::new(),
            part: None,
            outdated: false,
            missing: 0,
            uncovered: false,
        }
    }

    /// The hunks the diff shows for this step.
    pub fn only(&self) -> HashSet<String> {
        self.hunks.iter().cloned().collect()
    }
}

/// What a note's kind reads as.
fn note_label(kind: &str) -> &str {
    match kind {
        "choice" => "Decision",
        "upkeep" => "Upkeep",
        "gap" => "Limitation",
        "divergence" => "Divergence",
        "untested" => "Untested",
        other => other,
    }
}

fn wrapped(text: &str, cols: usize, lead: &str, style: Style) -> Vec<Row> {
    wrap(text, cols.saturating_sub(lead.chars().count()).max(10)).into_iter().map(|l| Row::text(lead, style).add(l, style)).collect()
}

fn button(label: &str, key: char) -> Row {
    Row::text(format!("▸ {label}  {key} "), Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD))
}

/// The rows above the diff in walkthrough mode; `tick` turns the spinner while one is written.
pub fn rows(w: &Value, step: usize, model: usize, cols: usize, tick: u64) -> Vec<Row> {
    let mut out = Vec::new();
    match status(w) {
        "running" => {
            let spin = SPINNER[(tick as usize) % SPINNER.len()];
            let by = w["model"].as_str().unwrap_or("the agent");
            out.push(Row::text(format!("{spin} "), fg(ORANGE)).add(format!("Writing the walkthrough with {by}…"), bold(FG)));
            out.extend(wrapped("It runs in the background: leave, it is waiting when you come back.", cols, "  ", dim()));
            out.push(Row::new());
            out.push(Row::text("  s stops it", dim()));
        }
        "none" | "failed" => {
            out.push(Row::text("Walk through these changes", bold(FG)));
            let about = "An agent groups the hunks into steps in the order they make sense, explains what each changes, and lists what no step covers. It explains, it does not judge. One background call, no tools, cached by the exact changes.";
            out.extend(wrapped(about, cols, "", dim()));
            out.push(Row::new());
            let mut pills = Row::text("Model  ", dim());
            for (i, (_, label, _)) in MODELS.iter().enumerate() {
                let on = i == model;
                pills = pills.add(if on { format!("▸ {label}  ") } else { format!("  {label}  ") }, if on { bold(ORANGE) } else { fg(FG) });
            }
            out.push(pills.add(format!(" {} · m picks another", MODELS[model].2), dim()));
            if status(w) == "failed" {
                out.push(Row::new());
                out.extend(wrapped(w["error"].as_str().unwrap_or("it failed"), cols, "✗ ", fg(RED)));
            }
            out.push(Row::new());
            out.push(button(if status(w) == "failed" { "try again" } else { "write it" }, 'r'));
        }
        _ => done_rows(w, step, cols, &mut out),
    }
    out
}

fn done_rows(w: &Value, step: usize, cols: usize, out: &mut Vec<Row>) {
    let steps = steps(w);
    out.push(Row::text(w["title"].as_str().unwrap_or("Walkthrough").to_string(), bold(FG)));
    out.extend(wrapped(w["summary"].as_str().unwrap_or_default(), cols, "", dim()));
    let mut part = None;
    for (i, s) in steps.iter().enumerate() {
        if s.part != part || s.uncovered {
            out.push(Row::new());
            if let Some(title) = s.part.as_ref().filter(|_| !s.uncovered) {
                out.push(Row::text(title.to_uppercase(), bold(DIM)));
            }
            part = s.part.clone();
        }
        let on = i == step;
        let number = if s.uncovered { "·".to_string() } else { (i + 1).to_string() };
        let mut row = Row::new().act(format!("{} {number:>2}  ", if on { "▸" } else { " " }), fg(ORANGE), Act::Pick(i));
        row = row.act(s.title.clone(), if on { bold(ORANGE) } else { fg(FG) }, Act::Pick(i));
        let badge = |row: Row, text: &str, color| row.add(format!("  {text}"), fg(color));
        row = match (s.weight.as_str(), s.outdated) {
            (_, true) => badge(row, "outdated", ORANGE),
            ("key", _) => badge(row, "key", GREEN),
            _ => row,
        };
        out.push(row);
    }
    let Some(s) = steps.get(step) else { return };
    out.push(Row::new());
    out.push(Row::text("─".repeat(cols), fg(cockpit_core::theme::LINE)));
    let kind = if s.weight == "skim" && !s.uncovered { "   context" } else { "" };
    out.push(Row::text(format!("‹ {} / {} ›  ", step + 1, steps.len()), fg(ORANGE)).add(s.title.clone(), bold(FG)).add(kind, dim()).add("   h l steps", dim()));
    if !s.text.is_empty() {
        out.extend(wrapped(&s.text, cols, "", fg(FG)));
    }
    if s.uncovered {
        for u in w["uncovered"].as_array().into_iter().flatten() {
            let note = u["note"].as_str().filter(|n| !n.is_empty()).map(|n| format!("  {n}")).unwrap_or_default();
            out.push(Row::text(format!("  {}", u["path"].as_str().unwrap_or_default()), fg(DIM)).add(note, fg(FG)));
        }
    }
    for (kind, text) in &s.notes {
        let lead = format!("◆ {}  ", note_label(kind));
        for (k, line) in wrap(text, cols.saturating_sub(lead.chars().count()).max(10)).into_iter().enumerate() {
            let head = if k == 0 { Row::text(lead.clone(), bold(ORANGE)) } else { Row::text(" ".repeat(lead.chars().count()), dim()) };
            out.push(head.add(line, fg(FG)));
        }
    }
    if s.outdated {
        out.push(Row::text(format!("{} of this step's hunks changed since it was written.", s.missing), fg(ORANGE)));
    }
    if w["fresh"] == false {
        out.push(Row::text("The changes moved on since this was written · r writes it again", fg(ORANGE)));
    }
    let cost = w["cost"].as_f64().map(|c| format!(" · ${c:.3}")).unwrap_or_default();
    out.push(Row::text(format!("{}{cost}", w["model"].as_str().unwrap_or_default()), dim()));
}

/// The walkthrough as Markdown, for the clipboard.
pub fn markdown(w: &Value) -> Option<String> {
    if status(w) != "done" {
        return None;
    }
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let mut lines = vec![format!("# {}", text(&w["title"])), String::new(), text(&w["summary"]), String::new()];
    for part in w["parts"].as_array().into_iter().flatten() {
        lines.push(format!("## {}", text(&part["title"])));
        if let Some(blurb) = part["blurb"].as_str().filter(|b| !b.is_empty()) {
            lines.extend([blurb.to_string(), String::new()]);
        }
        for s in part["steps"].as_array().into_iter().flatten() {
            let weight = match s["weight"].as_str() {
                Some("key") => " (key change)",
                Some("skim") => " (context)",
                _ => "",
            };
            lines.extend([format!("**{}**{weight}", text(&s["title"])), String::new(), text(&s["text"]), String::new()]);
            for n in s["notes"].as_array().into_iter().flatten() {
                lines.extend([format!("> {}: {}", note_label(n["kind"].as_str().unwrap_or_default()), text(&n["text"])), String::new()]);
            }
        }
    }
    let uncovered: Vec<String> = w["uncovered"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|u| format!("- `{}`{}", text(&u["path"]), u["note"].as_str().filter(|n| !n.is_empty()).map(|n| format!(": {n}")).unwrap_or_default()))
        .collect();
    if !uncovered.is_empty() {
        lines.push("## Not covered".into());
        lines.extend(uncovered);
    }
    Some(lines.join("\n"))
}
