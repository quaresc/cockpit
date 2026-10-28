//! The blocks drawn as text: sequence charts, mermaid sequence diagrams, and a branch's CI run and PR from the caches
//! the mod keeps.

use ratatui::style::{Modifier, Style};
use serde_json::Value;

use cockpit_core::files;
use cockpit_core::theme::{GREEN, ORANGE, RED, dim, fg};

use crate::markdown::Segment;
use crate::{mermaid, sequence};

/// Lines of styled pieces, a piece with the address a click on it opens.
pub type Drawn = Vec<Vec<Segment>>;

fn piece(text: impl Into<String>, style: Style) -> Segment {
    (text.into(), style, None)
}

fn text(value: &Value) -> String {
    value.as_str().map_or_else(|| value.to_string(), str::to_string)
}

/// The block drawn `cols` cells wide; a line can be wider (a crowded diagram) and is wrapped where it is shown.
pub fn render(block: &Value, cols: usize) -> Drawn {
    let caches = if block["type"] == "ci" {
        let load = |name: &str| files::load(&files::cache().join(name)).unwrap_or_default();
        (load("ci.json"), load("inbox.json"))
    } else {
        Default::default()
    };
    draw(block, cols, &caches)
}

/// The block drawn with the CI and inbox caches given.
fn draw(block: &Value, cols: usize, caches: &(Value, Value)) -> Drawn {
    let cols = cols.max(20);
    let chart = |block: &Value| -> Drawn {
        let empty = Vec::new();
        let rows = sequence::draw(block["actors"].as_array().unwrap_or(&empty), block["messages"].as_array().unwrap_or(&empty), cols as i64 - 4);
        rows.into_iter().map(|row| std::iter::once(piece("  ", Style::new())).chain(row.into_iter().map(|(t, s)| piece(t, s))).collect()).collect()
    };
    match block["type"].as_str() {
        Some("sequence") => chart(block),
        Some("mermaid") => match mermaid::sequence_of(block["text"].as_str().unwrap_or_default()) {
            Ok(parsed) => chart(&parsed),
            Err(err) => vec![vec![piece(format!("  {err}"), fg(RED))]],
        },
        Some("ci") => ci(block, cols, caches),
        _ => Vec::new(),
    }
}

/// The nearest whole number, halves to the even neighbour.
fn round(value: f64) -> usize {
    value.round_ties_even().max(0.0) as usize
}

fn ci(block: &Value, cols: usize, (runs, inbox): &(Value, Value)) -> Drawn {
    let (repo, branch) = (&block["repo"], &block["branch"]);
    let same = |v: &&Value| v["repo"] == *repo && v["branch"] == *branch;
    let run = runs["runs"].as_array().and_then(|r| r.iter().find(same)).cloned();
    let pr = inbox["mine"].as_array().and_then(|p| p.iter().find(same)).cloned();
    let mut head = vec![piece(format!("  {} · {}", text(repo), text(branch)), Style::new().add_modifier(Modifier::BOLD))];
    if let Some(pr) = &pr {
        head.push((format!("   #{} {}", text(&pr["number"]), text(&pr["title"])), dim(), pr["url"].as_str().map(str::to_string)));
    }
    let mut out = vec![head];
    let Some(run) = run else {
        out.push(vec![piece("  no CI run known for this branch yet", dim())]);
        return out;
    };
    let total = run["total"].as_u64().unwrap_or(1).max(1);
    let done = run["done"].as_u64().unwrap_or(0);
    let room = (cols as i64 - 30).clamp(10, 40) as usize;
    let filled = round(done as f64 / total as f64 * room as f64).min(room);
    let outcome = run["outcome"].as_str().unwrap_or_default();
    let (status, style) = if run["isRunning"].as_bool().unwrap_or(false) {
        ("running".to_string(), fg(ORANGE))
    } else if matches!(outcome, "success" | "succeeded") {
        ("passed".to_string(), fg(GREEN))
    } else {
        (if outcome.is_empty() { "?".to_string() } else { outcome.to_string() }, fg(RED))
    };
    out.push(vec![
        piece("  ", Style::new()),
        piece("█".repeat(filled), style),
        piece("░".repeat(room - filled), dim()),
        piece(format!("  {done}/{total} jobs · {status}"), style),
    ]);
    if let Some(failed) = run["failed"].as_object().filter(|f| !f.is_empty()) {
        let step = failed.get("step").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| format!(" · {s}")).unwrap_or_default();
        out.push(vec![piece(format!("  ✗ {}{step}", failed.get("job").map(text).unwrap_or_default()), fg(RED))]);
        for test in failed.get("tests").and_then(Value::as_array).into_iter().flatten() {
            out.push(vec![piece(format!("    {}", text(test)), fg(RED).add_modifier(Modifier::DIM))]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every text-drawn block of the fixtures against its expected text under tests/fixtures/text, at 80 and 120 columns.
    #[test]
    fn draws_the_expected_text() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let caches = (files::load(&dir.join("ci-cache.json")).unwrap(), files::load(&dir.join("inbox-cache.json")).unwrap());
        for fixture in ["sequence", "mermaid", "ci", "drawn"] {
            let spec = files::load(&dir.join(format!("{fixture}.json"))).unwrap();
            for cols in [80, 120] {
                let mut out = Vec::new();
                for (n, block) in spec["blocks"].as_array().unwrap().iter().enumerate() {
                    let flowchart = block["type"] == "mermaid" && crate::mermaid::is_flowchart(block["text"].as_str().unwrap());
                    if !["sequence", "mermaid", "ci"].contains(&block["type"].as_str().unwrap()) || flowchart {
                        continue;
                    }
                    out.push(format!("#{n}"));
                    out.extend(draw(block, cols, &caches).iter().map(|line| line.iter().map(|p| p.0.as_str()).collect::<String>().trim_end().to_string()));
                }
                let expected = std::fs::read_to_string(dir.join(format!("text/{fixture}_{cols}.txt"))).unwrap();
                assert_eq!(out.join("\n") + "\n", expected, "{fixture} at {cols} columns");
            }
        }
    }

    #[test]
    fn halves_round_to_even() {
        assert_eq!((round(0.5), round(1.5), round(2.5), round(17.14)), (0, 2, 2, 17));
    }
}
