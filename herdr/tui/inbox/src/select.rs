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

/// The question before a batch: "rerun 4 PRs?", or "rerun 3 rows?" across kinds.
pub fn question(tree: &Tree, rows: &[usize], label: &str) -> String {
    let kinds: HashSet<Kind> = rows.iter().map(|&n| tree.nodes[n].row.kind).collect();
    let what = if kinds.len() == 1 { describe(tree, rows) } else { Kind::Plain.count(rows.len()) };
    format!("{label} {what}?")
}
