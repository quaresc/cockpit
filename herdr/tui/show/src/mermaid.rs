//! mermaid.py's flowchart reader: `graph`/`flowchart` text becomes a flow block. Sequence diagrams stay with show.py's
//! renderer, through show_render.py.

use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::{Value, json};

static NODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([A-Za-z0-9_.\-]+)\s*(\(\[.*?\]\)|\[\[.*?\]\]|\[\(.*?\)\]|\(\(.*?\)\)|\{\{.*?\}\}|\[.*?\]|\(.*?\)|\{.*?\}|>.*?\])?").unwrap()
});
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:(?:--|==|-\.)\s*([^-=>|.][^>|]*?)\s*(?:-->|==>|\.->)|(?:-->|---|==>|===|-\.->|-\.-|--x|--o)(?:\s*\|([^|]*)\|)?)").unwrap()
});
static SIDEWAYS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(LR|RL)\b").unwrap());
const SKIPPED: [&str; 9] = ["%%", "classDef", "class ", "style ", "linkStyle", "click ", "subgraph", "end", "direction"];

pub fn is_flowchart(text: &str) -> bool {
    let start = text.trim_start();
    start.starts_with("flowchart") || start.starts_with("graph")
}

/// A node's id, label and kind from `A`, `A[label]`, `A(label)`, `A{label}` and the rest.
fn node(found: &Captures) -> (String, String, &'static str) {
    let ident = found[1].to_string();
    let shape = found.get(2).map_or("", |m| m.as_str());
    if shape.is_empty() {
        return (ident.clone(), ident, "box");
    }
    let kind = if shape.starts_with('{') { "decision" } else if shape.starts_with('(') { "round" } else { "box" };
    let label = shape.trim_matches(['[', ']', '(', ')', '{', '}', '>']).trim().trim_matches('"').to_string();
    (ident.clone(), if label.is_empty() { ident } else { label }, kind)
}

/// The flow block a mermaid flowchart describes.
pub fn flowchart(text: &str) -> Value {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty() && !l.trim().starts_with("%%")).collect();
    let header = lines.first().map_or("", |l| l.trim());
    let direction = if SIDEWAYS.is_match(header) { "right" } else { "down" };
    let (mut nodes, mut edges): (Vec<Value>, Vec<Value>) = (Vec::new(), Vec::new());
    for raw in lines.iter().skip(1) {
        for statement in raw.split(';') {
            let text = statement.trim();
            if text.is_empty() || SKIPPED.iter().any(|s| text.starts_with(s)) {
                continue;
            }
            let (mut at, mut previous): (usize, Option<(String, String)>) = (0, None);
            while at < text.len() {
                let Some(found) = NODE.captures(&text[at..]) else { break };
                let (ident, label, kind) = node(&found);
                let shaped = found.get(2).is_some();
                match nodes.iter_mut().find(|n| n["id"] == ident.as_str()) {
                    Some(n) if shaped => *n = json!({"id": ident, "label": label, "kind": kind}),
                    Some(_) => {}
                    None => nodes.push(json!({"id": ident, "label": label, "kind": kind})),
                }
                if let Some((from, label)) = previous.take() {
                    let mut edge = json!({"from": from, "to": ident});
                    if !label.is_empty() {
                        edge["label"] = json!(label);
                    }
                    edges.push(edge);
                }
                at += found.get(0).unwrap().end();
                let Some(link) = LINK.captures(&text[at..]) else { break };
                let label = link.get(1).or(link.get(2)).map_or("", |m| m.as_str()).trim().to_string();
                previous = Some((ident, label));
                at += link.get(0).unwrap().end();
            }
        }
    }
    json!({"type": "flow", "direction": direction, "nodes": nodes, "edges": edges})
}
