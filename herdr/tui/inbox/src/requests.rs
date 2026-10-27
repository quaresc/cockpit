//! Requests in flight (rerun, redeploy, preview, release…) that the mod carries out: inbox.py files them, this app
//! watches for their results and shows the working text, then the outcome, on the row that asked.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use cockpit_core::files;
use serde_json::Value;

use crate::facts::{flag, s};

/// A request waits this long for a Claude session to take it, and this long in all for its outcome.
const CLAIM_WAIT: f64 = 120.0;
const ANSWER_WAIT: f64 = 600.0;
const OUTCOME_SHOWN: f64 = 10.0;

pub struct Job {
    file: String,
    pub working: String,
    label: String,
    at: f64,
}

pub struct Outcome {
    pub ok: bool,
    pub message: String,
    at: f64,
}

#[derive(Default)]
pub struct Requests {
    pub pending: HashMap<String, Job>,
    pub outcome: HashMap<String, Outcome>,
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

impl Requests {
    /// The requests an action filed, as bridge.py returns them: row → {file, working, label, at}.
    pub fn merge(&mut self, pending: &Value) {
        for (row, job) in pending.as_object().into_iter().flatten() {
            let job = Job { file: s(job, "file").into(), working: s(job, "working").into(), label: s(job, "label").into(), at: job["at"].as_f64().unwrap_or_else(now) };
            self.pending.insert(row.clone(), job);
        }
    }

    /// Moves finished requests from pending to their outcome, read from the file the mod writes; the notes to show.
    pub fn settle(&mut self) -> Vec<String> {
        let dir = files::cache().join("requests");
        let mut notes = Vec::new();
        let rows: Vec<String> = self.pending.keys().cloned().collect();
        for row in rows {
            let job = &self.pending[&row];
            let path = dir.join("results").join(&job.file);
            let Some(outcome) = files::load(&path) else {
                let waited = now() - job.at;
                let is_claimed = dir.join("results").join(format!("{}.claimed", job.file)).exists();
                if waited > ANSWER_WAIT || (waited > CLAIM_WAIT && !is_claimed) {
                    let message = if is_claimed { "the session that took it never answered" } else { "no Claude session picked it up" };
                    // Left behind, a request would run whenever a session next starts, maybe hours later.
                    let _ = std::fs::remove_file(dir.join(&job.file));
                    self.outcome.insert(row.clone(), Outcome { ok: false, message: message.into(), at: now() });
                    self.pending.remove(&row);
                }
                continue;
            };
            let _ = std::fs::remove_file(&path);
            let job = self.pending.remove(&row).expect("listed above");
            let message = if s(&outcome, "state") == "refused" { format!("refused: {}", s(&outcome, "message")) } else { s(&outcome, "message").to_string() };
            notes.push(format!("{}: {message}", job.label));
            self.outcome.insert(row, Outcome { ok: flag(&outcome, "ok"), message, at: now() });
        }
        self.outcome.retain(|_, outcome| now() - outcome.at <= OUTCOME_SHOWN);
        notes
    }
}
