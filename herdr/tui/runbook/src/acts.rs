//! A step's and a runbook's actions, as runbook.py's rows offer them, each with its key.

use std::collections::HashMap;
use std::sync::LazyLock;

use cockpit_core::acts::Act;
use regex::Regex;
use serde_json::Value;

use crate::book::{Book, Step};
use crate::handoff;

pub static PR_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([\w.-]+)#(\d+)$").unwrap());

pub fn handoff_id(book: &Book, step: &Step) -> String {
    format!("{}-{}", book.slug, step.n)
}

pub fn pr_ref(step: &Step) -> Option<(String, u64)> {
    let found = PR_REF.captures(&step.where_)?;
    Some((found[1].to_string(), found[2].parse().ok()?))
}

pub fn step_acts(book: &Book, step: &Step, handoffs: &HashMap<String, Value>) -> Vec<Act> {
    let mut acts = Vec::new();
    if step.is_open() {
        if handoffs.get(&handoff_id(book, step)).is_some_and(handoff::in_flight) {
            acts.push(Act::new("session", 's', "session").main());
        } else {
            acts.push(Act::new("run", 'r', "run").main().confirm(&format!("run-{}-{}", book.slug, step.n)));
        }
        acts.push(Act::new("done", 'd', "done"));
    }
    if pr_ref(step).is_some() {
        let act = Act::new(&format!("open {}", step.where_), 'o', "open");
        acts.push(if acts.is_empty() { act.main() } else { act });
    }
    if !step.command.is_empty() {
        let act = Act::new("copy command", 'c', "copy");
        acts.push(if acts.is_empty() { act.main() } else { act });
    }
    acts.push(Act::new("edit", 'e', "edit"));
    acts
}

pub fn book_acts(book: &Book) -> Vec<Act> {
    vec![
        Act::new("refresh", 'R', "refresh").main(),
        Act::new("talk", 't', "talk"),
        Act::new("delete", 'D', "delete").confirm(&format!("delete-{}", book.path.display())),
    ]
}
