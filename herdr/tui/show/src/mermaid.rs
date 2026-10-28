//! Mermaid text read into blocks: `graph`/`flowchart` text becomes a flow block, `sequenceDiagram` text a sequence block. Styling,
//! classes, subgraphs and sequence boxes (loop, alt, rect) are read past, since a terminal draws none of them.

use std::sync::LazyLock;

use regex::{Captures, Regex};
use serde_json::{Value, json};

static NODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([A-Za-z0-9_.\-]+)\s*(\(\[.*?\]\)|\[\[.*?\]\]|\[\(.*?\)\]|\(\(.*?\)\)|\{\{.*?\}\}|\[.*?\]|\(.*?\)|\{.*?\}|>.*?\])?").unwrap()
});
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:(?:--|==|-\.)\s*([^-=>|.][^>|]*?)\s*(?:-->|==>|\.->)|(?:-->|---|==>|===|-\.->|-\.-|--x|--o)(?:\s*\|([^|]*)\|)?)").unwrap()
});
static ARROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([^\s:\-+>]+)\s*(-->>|->>|-->|->|--x|-x|--\)|-\))\s*[+-]?\s*([^\s:]+)\s*:\s*(.*)$").unwrap()
});
static DECLARED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(participant|actor)\s+(\S+)(?:\s+as\s+(.+))?$").unwrap());
static NOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[Nn]ote\s+(?:left of|right of|over)\s+([^,:]+)(?:,[^:]+)?\s*:\s*(.*)$").unwrap());
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

/// The sequence block a mermaid sequence diagram's lines (after its header) describe.
fn sequence(lines: &[&str]) -> Value {
    let (mut actors, mut messages): (Vec<Value>, Vec<Value>) = (Vec::new(), Vec::new());
    let mut actor = |ident: &str, label: Option<&str>| match actors.iter_mut().find(|a| a["id"] == ident) {
        Some(a) => {
            if let Some(label) = label.filter(|l| !l.is_empty()) {
                a["label"] = json!(label);
            }
        }
        None => actors.push(json!({"id": ident, "label": label.filter(|l| !l.is_empty()).unwrap_or(ident)})),
    };
    for raw in lines {
        let text = raw.trim();
        if let Some(d) = DECLARED.captures(text) {
            actor(&d[2], d.get(3).map(|m| m.as_str()));
        } else if let Some(n) = NOTE.captures(text) {
            let over = n[1].trim();
            actor(over, None);
            messages.push(json!({"note": &n[2], "over": over}));
        } else if let Some(a) = ARROW.captures(text) {
            actor(&a[1], None);
            actor(&a[3], None);
            messages.push(json!({"from": &a[1], "to": &a[3], "label": a[4].trim(), "reply": a[2].starts_with("--")}));
        }
    }
    json!({"type": "sequence", "actors": actors, "messages": messages})
}

/// The sequence block a mermaid text describes, or why it cannot be drawn as text.
pub fn sequence_of(text: &str) -> Result<Value, String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty() && !l.trim().starts_with("%%")).collect();
    let header = lines.first().ok_or("empty mermaid")?.trim();
    if header.starts_with("sequenceDiagram") {
        return Ok(sequence(&lines[1..]));
    }
    Err(format!("only flowchart and sequenceDiagram are drawn, not {}", header.split_whitespace().next().unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn reads_shapes_and_labelled_links() {
        let flow = flowchart("graph LR\n  A[Start] -->|go| B{Ready?}\n  B -- yes --> C((Done))");
        assert_eq!(flow["direction"], "right");
        assert_eq!(flow["nodes"].as_array().map(Vec::len), Some(3));
        assert_eq!(flow["edges"][0], json!({"from": "A", "to": "B", "label": "go"}));
        assert_eq!(flow["edges"][1], json!({"from": "B", "to": "C", "label": "yes"}));
    }

    proptest! {
        /// Whatever the text, reading it ends, and every edge joins two nodes it read.
        #[test]
        fn any_flowchart_reads(lines in prop::collection::vec("[A-Za-z0-9_ \\[\\](){}>|;:\"%-=.ox]{0,30}", 0..12)) {
            let flow = flowchart(&format!("flowchart TD\n{}", lines.join("\n")));
            let ids: Vec<&Value> = flow["nodes"].as_array().into_iter().flatten().map(|n| &n["id"]).collect();
            for edge in flow["edges"].as_array().into_iter().flatten() {
                prop_assert!(ids.contains(&&edge["from"]) && ids.contains(&&edge["to"]));
            }
        }
    }
}
