//! What a waiting agent asks, read off the bottom of its screen: the question, its numbered choices, whether they are
//! checkboxes, where the cursor is and where Submit sits; and the keys that answer it. Ported from herdr-relevo's
//! `lib/prompt.js` (Apache-2.0), whose key sequences were measured on Claude Code's menus.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

static OPTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([❯›>]\s*)?(\d)\.\s+(.+?)\s*$").unwrap());
static SUBMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^([❯›>]\s*)?(Submit|Submit answers)$").unwrap());
static BOX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[([ xX✔✓✗])?\]\s*").unwrap());
static FRAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[│┃╭╮╰╯─━┌┐└┘|\s]+|[│┃╭╮╰╯─━┌┐└┘|\s]+$").unwrap());
static BESIDE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s{3,}[┌│└╭╰].*$").unwrap());
/// Lines between two choices that still belong to the menu: a description, a blank, a separator, Submit.
const MAX_GAP: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub enum Cursor {
    On(u32),
    Submit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub n: u32,
    pub label: String,
    pub detail: String,
    /// A checkbox's state in a multiple choice, None in a single one.
    pub checked: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Question {
    pub text: String,
    pub choices: Vec<Choice>,
    pub cursor: Option<Cursor>,
    /// Submit follows this choice (multiple choice forms).
    pub submit_after: Option<u32>,
    pub preview: String,
    /// The choice whose field takes free text, if any.
    pub free: Option<u32>,
}

impl Question {
    pub fn multi(&self) -> bool {
        self.choices.iter().any(|c| c.checked.is_some())
    }

    pub fn kind(&self) -> &'static str {
        match () {
            _ if self.choices.is_empty() => "text",
            _ if self.multi() => "multi",
            _ => "single",
        }
    }

    /// Claude's tool permission menu: "Do you want to …?" over Yes / … / No.
    pub fn permission(&self) -> bool {
        self.text.to_lowercase().starts_with("do you want to") && self.choices.first().is_some_and(|c| c.label.starts_with("Yes"))
    }

    pub fn json(&self) -> Value {
        let cursor = match &self.cursor {
            Some(Cursor::On(n)) => json!(n),
            Some(Cursor::Submit) => json!("submit"),
            None => Value::Null,
        };
        let choices: Vec<Value> = self
            .choices
            .iter()
            .map(|c| json!({"n": c.n, "label": c.label, "detail": c.detail, "checked": c.checked, "free": Some(c.n) == self.free}))
            .collect();
        json!({"text": self.text, "kind": self.kind(), "choices": choices, "cursor": cursor, "submit": self.submit_after.is_some(), "preview": self.preview, "permission": self.permission()})
    }
}

fn unframe(line: &str) -> String {
    FRAME.replace_all(line, "").into_owned()
}

