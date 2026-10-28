//! Type to filter: each word typed matched fuzzily against one of a step's number, text, place, status or command, the
//! matched characters kept per field for the rows to highlight; across the shown runbook or, the tabs counting, all.

use std::collections::HashMap;

use cockpit_core::input::Input;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::book::{Book, Step};

#[derive(Default)]
pub struct Filter {
    pub input: Input,
    /// Typing in the bar, rather than kept after enter.
    pub editing: bool,
    /// Counting matches in every runbook, each tab showing its own.
    pub global: bool,
}

impl Filter {
    pub fn is_on(&self) -> bool {
        self.editing || !self.input.text.is_empty()
    }

    pub fn query(&self) -> &str {
        self.input.text.trim()
    }
}

/// The characters a step matched on, by field, as char indices: number, step, where, status, command.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hit {
    pub fields: [Vec<usize>; 5],
}

/// Each runbook's matching steps by number, for a query; None when nothing is typed, so every step shows.
pub fn find(books: &[Book], query: &str) -> Option<HashMap<String, HashMap<String, Hit>>> {
    if query.is_empty() {
        return None;
    }
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    Some(books.iter().map(|b| (b.slug.clone(), b.steps.iter().filter_map(|s| hit(&pattern, &mut matcher, s).map(|h| (s.n.clone(), h))).collect())).collect())
}

/// Each word typed must match within one field, so a match never strings letters together across columns.
fn hit(pattern: &Pattern, matcher: &mut Matcher, step: &Step) -> Option<Hit> {
    let fields = [&step.n, &step.step, &step.where_, &step.status, &step.command];
    let mut found = Hit::default();
    let mut buf = Vec::new();
    for atom in &pattern.atoms {
        let mut best: Option<(u16, usize, Vec<u32>)> = None;
        for (k, field) in fields.iter().enumerate() {
            let mut indices = Vec::new();
            if let Some(score) = atom.indices(Utf32Str::new(field, &mut buf), matcher, &mut indices)
                && best.as_ref().is_none_or(|(top, _, _)| score > *top)
            {
                best = Some((score, k, indices));
            }
        }
        let (_, k, indices) = best?;
        found.fields[k].extend(indices.into_iter().map(|i| i as usize));
    }
    for field in &mut found.fields {
        field.sort_unstable();
        field.dedup();
    }
    Some(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(n: &str, text: &str, command: &str) -> Step {
        Step { n: n.into(), step: text.into(), where_: "prod".into(), status: "⬜ to do".into(), command: command.into() }
    }

    fn hits_book() -> Book {
        Book { path: "x.md".into(), slug: "x".into(), title: String::new(), updated: String::new(), steps: Vec::new(), notes: vec![] }
    }

    #[test]
    fn matches_split_by_field() {
        let book = Book { steps: vec![step("1", "Backfill dry run", "mix backfill --dry-run"), step("2", "Deploy", "")], ..hits_book() };
        let found = find(&[book], "dry").unwrap();
        let hits = &found["x"];
        assert_eq!(hits.len(), 1);
        assert_eq!(hits["1"].fields[1], [9, 10, 11]);
        // "deploy" and "prod" together hold d, r and y, but no single field of step 2 does.
        assert!(find(&[Book { steps: vec![step("2", "Deploy", "")], ..hits_book() }], "dry").unwrap()["x"].is_empty());
    }
}
