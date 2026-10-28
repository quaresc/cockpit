//! The file parser and writer: what it reads it writes back byte for byte, and an edit leaves every other line as it was.

use cockpit_todo::doc::{Doc, undated};
use proptest::prelude::*;

fn line() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9 ()#:/.-]{0,30}".prop_map(|t| format!("- [ ] {t}")),
        "[a-zA-Z0-9 ()#:/.-]{0,30}".prop_map(|t| format!("- [x] {t}")),
        "[a-z ]{0,10}".prop_map(|t| format!("  - [x] {t}  ")),
        Just("- [X] upper is not an item".to_string()),
        Just("# To-do".to_string()),
        Just(String::new()),
        "[^\n]{0,40}",
    ]
}

fn doc() -> impl Strategy<Value = (String, bool)> {
    (prop::collection::vec(line(), 0..14), any::<bool>()).prop_map(|(lines, newline)| {
        let mut text = lines.join("\n");
        if newline {
            text.push('\n');
        }
        (text, newline)
    })
}

/// The lines that are not items, in order.
fn others(doc: &Doc) -> Vec<String> {
    let items: Vec<usize> = doc.items().iter().map(|i| i.line).collect();
    doc.lines().iter().enumerate().filter(|(n, _)| !items.contains(n)).map(|(_, l)| l.clone()).collect()
}

/// The item lines, sorted, for comparing as a set.
fn item_lines(doc: &Doc) -> Vec<String> {
    let mut lines: Vec<String> = doc.items().iter().map(|i| doc.lines()[i.line].clone()).collect();
    lines.sort();
    lines
}

proptest! {
    #[test]
    fn writes_back_what_it_read((text, _) in doc()) {
        prop_assert_eq!(Doc::parse(&text).text(), text);
    }

    #[test]
    fn an_edit_keeps_every_other_line((text, _) in doc(), pick in any::<prop::sample::Index>(), op in 0..5u8, new in "[a-z]{1,8}") {
        let before = Doc::parse(&text);
        let items = before.items();
        let mut after = before.clone();
        let touched = if items.is_empty() { None } else { Some(items[pick.index(items.len())].clone()) };
        match (op, &touched) {
            (0, _) => after.add(&new),
            (1, Some(item)) => after.toggle(item.line, "2026-10-28"),
            (2, Some(item)) => after.set(item.line, item.done, &new),
            (3, Some(item)) => after.remove(item.line),
            (4, _) => after.order(),
            _ => {}
        }
        prop_assert_eq!(others(&before), others(&after));
        let mut kept = item_lines(&before);
        let mut now = item_lines(&after);
        if let (Some(item), true) = (&touched, matches!(op, 1..=3)) {
            let raw = before.lines()[item.line].clone();
            kept.remove(kept.iter().position(|l| *l == raw).unwrap());
            let written = match op {
                1 if item.done => Some(format!("- [ ] {}", undated(&item.text))),
                1 => Some(format!("- [x] {} (done 2026-10-28)", undated(&item.text))),
                2 => Some(format!("- [{}] {new}", if item.done { "x" } else { " " })),
                _ => None,
            };
            if let Some(raw) = written {
                let at = now.iter().position(|l| *l == raw);
                prop_assert!(at.is_some(), "{} not written", raw);
                now.remove(at.unwrap());
            }
        }
        if op == 0 {
            now.remove(now.iter().position(|l| *l == format!("- [ ] {new}")).unwrap());
        }
        prop_assert_eq!(kept, now);
    }

    #[test]
    fn order_puts_open_items_first((text, _) in doc()) {
        let mut doc = Doc::parse(&text);
        doc.order();
        let done: Vec<bool> = doc.items().iter().map(|i| i.done).collect();
        prop_assert!(done.windows(2).all(|w| !(w[0] && !w[1])));
    }
}
