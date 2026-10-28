//! What one rebuild reads: the caches, the muted repos and Claude's spend per ticket, read once and handed to the
//! model, so the model itself never touches a file and tests can feed it fixtures.

use std::collections::HashMap;
use std::path::PathBuf;

use cockpit_core::files;
use serde_json::{Value, json};

#[derive(Clone, Debug, Default)]
pub struct Inputs {
    /// inbox.json; None before the mod first wrote it.
    pub data: Option<Value>,
    pub ci: Value,
    pub brief: Option<Value>,
    pub muted: Vec<String>,
    /// What each ticket cost, in US dollars, summed over the sessions that worked on it.
    pub costs: HashMap<String, f64>,
}

impl Inputs {
    pub fn read() -> Self {
        let cache = |name: &str| files::load(&files::cache().join(name));
        Self {
            data: cache("inbox.json"),
            ci: cache("ci.json").unwrap_or_else(|| json!({"runs": []})),
            brief: cache("brief.json"),
            muted: muted(),
            costs: ticket_costs(),
        }
    }
}

fn muted() -> Vec<String> {
    files::load(&files::config_dir().join("muted.json"))
        .and_then(|v| v.as_array().map(|a| a.iter().filter_map(|r| r.as_str().map(str::to_string)).collect()))
        .unwrap_or_default()
}

fn ticket_costs() -> HashMap<String, f64> {
    let mut total = HashMap::new();
    for path in json_files("costs") {
        if let Some(Value::Object(spent)) = files::load(&path) {
            for (ticket, usd) in spent {
                *total.entry(ticket).or_insert(0.0) += usd.as_f64().unwrap_or(0.0);
            }
        }
    }
    total
}

pub fn json_files(dir: &str) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(files::cache().join(dir))
        .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect())
        .unwrap_or_default();
    paths.sort();
    paths
}
