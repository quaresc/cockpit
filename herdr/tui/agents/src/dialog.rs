//! What an agent's terminal shows besides its words and its permission asks: a dialog a command opened (`/model`,
//! `/resume`, `/config`), a yes-or-no confirmation, or the menu completing a slash command as it is typed, read off the
//! screen under the agent's shapes so a chat can draw it and press its keys.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use crate::prompt::{self, Cursor, Question, Shapes};

/// What is on the screen: a list to pick from, a yes-or-no, a slash command's completion menu, or a dialog read as
/// its lines when no list in it parses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Select,
    Confirm,
    Menu,
    Text,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub n: u32,
    pub label: String,
    pub hint: String,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub kind: Kind,
    pub title: String,
    pub body: Vec<String>,
    pub options: Vec<Item>,
    pub numbered: bool,
    pub footer: String,
    /// The keys its footer names and what each does, for a card's buttons: `("s", "use this session only")`.
    pub keys: Vec<(String, String)>,
    /// A Select or Confirm as `agents answer` takes it.
    pub question: Option<Question>,
}

/// How many lines up from the bottom a dialog is looked for.
const DIALOG_LINES: usize = 40;
/// How many lines of completion rows a menu holds at most.
const MENU_LINES: usize = 16;

static YES_NO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(yes|no)\b").unwrap());
/// One key hint of a dialog's footer: the key, then what it does.
static FOOTER_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<key>Enter(?:/[↓↑])?|Esc|Tab|Space|←/→|↑/↓|[←→↑↓]|Ctrl\+[A-Za-z]|[A-Za-z])\s+(?:to|for)\s+(?P<does>.+)$").unwrap());

/// The dialog on the screen, if one is: never a permission ask, which the question card owns.
pub fn read(screen: &str, shapes: &Shapes) -> Option<Dialog> {
    let raw: Vec<&str> = screen.lines().collect();
    let lines: Vec<String> = raw.iter().map(|l| prompt::unframe(l)).collect();
    if let Some(at) = prompt::input_line(&lines)
        && let Some(menu) = completion(&lines, &raw, at, shapes)
    {
        return Some(menu);
    }
    let q = prompt::read_for(screen, shapes);
    // A permission ask is the question card's; a bare yes or no after a command is a confirmation.
    if q.permission && !yes_or_no(&q) {
        return None;
    }
    let hint = hint_at(&lines, shapes);
    if !q.choices.is_empty() && (q.cursor.is_some() || hint.is_some()) {
        return Some(picked(q, &lines, hint, shapes));
    }
    plain(&raw, &lines, hint, shapes)
}

/// Two choices, yes and no.
fn yes_or_no(q: &Question) -> bool {
    q.choices.len() == 2 && q.choices.iter().all(|c| YES_NO.is_match(&c.label))
}

/// A list the cursor moves through, or a yes-or-no.
fn picked(q: Question, lines: &[String], hint: Option<usize>, shapes: &Shapes) -> Dialog {
    let cursor = match q.cursor {
        Some(Cursor::On(n)) => Some(n),
        _ => None,
    };
    // A row's words after a wide gap describe it: "Opus 5.5    For complex work".
    let split = |label: &str| -> (String, String) {
        match label.find("  ") {
            Some(at) => (label[..at].trim().to_string(), label[at..].trim().to_string()),
            None => (label.trim().to_string(), String::new()),
        }
    };
    let options = q
        .choices
        .iter()
        .map(|c| {
            let (label, beside) = split(&c.label);
            let hint = if c.detail.is_empty() { beside } else { c.detail.clone() };
            Item { n: c.n, label, hint, selected: cursor == Some(c.n) }
        })
        .collect();
    let confirm = yes_or_no(&q);
    // Its own lines: from the rule it opened under, words on that rule or not.
    let under = q.block.iter().rposition(|l| top_rule(l)).map_or(0, |i| i + 1);
    let block: Vec<String> = q.block[under..].iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    let title = block.iter().find(|l| Shapes::any(&shapes.dialog_title, l)).cloned().or_else(|| block.first().cloned()).unwrap_or_default();
    let body: Vec<String> = block.into_iter().filter(|l| *l != title).collect();
    let footer = hint.map(|i| footer_at(lines, i)).unwrap_or_default();
    Dialog {
        kind: if confirm { Kind::Confirm } else { Kind::Select },
        title,
        body,
        options,
        numbered: q.numbered,
        footer: footer.clone(),
        keys: keys_in(&footer),
        question: Some(q),
    }
}

