//! Rows picked for a batch action, by key so a rebuild keeps them: the actions they all share, and how many of what
//! they are, for the panel and the question asked before running one on each.

use std::collections::HashSet;

use cockpit_core::acts::Act;

use crate::row::Kind;
use crate::tree::Tree;

#[derive(Default)]
pub struct Picked {
    pub keys: HashSet<String>,
}

impl Picked {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn clear(&mut self) {
        self.keys.clear();
    }

    /// Picks the row, or drops it when picked; headings and detail lines are never picked.
    pub fn toggle(&mut self, tree: &Tree, node: usize) -> bool {
        let row = &tree.nodes[node].row;
        if row.kind == Kind::Plain {
            return false;
        }
        if !self.keys.remove(&row.key) {
            self.keys.insert(row.key.clone());
        }
        true
    }

    /// Picks every visible row of the cursor row's kind; drops them all when they all were picked already.
    pub fn same_kind(&mut self, tree: &Tree, node: usize) {
        let kind = tree.nodes[node].row.kind;
        if kind == Kind::Plain {
            return;
        }
        let keys: Vec<String> = tree.shown().iter().map(|s| &tree.nodes[s.node].row).filter(|r| r.kind == kind).map(|r| r.key.clone()).collect();
        if keys.iter().all(|k| self.keys.contains(k)) {
            for key in keys {
                self.keys.remove(&key);
            }
        } else {
            self.keys.extend(keys);
        }
    }

    /// Forgets the picked rows a rebuild took away.
    pub fn keep_known(&mut self, tree: &Tree) {
        let known: HashSet<&str> = tree.nodes.iter().map(|n| n.row.key.as_str()).collect();
        self.keys.retain(|k| known.contains(k.as_str()));
    }

    /// One row per picked key, in tree order.
    pub fn rows(&self, tree: &Tree) -> Vec<usize> {
        let mut seen = HashSet::new();
        (0..tree.nodes.len()).filter(|&n| self.keys.contains(&tree.nodes[n].row.key) && seen.insert(&tree.nodes[n].row.key)).collect()
    }
}

/// The actions every one of these rows has, by label and key, in the first row's order; the main one stays main.
pub fn shared(tree: &Tree, rows: &[usize]) -> Vec<Act> {
    let Some(&first) = rows.first() else { return Vec::new() };
    tree.nodes[first]
        .row
        .acts
        .iter()
        // A start opens the form on one row; it is never run for many at once.
        .filter(|act| act.call.as_ref().is_none_or(|c| c.function != "start"))
        .filter(|act| rows.iter().all(|&n| tree.nodes[n].row.acts.iter().any(|a| a.act.label == act.act.label && a.act.key == act.act.key)))
        .map(|a| a.act.clone())
        .collect()
}

/// "4 PRs" when they are all of one kind, "2 PRs, 1 ticket" otherwise.
pub fn describe(tree: &Tree, rows: &[usize]) -> String {
    let mut kinds: Vec<(Kind, usize)> = Vec::new();
    for &n in rows {
        let kind = tree.nodes[n].row.kind;
        match kinds.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, count)) => *count += 1,
            None => kinds.push((kind, 1)),
        }
    }
    kinds.iter().map(|(kind, n)| kind.count(*n)).collect::<Vec<_>>().join(", ")
}

/// The label of the irreversible removal, the only action a kept worktree stays out of.
pub const REMOVE: &str = "remove worktree";

/// "a, b and 7 more": a list kept short, its count still said, its items joined by `sep`.
pub fn short(names: &[String], first: usize, sep: &str) -> String {
    match names.len() {
        n if n <= first + 1 => names.join(sep),
        n => format!("{} and {} more", names[..first].join(sep), n - first),
    }
}

/// What keeps a worktree row, in words: "1 uncommitted change, locked".
fn why(tree: &Tree, node: usize) -> String {
    blockers(tree, node).into_iter().filter_map(|b| b["words"].as_str().map(str::to_string)).collect::<Vec<_>>().join(", ")
}

/// The picked rows an action runs on: every one, but a removal leaves the worktrees something keeps.
pub fn takes(tree: &Tree, rows: &[usize], label: &str) -> Vec<usize> {
    rows.iter().copied().filter(|&r| !(label == REMOVE && kept(tree, r))).collect()
}

/// The worktrees a removal leaves, with what keeps each: "a (1 uncommitted change); b (…)".
pub fn stays(tree: &Tree, rows: &[usize]) -> String {
    let said: Vec<String> = rows.iter().filter(|&&n| kept(tree, n)).map(|&n| format!("{} ({})", tree.nodes[n].row.name, why(tree, n))).collect();
    short(&said, 3, "; ")
}

/// The question before a batch: "rerun 4 PRs?", or "rerun 3 rows?" across kinds. A removal of worktrees says which go
/// and which stay, with what keeps them.
pub fn question(tree: &Tree, rows: &[usize], label: &str) -> String {
    let kinds: HashSet<Kind> = rows.iter().map(|&n| tree.nodes[n].row.kind).collect();
    if label == REMOVE && kinds.len() == 1 && kinds.contains(&Kind::Worktree) {
        let go: Vec<String> = rows.iter().filter(|&&n| !kept(tree, n)).map(|&n| tree.nodes[n].row.name.clone()).collect();
        let goes = if go.len() == 1 {
            "Its folder is deleted; its branch too once git counts it merged."
        } else {
            "Their folders are deleted; their branches too once git counts them merged."
        };
        let mut question = format!("{label}: {}? {goes}", Kind::Worktree.count(go.len()));
        question.push_str(&format!(" Goes: {}.", short(&go, 5, ", ")));
        let stay = stays(tree, rows);
        if !stay.is_empty() {
            question.push_str(&format!(" Stays: {stay}."));
        }
        return question;
    }
    let what = if kinds.len() == 1 { describe(tree, rows) } else { Kind::Plain.count(rows.len()) };
    format!("{label} {what}?")
}

/// The question before a merged session's worktree goes with its space: the session ends first.
pub fn ending(name: &str) -> String {
    format!("{REMOVE}: {name}? Ends the session in it, then deletes its folder; its branch too once git counts it merged.")
}

/// The question before one worktree's removal: what goes, or what keeps it and that the hub refuses then.
pub fn removal(tree: &Tree, node: usize) -> String {
    let name = &tree.nodes[node].row.name;
    if kept(tree, node) {
        format!("{REMOVE}: {name}? Kept by {}; the hub refuses while anything keeps it.", why(tree, node))
    } else {
        format!("{REMOVE}: {name}? Its folder is deleted; its branch too once git counts it merged.")
    }
}

/// What keeps a worktree row, as its item says.
fn blockers(tree: &Tree, node: usize) -> Vec<serde_json::Value> {
    tree.nodes[node].row.item.as_ref().and_then(|i| i["blockers"].as_array().cloned()).unwrap_or_default()
}

/// A worktree something keeps: a batch leaves it where it is.
pub fn kept(tree: &Tree, node: usize) -> bool {
    tree.nodes[node].row.kind == Kind::Worktree && !blockers(tree, node).is_empty()
}
