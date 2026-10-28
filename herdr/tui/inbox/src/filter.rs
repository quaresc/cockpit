//! Type to filter: a fuzzy match of what is typed against every row's text, the matched characters kept to light up.

use std::collections::HashMap;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// The rows that match the query, by index, each with the characters that matched, sorted; empty for a blank query.
pub fn matches<'a>(query: &str, haystacks: impl Iterator<Item = (usize, &'a str)>) -> HashMap<usize, Vec<usize>> {
    if query.trim().is_empty() {
        return HashMap::new();
    }
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut out = HashMap::new();
    for (n, text) in haystacks {
        let mut hits = Vec::new();
        if pattern.indices(Utf32Str::new(text, &mut buf), &mut matcher, &mut hits).is_some() {
            let mut hits: Vec<usize> = hits.into_iter().map(|i| i as usize).collect();
            hits.sort_unstable();
            hits.dedup();
            out.insert(n, hits);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_fuzzily_with_the_characters_that_matched() {
        let rows = [(0, "orders#12 fix the login"), (1, "billing#3 add taxes"), (2, "ENG-4  in progress  Login page")];
        let found = matches("lgin", rows.iter().map(|(n, t)| (*n, *t)));
        assert_eq!(found.keys().copied().collect::<std::collections::BTreeSet<_>>(), [0, 2].into());
        assert_eq!(found[&0], vec![18, 20, 21, 22]);
        assert!(matches("  ", rows.iter().map(|(n, t)| (*n, *t))).is_empty());
    }
}