/// A dialog with no list that parses, read as its lines: a slider, a search box, a settings table.
fn plain(raw: &[&str], lines: &[String], hint: Option<usize>, shapes: &Shapes) -> Option<Dialog> {
    let from = lines.len().saturating_sub(DIALOG_LINES);
    let top = (from..lines.len()).rev().find(|&i| top_rule(raw[i]));
    // A dialog stands under a frame, or ends with its hint; an input line under either means the agent is back at rest.
    let start = match (top, hint) {
        (Some(t), _) => t + 1,
        (None, Some(h)) => (from..h).rev().find(|&i| lines[i].trim().is_empty()).map_or(from, |i| i + 1),
        (None, None) => return None,
    };
    if prompt::input_line(lines).is_some_and(|i| i > start) {
        return None;
    }
    let end = hint.unwrap_or(lines.len());
    let mut body: Vec<String> = Vec::new();
    for l in lines[start..end].iter().map(|l| l.trim_end()) {
        if l.trim().is_empty() && body.last().is_none_or(|b| b.is_empty()) {
            continue;
        }
        body.push(l.trim_start_matches(' ').to_string());
    }
    while body.last().is_some_and(|l| l.is_empty()) {
        body.pop();
    }
    let title = body.iter().find(|l| Shapes::any(&shapes.dialog_title, l)).cloned().or_else(|| body.first().cloned()).unwrap_or_default();
    if title.is_empty() {
        return None;
    }
    let body: Vec<String> = body.into_iter().filter(|l| *l != title).collect();
    let footer = hint.map(|i| footer_at(lines, i)).unwrap_or_default();
    Some(Dialog { kind: Kind::Text, title, body, options: Vec::new(), numbered: false, footer: footer.clone(), keys: keys_in(&footer), question: None })
}

/// The slash command completion menu right above the input line: its rows, and which one the cursor is on.
fn completion(lines: &[String], raw: &[&str], at: usize, shapes: &Shapes) -> Option<Dialog> {
    let typed = lines[at].trim().trim_start_matches(['>', '›', '❯']).trim();
    if !typed.starts_with('/') {
        return None;
    }
    fn row<'a>(shapes: &Shapes, l: &'a str) -> Option<regex::Captures<'a>> {
        shapes.completion.iter().find_map(|r| r.captures(l))
    }
    let mut items: Vec<Item> = Vec::new();
    // A description wrapped onto the lines under its row, met before the row walking up.
    let mut wrapped: Vec<String> = Vec::new();
    let mut i = at;
    while i > 0 && at - i < MENU_LINES + 4 {
        i -= 1;
        let l = lines[i].trim_end();
        if l.trim().is_empty() || rule(l) {
            if items.is_empty() && wrapped.is_empty() {
                continue;
            }
            break;
        }
        if let Some(caps) = row(shapes, l) {
            let selected = raw[i].trim_start().starts_with(['❯', '›']);
            let mut hint = caps.get(2).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
            for more in wrapped.drain(..).rev() {
                hint = format!("{hint} {more}").trim().to_string();
            }
            items.insert(0, Item { n: 0, label: caps[1].to_string(), hint, selected });
        } else if raw[i].starts_with("  ") && !raw[i].trim_start().starts_with(['❯', '›']) {
            wrapped.push(l.trim().to_string());
        } else {
            break;
        }
    }
    if items.is_empty() {
        return None;
    }
    for (k, item) in items.iter_mut().enumerate() {
        item.n = k as u32 + 1;
    }
    Some(Dialog {
        kind: Kind::Menu,
        title: String::new(),
        body: Vec::new(),
        options: items,
        numbered: false,
        footer: String::new(),
        keys: Vec::new(),
        question: None,
    })
}

