//! What is left of getting started, from the doctor's kept report (`cockpit doctor --json` writes it to the cache): each
//! step done or not, with the command that does it, for the spaces' empty states.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::files;
use crate::text::{fit, width};
use crate::theme::{FG, GREEN, bold, dim, fg};

/// How often a space asks the doctor again while its report stays missing or old.
const AGAIN: Duration = Duration::from_secs(10 * 60);

/// One step of getting started.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub label: String,
    pub done: bool,
    /// The command that does it, else what to do in words.
    pub fix: Option<String>,
}

/// The kept report, null before the doctor first ran.
pub fn report() -> Value {
    files::load(&files::cache().join("doctor.json")).unwrap_or(Value::Null)
}

/// The report's steps, in order.
pub fn steps(report: &Value) -> Vec<Step> {
    report["getting_started"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| Step {
            label: s["label"].as_str().unwrap_or_default().to_string(),
            done: s["done"] == true,
            fix: s["fix"]["command"].as_str().or(s["fix"]["label"].as_str()).map(str::to_string),
        })
        .collect()
}

/// Runs `cockpit doctor --json --cached` in the background, at most every ten minutes, so the next read has a report.
pub fn refresh() {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if last.is_some_and(|at| at.elapsed() < AGAIN) {
        return;
    }
    *last = Some(Instant::now());
    let Ok(exe) = std::env::current_exe() else { return };
    use std::process::{Command, Stdio};
    if let Ok(mut child) = Command::new(exe).args(["doctor", "--json", "--cached"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        // Waited for off the keys' thread, so it never lingers as a zombie.
        std::thread::spawn(move || child.wait());
    }
}

/// The checklist in `cells` columns: a heading saying how many steps are done, then a line per step with its fix; none
/// when every step is done.
pub fn lines(steps: &[Step], cells: usize) -> Vec<Line<'static>> {
    let left = steps.iter().filter(|s| !s.done).count();
    if steps.is_empty() || left == 0 {
        return Vec::new();
    }
    let mut lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("Getting started", bold(FG)),
            Span::styled(format!("   {} of {} done · cockpit doctor --all says more", steps.len() - left, steps.len()), dim()),
        ]),
    ];
    let label_w = steps.iter().map(|s| width(&s.label)).max().unwrap_or(0).min(cells.saturating_sub(6));
    for step in steps {
        let (mark, style) = if step.done { ("✓", fg(GREEN)) } else { ("○", fg(FG)) };
        let label = fit(&step.label, label_w);
        let mut spans = vec![Span::styled(format!(" {mark} "), style), Span::styled(format!("{label:<label_w$}"), if step.done { dim() } else { fg(FG) })];
        if let Some(fix) = step.fix.as_ref().filter(|_| !step.done) {
            let room = cells.saturating_sub(3 + label_w + 3);
            if room > 8 {
                spans.push(Span::styled(format!("   {}", fit(fix, room)), dim()));
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim_end().to_string()
    }

    #[test]
    fn the_steps_left_read_with_their_fix() {
        let report = json!({"getting_started": [
            {"id": "setup", "label": "Set up cockpit", "done": true, "fix": null},
            {"id": "github", "label": "Sign in to GitHub", "done": false, "fix": {"label": "gh auth login", "command": "gh auth login"}},
            {"id": "tools", "label": "Give Claude Code cockpit's tools", "done": false, "fix": {"label": "/plugin install cockpit"}},
        ]});
        let found = steps(&report);
        assert_eq!(found[1], Step { label: "Sign in to GitHub".into(), done: false, fix: Some("gh auth login".into()) });
        assert_eq!(found[2].fix.as_deref(), Some("/plugin install cockpit"), "words when no command does it");
        let shown: Vec<String> = lines(&found, 80).iter().map(text).collect();
        assert_eq!(shown[1], "Getting started   1 of 3 done · cockpit doctor --all says more");
        assert_eq!(shown[2], " ✓ Set up cockpit");
        assert_eq!(shown[3], " ○ Sign in to GitHub                  gh auth login");
        let narrow: Vec<String> = lines(&found, 30).iter().skip(2).map(text).collect();
        assert!(narrow.iter().all(|l| width(l) <= 30 && !l.contains("gh auth login")), "a narrow pane drops the fixes first: {narrow:?}");
    }

    #[test]
    fn nothing_when_every_step_is_done_or_no_report() {
        assert!(lines(&steps(&json!({"getting_started": [{"label": "Set up cockpit", "done": true}]})), 80).is_empty());
        assert!(lines(&steps(&Value::Null), 80).is_empty());
    }
}
