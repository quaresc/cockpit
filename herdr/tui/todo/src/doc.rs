//! The to-do file: Markdown whose `- [ ] text` and `- [x] text` lines are the items. Every other line, and every item
//! line no edit touched, is written back exactly as it was read.

use std::sync::LazyLock;

use regex::Regex;

static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^- \[( |x)\] (.*)$").unwrap());
static DONE_ON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\(done \d{4}-\d{2}-\d{2}\)$").unwrap());

/// What a new file starts with, as todo.py writes it.
pub const NEW_FILE: &str = "# To-do\n\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The item's line in the file.
    pub line: usize,
    pub done: bool,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Doc {
    /// The file split on newlines: a file ending with one has an empty last line.
    lines: Vec<String>,
}

/// The item a line holds, read as todo.py reads it: around its spaces.
fn item_of(raw: &str) -> Option<(bool, String)> {
    let found = ITEM.captures(raw.trim())?;
    Some((&found[1] == "x", found[2].to_string()))
}

fn line_of(done: bool, text: &str) -> String {
    format!("- [{}] {text}", if done { "x" } else { " " })
}

/// The text without the "(done YYYY-MM-DD)" a toggle adds.
pub fn undated(text: &str) -> &str {
    DONE_ON.find(text).map_or(text, |m| &text[..m.start()])
}

impl Doc {
    pub fn parse(text: &str) -> Self {
        Self { lines: text.split('\n').map(str::to_string).collect() }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The items in file order.
    pub fn items(&self) -> Vec<Item> {
        self.lines.iter().enumerate().filter_map(|(line, raw)| item_of(raw).map(|(done, text)| Item { line, done, text })).collect()
    }

    /// A new open item at the top of the list, where /todo puts them too.
    pub fn add(&mut self, text: &str) {
        let at = match self.items().first() {
            Some(first) => first.line,
            // After the heading, before the empty line that ends the file.
            None if self.lines.len() > 1 && self.lines.last().is_some_and(String::is_empty) => self.lines.len() - 1,
            None => self.lines.len(),
        };
        self.lines.insert(at, line_of(false, text));
    }

    /// The item on `line` rewritten; the others keep their place.
    pub fn set(&mut self, line: usize, done: bool, text: &str) {
        if let Some(raw) = self.lines.get_mut(line) {
            *raw = line_of(done, text);
        }
    }

    pub fn remove(&mut self, line: usize) {
        if line < self.lines.len() {
            self.lines.remove(line);
        }
    }

    /// Two lines swap places, both written as they are.
    pub fn swap(&mut self, a: usize, b: usize) {
        if a < self.lines.len() && b < self.lines.len() {
            self.lines.swap(a, b);
        }
    }

    /// Open items first, done ones after, each kept in its order, in the lines the items already take.
    pub fn order(&mut self) {
        let items = self.items();
        let slots: Vec<usize> = items.iter().map(|i| i.line).collect();
        let mut raws: Vec<String> = items.iter().filter(|i| !i.done).map(|i| self.lines[i.line].clone()).collect();
        raws.extend(items.iter().filter(|i| i.done).map(|i| self.lines[i.line].clone()));
        for (slot, raw) in slots.into_iter().zip(raws) {
            self.lines[slot] = raw;
        }
    }

    /// An item done or open again: the date it was done on added or taken off, the list ordered again.
    pub fn toggle(&mut self, line: usize, today: &str) {
        let Some(item) = self.items().into_iter().find(|i| i.line == line) else { return };
        let text = undated(&item.text);
        if item.done {
            self.set(line, false, text);
        } else {
            self.set(line, true, &format!("{text} (done {today})"));
        }
        self.order();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_items_around_other_lines() {
        let doc = Doc::parse("# To-do\n\n- [ ] one\n  - [x] two (done 2026-10-27)\nnotes\n");
        let items = doc.items();
        assert_eq!(items.len(), 2);
        assert_eq!((items[1].line, items[1].done, items[1].text.as_str()), (3, true, "two (done 2026-10-27)"));
        assert_eq!(doc.text(), "# To-do\n\n- [ ] one\n  - [x] two (done 2026-10-27)\nnotes\n");
    }

    #[test]
    fn adds_on_top_or_after_the_heading() {
        let mut doc = Doc::parse(NEW_FILE);
        doc.add("first");
        assert_eq!(doc.text(), "# To-do\n\n- [ ] first\n");
        doc.add("second");
        assert_eq!(doc.text(), "# To-do\n\n- [ ] second\n- [ ] first\n");
    }

    #[test]
    fn toggles_with_the_date_and_orders() {
        let mut doc = Doc::parse("# To-do\n\n- [ ] a\n- [ ] b\n- [x] c (done 2026-10-01)\n");
        doc.toggle(2, "2026-10-28");
        assert_eq!(doc.text(), "# To-do\n\n- [ ] b\n- [x] a (done 2026-10-28)\n- [x] c (done 2026-10-01)\n");
        doc.toggle(4, "2026-10-28");
        assert_eq!(doc.text(), "# To-do\n\n- [ ] b\n- [ ] c\n- [x] a (done 2026-10-28)\n");
    }
}