/// A rule across the screen, as the agent frames its input box.
fn rule(line: &str) -> bool {
    line.trim().chars().take_while(|c| matches!(c, '─' | '━' | '═' | '▔' | '▁')).count() >= 12
}

/// The rule a dialog opens under, words on it or not ("▔▔▔ Ctrl+Y to paste deleted text ▔").
fn top_rule(line: &str) -> bool {
    line.trim_start().chars().take_while(|c| *c == '▔' || *c == '▁').count() >= 12
}

/// The line of the dialog's key hint, among the last ones.
fn hint_at(lines: &[String], shapes: &Shapes) -> Option<usize> {
    (0..lines.len()).rev().take(24).find(|&i| Shapes::any(&shapes.dialog_hint, lines[i].trim()) || prompt::HINT.is_match(&lines[i]))
}

/// The hint from its line on, joined with the lines it wrapped onto.
fn footer_at(lines: &[String], at: usize) -> String {
    let mut out = lines[at].trim().to_string();
    for l in lines[at + 1..].iter().map(|l| l.trim()) {
        if l.is_empty() || FOOTER_KEY.is_match(l) {
            break;
        }
        out = format!("{out} {l}");
    }
    out
}

/// The keys a footer names, as hc names them, with what each does: "Enter to confirm · s for this session only".
fn keys_in(footer: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in footer.split([' ', '\u{a0}']).collect::<Vec<_>>().join(" ").split(" · ").flat_map(|p| p.split(" • ")) {
        let Some(caps) = FOOTER_KEY.captures(part.trim()) else { continue };
        let does = caps["does"].trim().to_string();
        for key in named(&caps["key"]) {
            out.push((key, does.clone()));
        }
    }
    out
}

/// A key as the footer writes it, as hc presses it; a pair of arrows is two keys.
fn named(key: &str) -> Vec<String> {
    match key {
        "Esc" => vec!["escape".into()],
        "Tab" => vec!["tab".into()],
        "Space" => vec!["space".into()],
        "←/→" => vec!["left".into(), "right".into()],
        "↑/↓" => vec!["up".into(), "down".into()],
        "←" => vec!["left".into()],
        "→" => vec!["right".into()],
        "↑" => vec!["up".into()],
        "↓" => vec!["down".into()],
        k if k.starts_with("Enter") => vec!["enter".into()],
        k if k.starts_with("Ctrl+") => vec![k.to_ascii_lowercase()],
        k => vec![k.to_string()],
    }
}