/// The question on the screen: its choices when it shows a menu, else its last two lines as the question.
pub fn read(screen: &str) -> Question {
    let all: Vec<String> = screen.lines().map(unframe).collect();
    let lines: Vec<String> = all[all.len().saturating_sub(60)..].to_vec();
    let none = Question { text: last_lines(&lines, 2), ..Default::default() };
    let Some(last) = (0..lines.len()).rev().find(|&i| OPTION.is_match(&lines[i])) else { return none };
    let number = |i: usize| OPTION.captures(&lines[i]).and_then(|c| c[2].parse::<u32>().ok()).unwrap_or(0);
    // Walk up from the last choice to choice 1.
    let mut found = vec![(last, number(last))];
    let mut gap = 0;
    let mut i = last;
    while i > 0 && found[0].1 > 1 && gap <= MAX_GAP {
        i -= 1;
        if !OPTION.is_match(&lines[i]) {
            gap += 1;
            continue;
        }
        if number(i) != found[0].1 - 1 {
            break;
        }
        found.insert(0, (i, number(i)));
        gap = 0;
    }
    if found[0].1 != 1 {
        return none;
    }
    let mut q = Question::default();
    for (k, &(i, n)) in found.iter().enumerate() {
        let caps = OPTION.captures(&lines[i]).expect("matched above");
        let raw = BESIDE.replace(&caps[3], "").into_owned();
        if caps.get(1).is_some() {
            q.cursor = Some(Cursor::On(n));
        }
        let next = found.get(k + 1).map(|f| f.0).unwrap_or_else(|| lines.len().min(i + 1 + MAX_GAP));
        let mut detail = Vec::new();
        for l in lines[i + 1..next].iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
            if let Some(s) = SUBMIT.captures(l) {
                q.submit_after = Some(n);
                if s.get(1).is_some() {
                    q.cursor = Some(Cursor::Submit);
                }
            } else if !OPTION.is_match(l) {
                detail.push(l.to_string());
            }
        }
        let (label, checked) = match BOX.captures(&raw) {
            Some(b) => (raw[b[0].len()..].to_string(), Some(b.get(1).is_some_and(|m| m.as_str() != " "))),
            None => (raw.clone(), None),
        };
        let detail = if k + 1 < found.len() { detail.join(" ") } else { String::new() };
        q.choices.push(Choice { n, label, detail, checked });
    }
    q.text = question_above(&lines, found[0].0);
    q.preview = preview_beside(screen);
    q.free = free_option(&q);
    q
}

/// The question's own lines: not the tab strip of a multi-question form ("← ☒ CI gates ✔ Submit →").
fn last_lines(lines: &[String], n: usize) -> String {
    let kept: Vec<&str> = lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty() && !(l.starts_with('←') && l.ends_with('→'))).collect();
    kept[kept.len().saturating_sub(n)..].join("\n")
}

/// The paragraph right above the choices, two lines at most: it ends at a blank, the input prompt or a tab strip.
fn question_above(lines: &[String], first: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = first;
    while i > 0 && lines[i - 1].trim().is_empty() {
        i -= 1;
    }
    while i > 0 && out.len() < 2 {
        i -= 1;
        let l = lines[i].trim();
        let prompt = l.starts_with(['❯', '›', '>']) && l.chars().nth(1).is_some_and(char::is_whitespace);
        if l.is_empty() || prompt || (l.starts_with('←') && l.ends_with('→')) || l.starts_with('☐') || l.starts_with('☒') {
            break;
        }
        out.insert(0, l.to_string());
    }
    out.join("\n")
}

/// The box drawn beside the choices shows the one under the cursor: its inner text, or "".
fn preview_beside(screen: &str) -> String {
    static TOP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d\.\s.*\s{3,}┌─+┐\s*$").unwrap());
    static BOTTOM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"└─+┘\s*$").unwrap());
    static INNER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"│(.*)│\s*$").unwrap());
    let raw: Vec<&str> = screen.lines().collect();
    let raw = &raw[raw.len().saturating_sub(40)..];
    let Some(top) = raw.iter().position(|l| TOP.is_match(l)) else { return String::new() };
    let mut out: Vec<String> = Vec::new();
    for l in &raw[top + 1..] {
        if BOTTOM.is_match(l) {
            break;
        }
        if let Some(c) = INNER.captures(l) {
            out.push(c[1].trim().to_string());
        }
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out.join("\n")
}

/// The free-text choice: "Type something", else in a single choice the one before "Chat about this" (its label is
/// what was typed once something was).
fn free_option(q: &Question) -> Option<u32> {
    if let Some(c) = q.choices.iter().find(|c| c.label.to_lowercase().starts_with("type something")) {
        return Some(c.n);
    }
    let last = q.choices.last()?;
    (last.label.to_lowercase().starts_with("chat about this") && last.n > 1).then(|| last.n - 1)
}

