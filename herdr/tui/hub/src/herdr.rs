//! The herdr CLI: a run whose output the caller reads, its `result`, and the spaces with their "[N] " prefix stripped.

use std::sync::LazyLock;

use serde_json::Value;

use crate::proc::{self, Out};

pub fn bin() -> String {
    std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into())
}

fn argv(args: &[&str]) -> Vec<String> {
    std::iter::once(bin()).chain(args.iter().map(|a| a.to_string())).collect()
}

/// A herdr call run to its end, 15 seconds at most.
pub fn run(args: &[&str]) -> Out {
    proc::run15(&argv(args))
}

/// The `result` of a call that succeeded or not, the way a JSON parse of its output finds it.
pub fn result_of(out: &Out) -> Option<Value> {
    serde_json::from_str::<Value>(&out.stdout).ok()?.get("result").cloned()
}

/// herdr's spaces, each label stripped of its "[N] " prefix.
pub fn spaces() -> Vec<Value> {
    let listed = result_of(&run(&["workspace", "list"])).and_then(|r| r.get("workspaces").and_then(Value::as_array).cloned()).unwrap_or_default();
    static PREFIX: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^\[\d+\]\s*").expect("a valid pattern"));
    listed
        .into_iter()
        .map(|mut w| {
            let label = w["label"].as_str().unwrap_or_default();
            let stripped = PREFIX.replace(label, "").into_owned();
            if let Some(obj) = w.as_object_mut() {
                obj.insert("label".into(), stripped.into());
            }
            w
        })
        .collect()
}