impl Dialog {
    /// The dialog as the chat view sends it: `{kind, title, body, options: [{n, label, hint, selected}], numbered,
    /// footer, keys: [{key, does}]}`.
    pub fn json(&self) -> Value {
        let kind = match self.kind {
            Kind::Select => "select",
            Kind::Confirm => "confirm",
            Kind::Menu => "menu",
            Kind::Text => "text",
        };
        json!({
            "kind": kind, "title": self.title, "body": self.body,
            "options": self.options.iter().map(|o| json!({"n": o.n, "label": o.label, "hint": o.hint, "selected": o.selected})).collect::<Vec<_>>(),
            "numbered": self.numbered, "footer": self.footer,
            "keys": self.keys.iter().map(|(key, does)| json!({"key": key, "does": does})).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/screens/{name}.txt", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn claude(name: &str) -> Option<Dialog> {
        read(&screen(name), &Shapes::of("claude"))
    }

    #[test]
    fn the_model_picker_reads_as_a_select_with_its_title_cursor_and_keys() {
        let d = claude("claude-model").expect("a dialog");
        assert_eq!((d.kind, d.title.as_str(), d.numbered), (Kind::Select, "Select model", true));
        assert_eq!(d.options.len(), 9, "the rows numbered on screen");
        assert_eq!(d.options.iter().find(|o| o.selected).map(|o| o.n), Some(6));
        assert_eq!(d.options[1].label, "Opus 5.5");
        assert!(d.body[0].starts_with("Switch between Claude models"));
        assert_eq!(
            d.keys,
            [("enter".to_string(), "set as default".to_string()), ("s".into(), "use this session only".into()), ("escape".into(), "cancel".into())]
        );
        assert!(d.question.is_some(), "answered as a question, with its fingerprint");
        let down = claude("claude-model-down").unwrap();
        assert_eq!(down.options.iter().find(|o| o.selected).map(|o| o.n), Some(7));
    }

    #[test]
    fn a_slider_a_search_box_and_a_settings_table_read_as_their_lines() {
        let effort = claude("claude-effort").unwrap();
        assert_eq!((effort.kind, effort.title.as_str()), (Kind::Text, "Effort"));
        assert!(effort.body.iter().any(|l| l.contains("low") && l.contains("max")));
        assert_eq!(effort.keys.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["left", "right", "enter", "s", "escape"]);
        let resume = claude("claude-resume").unwrap();
        assert_eq!((resume.kind, resume.title.as_str()), (Kind::Text, "Resume session"));
        assert!(resume.footer.ends_with("Esc to cancel"), "a hint wrapped onto two lines is one: {}", resume.footer);
        assert!(resume.keys.iter().any(|(k, _)| k == "ctrl+a"));
        let config = claude("claude-config").unwrap();
        assert_eq!(config.kind, Kind::Text);
        assert!(config.title.starts_with("Settings"), "its tab strip: {}", config.title);
        assert!(config.body.iter().any(|l| l.starts_with("Default permission mode")));
    }

    #[test]
    fn the_completion_menu_reads_as_a_menu_with_the_row_the_cursor_is_on() {
        let d = claude("claude-slash-menu").unwrap();
        assert_eq!(d.kind, Kind::Menu);
        let labels: Vec<&str> = d.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, ["/model", "/mobile", "/morning"]);
        assert!(d.options[0].selected);
        assert!(d.options[0].hint.starts_with("Set the AI model"));
        assert!(d.options[2].hint.contains("artifact, or set it up"), "a wrapped description joins its row: {}", d.options[2].hint);
    }

    #[test]
    fn an_empty_prompt_a_working_agent_and_a_permission_ask_are_no_dialog() {
        for name in ["claude-prompt-empty", "claude-working", "claude-rested", "claude-bash", "claude-edit"] {
            assert!(claude(name).is_none(), "{name}: {:?}", claude(name).map(|d| (d.kind, d.title)));
        }
        assert!(read("Pick one:\n  1. apples\n  2. pears\n\n❯ ", &Shapes::of("claude")).is_none(), "a list the agent printed above its box");
    }

    #[test]
    fn an_unmeasured_agent_s_dialog_still_reads_through_the_shared_shapes() {
        let d = read(&screen("claude-model"), &Shapes::of("pig")).unwrap();
        assert_eq!(d.kind, Kind::Select);
        assert_eq!(d.title, "Select model", "the first line of its block");
        let menu = read(&screen("claude-slash-menu"), &Shapes::of("pig")).unwrap();
        assert_eq!(menu.options.len(), 3);
    }

    #[test]
    fn a_yes_or_no_is_a_confirm() {
        let s = "▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔\n Clear the conversation?\n\n ❯ 1. Yes\n   2. No\n\n Enter to confirm · Esc to cancel\n";
        let d = read(s, &Shapes::of("claude")).unwrap();
        assert_eq!((d.kind, d.title.as_str()), (Kind::Confirm, "Clear the conversation?"));
        assert_eq!(d.json()["options"][0]["selected"], json!(true));
    }
}