/// The arrow presses from the cursor to choice `n`; Submit sits in the list right after `submit_after`.
pub fn move_keys(q: &Question, to: Option<u32>) -> Vec<String> {
    let order: Vec<Option<u32>> = q.choices.iter().flat_map(|c| if q.submit_after == Some(c.n) { vec![Some(c.n), None] } else { vec![Some(c.n)] }).collect();
    let at = match q.cursor {
        Some(Cursor::On(n)) => Some(n),
        Some(Cursor::Submit) => None,
        None => Some(1),
    };
    let (Some(from), Some(target)) = (order.iter().position(|o| *o == at), order.iter().position(|o| *o == to)) else { return Vec::new() };
    let key = if target > from { "down" } else { "up" };
    vec![key.to_string(); from.abs_diff(target)]
}

/// The agent's last words on screen: its last "⏺"/"•" line that is not a tool call or hook noise.
pub fn last_message(screen: &str) -> String {
    static SAID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[⏺•]\s+(.+?)\s*$").unwrap());
    static TOOL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Z][A-Za-z]*\(|hook was skipped|^User (answered|declined)").unwrap());
    screen.lines().rev().filter_map(|l| SAID.captures(l)).map(|c| c[1].to_string()).find(|t| !TOOL.is_match(t)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINGLE: &str = "⏺ Old invoices store two dates. Before I migrate 48,210 rows I need one decision.

 ☐ Invoice date

Which date should old invoices keep?

❯ 1. Issue date
     Matches the PDFs
  2. Payment date
     What reports use
  3. Type something.
  4. Chat about this

Enter to select · ↑/↓ to navigate · Esc to cancel";

    const MULTI: &str = "Which platforms should sync offline drafts?

  1. [✔] iOS
❯ 2. [✔] macOS
  3. [ ] Web
     needs IndexedDB
  4. [ ] Type something
     Submit
";

    #[test]
    fn a_single_choice_reads_with_its_cursor_details_and_free_field() {
        let q = read(SINGLE);
        assert_eq!(q.text, "Which date should old invoices keep?");
        assert_eq!(q.kind(), "single");
        assert_eq!(q.choices.len(), 4);
        assert_eq!((q.choices[0].label.as_str(), q.choices[0].detail.as_str()), ("Issue date", "Matches the PDFs"));
        assert_eq!(q.cursor, Some(Cursor::On(1)));
        assert_eq!(q.free, Some(3));
        assert_eq!(move_keys(&q, Some(2)), ["down"]);
        assert_eq!(last_message(SINGLE), "Old invoices store two dates. Before I migrate 48,210 rows I need one decision.");
    }

    #[test]
    fn a_multiple_choice_reads_its_boxes_and_where_submit_is() {
        let q = read(MULTI);
        assert_eq!(q.kind(), "multi");
        assert_eq!(q.choices.iter().map(|c| c.checked).collect::<Vec<_>>(), [Some(true), Some(true), Some(false), Some(false)]);
        assert_eq!(q.choices[2].detail, "needs IndexedDB");
        assert_eq!(q.submit_after, Some(4));
        assert_eq!(q.cursor, Some(Cursor::On(2)));
        assert_eq!(q.free, Some(4));
        assert_eq!(move_keys(&q, None), ["down", "down", "down"]);
        assert_eq!(move_keys(&q, Some(1)), ["up"]);
    }

    #[test]
    fn a_screen_without_a_menu_is_a_free_text_question() {
        let q = read("⏺ Done with the migration.\n\nShall I open the PR now, or wait for the backfill?\n");
        assert_eq!((q.kind(), q.choices.len()), ("text", 0));
        assert!(q.text.ends_with("or wait for the backfill?"));
    }

    #[test]
    fn a_permission_prompt_is_told_apart() {
        let q = read("Bash command\n  rm -rf build\n\nDo you want to proceed?\n❯ 1. Yes\n  2. Yes, and don't ask again for rm commands\n  3. No, and tell Claude what to do differently (esc)\n");
        assert!(q.permission());
        assert_eq!(q.json()["choices"][2]["label"], "No, and tell Claude what to do differently (esc)");
    }
}
