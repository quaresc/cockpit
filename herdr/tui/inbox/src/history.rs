//! Each ticket's cost history for its sparkline, asked of the hub in the background at most every few minutes; when the
//! hub cannot be reached, its store (~/.cache/cockpit/history/ticket_cost.jsonl, one {key, value} per line) is read.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use cockpit_core::{files, hub};
use serde_json::Value;

const KIND: &str = "ticket_cost";
/// Costs move slowly: the series are asked again after this, or as soon as the tickets change.
const EVERY: Duration = Duration::from_secs(300);

type Series = HashMap<String, Vec<f64>>;

pub struct History {
    enabled: bool,
    series: Series,
    asked: Option<(Instant, Vec<String>)>,
    busy: bool,
    tx: Sender<Series>,
    rx: Receiver<Series>,
}

impl History {
    /// `enabled` false for tests: the series given stay, nothing is asked.
    pub fn new(enabled: bool, series: Series) -> Self {
        let (tx, rx) = channel();
        Self { enabled, series, asked: None, busy: false, tx, rx }
    }

    pub fn series(&self) -> &Series {
        &self.series
    }

    /// Asks for these tickets' series, unless asked for the same tickets lately or still asking.
    pub fn want(&mut self, mut ids: Vec<String>) {
        ids.sort();
        let fresh = self.asked.as_ref().is_some_and(|(at, asked)| *asked == ids && at.elapsed() < EVERY);
        if !self.enabled || self.busy || fresh || ids.is_empty() {
            return;
        }
        self.asked = Some((Instant::now(), ids.clone()));
        self.busy = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let kept = std::fs::read_to_string(files::cache().join("history").join(format!("{KIND}.jsonl"))).unwrap_or_default();
            let series = ids
                .into_iter()
                .map(|id| {
                    let mut values = hub::history(KIND, &id);
                    if values.is_empty() {
                        values = from_store(&kept, &id);
                    }
                    (id, values)
                })
                .filter(|(_, v)| !v.is_empty())
                .collect();
            let _ = tx.send(series);
        });
    }

    /// Takes the series that arrived; true when they differ from those shown, for the rows to be built again.
    pub fn pump(&mut self) -> bool {
        let Ok(series) = self.rx.try_recv() else { return false };
        self.busy = false;
        let changed = series != self.series;
        self.series = series;
        changed
    }
}

/// The key's values in the hub's store, oldest first.
fn from_store(store: &str, key: &str) -> Vec<f64> {
    store.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|p| p["key"].as_str() == Some(key)).filter_map(|p| p["value"].as_f64()).collect()
}
