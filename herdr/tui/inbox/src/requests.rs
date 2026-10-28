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
    row: String,
    pub working: String,
    label: String,
    at: f64,
}

pub struct Outcome {
    pub ok: bool,
    pub message: String,
    at: f64,
}

/// Requests by their file, so a second one from the same row (rerun, then redeploy) never hides the first from its
/// timeout; outcomes by row.
#[derive(Default)]
pub struct Requests {
    pending: HashMap<String, Job>,
    pub outcome: HashMap<String, Outcome>,
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

impl Requests {
    /// The requests an action filed, as bridge.py returns them: row → {file, working, label, at}.
    pub fn merge(&mut self, pending: &Value) {
        for (row, job) in pending.as_object().into_iter().flatten() {
            let file = s(job, "file").to_string();
            let job = Job { row: row.clone(), working: s(job, "working").into(), label: s(job, "label").into(), at: job["at"].as_f64().unwrap_or_else(now) };
            self.pending.insert(file, job);
        }
    }

    /// How many requests are in flight or show their outcome, to tell when that changed.
    pub fn count(&self) -> usize {
        self.pending.len() + self.outcome.len()
    }

    /// The row's latest request still in flight.
    pub fn working(&self, row: &str) -> Option<&Job> {
        self.pending.values().filter(|job| job.row == row).max_by(|a, b| a.at.total_cmp(&b.at))
    }

    /// Moves finished requests from pending to their outcome, read from the file the mod writes; the notes to show.
    pub fn settle(&mut self) -> Vec<String> {
        let dir = files::cache().join("requests");
        let mut notes = Vec::new();
        let names: Vec<String> = self.pending.keys().cloned().collect();
        for file in names {
            let job = &self.pending[&file];
            let path = dir.join("results").join(&file);
            let Some(outcome) = files::load(&path) else {
                let waited = now() - job.at;
                let is_claimed = dir.join("results").join(format!("{file}.claimed")).exists();
                if waited > ANSWER_WAIT || (waited > CLAIM_WAIT && !is_claimed) {
                    let message = if is_claimed { "the session that took it never answered" } else { "no Claude session picked it up" };
                    // Left behind, a request would run whenever a session next starts, maybe hours later.
                    let _ = std::fs::remove_file(dir.join(&file));
                    let job = self.pending.remove(&file).expect("listed above");
                    self.outcome.insert(job.row, Outcome { ok: false, message: message.into(), at: now() });
                }
                continue;
            };
            let _ = std::fs::remove_file(&path);
            let job = self.pending.remove(&file).expect("listed above");
            let message = if s(&outcome, "state") == "refused" { format!("refused: {}", s(&outcome, "message")) } else { s(&outcome, "message").to_string() };
            notes.push(format!("{}: {message}", job.label));
            self.outcome.insert(job.row, Outcome { ok: flag(&outcome, "ok"), message, at: now() });
        }
        self.outcome.retain(|_, outcome| now() - outcome.at <= OUTCOME_SHOWN);
        notes
    }
}
