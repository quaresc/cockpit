//! The report_step tool: how a runbook step the runbook app handed to the agent went, recorded in its handoff and in
//! the step's status cell, as the Claude plugin records it.

use std::path::Path;

use cockpit_core::files;
use serde_json::{Value, json};

use super::tools::OUTCOMES;

const OPEN: [&str; 3] = ["sent", "running", "waiting"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The runbook status cell a final outcome writes, the way the runbook app's done button writes it.
fn status_of(outcome: &str) -> String {
    use chrono::Datelike;
    let now = chrono::Local::now();
    match outcome {
        "done" => format!("✅ done {:02} {}", now.day(), MONTHS[now.month0() as usize]),
        "failed" => "❌ failed".into(),
        _ => "❌ blocked".into(),
    }
}

/// Sets one step's status cell, the fourth column of its `| # | Step | Where | Status | Command |` row.
fn set_status(path: &Path, step: &str, status: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let Some(index) = lines.iter().position(|l| l.trim().starts_with('|') && l.split('|').count() > 5 && l.split('|').nth(1).map(str::trim) == Some(step))
    else {
        return false;
    };
    let mut cells: Vec<String> = lines[index].split('|').map(str::to_string).collect();
    cells[4] = format!(" {status} ");
    lines[index] = cells.join("|");
    std::fs::write(path, lines.join("\n")).is_ok()
}

pub fn report(dir: &Path, input: &Value) -> Result<String, String> {
    let id = input["handoff"].as_str().unwrap_or_default();
    let outcome = input["outcome"].as_str().unwrap_or_default();
    let message = input["message"].as_str().unwrap_or_default().trim();
    let valid = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c));
    let path = dir.join(format!("{id}.json"));
    let Some(handoff) = valid.then(|| files::load(&path)).flatten() else {
        let mut open: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| files::load(&e.path()))
            .filter(|h| h["state"].as_str().is_some_and(|s| OPEN.contains(&s)))
            .filter_map(|h| h["id"].as_str().map(str::to_string))
            .collect();
        open.sort();
        return Err(format!("no handoff {}; open handoffs: {}", json!(id), if open.is_empty() { "none".into() } else { open.join(", ") }));
    };
    let state = handoff["state"].as_str().unwrap_or_default();
    if !OPEN.contains(&state) {
        let said = handoff["message"].as_str().filter(|m| !m.is_empty()).map(|m| format!(" ({m})")).unwrap_or_default();
        return Err(format!("handoff {id} was already reported {state}{said}; there is nothing left to report"));
    }
    if !OUTCOMES.contains(&outcome) {
        return Err(format!("outcome must be one of {}, got {}", OUTCOMES.join(", "), input["outcome"]));
    }
    if message.is_empty() {
        return Err("message must say in one line what happened, or what the step waits for".into());
    }
    let step = handoff["step"].as_str().unwrap_or_default();
    let name = format!("step {step} of {}", handoff["slug"].as_str().unwrap_or_default().to_uppercase());
    let mut next = handoff.clone();
    next["message"] = message.into();
    if outcome == "waiting" {
        next["state"] = "waiting".into();
        files::write(&path, &next).map_err(|e| e.to_string())?;
        return Ok(format!("Recorded: {name} waits for the person's go. Once it has run, report done, failed or blocked."));
    }
    let status = status_of(outcome);
    let written = set_status(Path::new(handoff["path"].as_str().unwrap_or_default()), step, &status);
    next["state"] = outcome.into();
    next["reportedAt"] = crate::find::now_ms().into();
    files::write(&path, &next).map_err(|e| e.to_string())?;
    Ok(if written {
        format!("Recorded: {name} {outcome}; its runbook status now reads \"{status}\".")
    } else {
        format!("Recorded: {name} {outcome}, but no row {step} was found in {}: set its status there yourself.", handoff["path"].as_str().unwrap_or_default())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_reported_done_sets_its_status_cell_once() {
        let dir = tempfile::tempdir().unwrap();
        let book = dir.path().join("eng-412.md");
        std::fs::write(&book, "| # | Step | Where | Status | Command |\n|---|---|---|---|---|\n| 1 | Migrate | api | | `mix ecto.migrate` |\n").unwrap();
        let handoff = json!({"id": "eng-412-1", "slug": "eng-412", "step": "1", "title": "Migrate", "path": book, "state": "sent", "sentAt": 1});
        files::write(&dir.path().join("eng-412-1.json"), &handoff).unwrap();
        let said = report(dir.path(), &json!({"handoff": "eng-412-1", "outcome": "failed", "message": "the migration timed out"})).unwrap();
        assert_eq!(said, "Recorded: step 1 of ENG-412 failed; its runbook status now reads \"❌ failed\".");
        assert!(std::fs::read_to_string(&book).unwrap().contains("| 1 | Migrate | api | ❌ failed | `mix ecto.migrate` |"));
        let again = report(dir.path(), &json!({"handoff": "eng-412-1", "outcome": "done", "message": "ok"})).unwrap_err();
        assert_eq!(again, "handoff eng-412-1 was already reported failed (the migration timed out); there is nothing left to report");
        assert_eq!(
            report(dir.path(), &json!({"handoff": "eng-9", "outcome": "done", "message": "ok"})).unwrap_err(),
            "no handoff \"eng-9\"; open handoffs: none"
        );
    }
}
