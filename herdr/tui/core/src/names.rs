// Prompt condensation adapts naming.sh from https://github.com/qu8n/herdr-automatic-rename (MIT).
// The word lists and condensation rules are translated to Rust; see THIRD_PARTY_NOTICES.md.
//! What cockpit calls an agent: the name the person gave it, else the agent's own title for its work, else its first
//! prompt in a few words, else the agent's name. A Herdr tab takes that name only while its label is a placeholder or
//! the one cockpit gave it last, so a name the person typed, or another tool gave, always stays.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};

use crate::files;
use crate::transcripts::{Detail, Follow, Transcripts};

/// The most characters an automatic name takes, as a tab label shows it.
pub const MAX: usize = 28;

/// Whether cockpit names agents, tabs and spaces itself: `names` in the config, on unless `false`.
pub fn on(config: &Value) -> bool {
    config["names"] != false
}

// ---------------------------------------------------------------------------------------------------------------------
// Who a tab's label belongs to

/// A label nobody chose: empty, or the number Herdr gives a new tab.
pub fn placeholder(label: &str) -> bool {
    let label = label.trim();
    label.is_empty() || label.chars().all(|c| c.is_ascii_digit())
}

/// A tab label that names nothing but what runs there, never shown as the agent's name: a placeholder, a shell, or the
/// agent by one of its names (`agent`, from `agent_names`). Only a placeholder is ever written over (`owner`).
pub fn default_label(label: &str, agent: &[&str]) -> bool {
    let label = label.trim().to_lowercase();
    placeholder(&label) || SHELLS.contains(&label.as_str()) || agent.iter().any(|n| !n.trim().is_empty() && n.trim().to_lowercase() == label)
}

/// The names a tab running `program` may carry before anyone names it: the program, and its agent's id, name,
/// executable and aliases.
pub fn agent_names(program: &str, agent: Option<&crate::agent::Agent>) -> Vec<String> {
    let mut out = vec![program.to_string()];
    if let Some(a) = agent {
        out.extend([a.id.clone(), a.name.clone(), a.bin.clone()]);
        out.extend(a.aliases.iter().cloned());
    }
    out.retain(|n| !n.trim().is_empty());
    out
}

/// Who a tab's label belongs to, from the label and the name cockpit gave that tab last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Nobody chose it: cockpit may name the tab.
    Placeholder,
    /// The name cockpit gave it last: cockpit may name it again.
    Cockpit,
    /// The person typed it, or another tool gave it: cockpit never writes over it.
    Someone,
}

/// Who owns `label`: a placeholder is nobody's, a label cockpit gave last is its own, anything else ("Claude" too) is
/// someone's.
pub fn owner(label: &str, given: Option<&str>) -> Owner {
    if placeholder(label) {
        Owner::Placeholder
    } else if given.is_some_and(|g| g == label) {
        Owner::Cockpit
    } else {
        Owner::Someone
    }
}

/// Whether cockpit writes `name` into a tab labelled `label`: a real name, different from the label, over a label that
/// is a placeholder or its own.
pub fn writes(label: &str, given: Option<&str>, name: &str, source: Source) -> bool {
    source != Source::Default && !name.trim().is_empty() && name != label && owner(label, given) != Owner::Someone
}

fn dir() -> PathBuf {
    files::cache().join("names")
}

fn given_path() -> PathBuf {
    dir().join("given.json")
}

/// The name cockpit gave each tab and space last, by id: read again only when the file changed.
pub fn given() -> HashMap<String, String> {
    type Kept = (Option<SystemTime>, HashMap<String, String>);
    static READ: Mutex<Option<Kept>> = Mutex::new(None);
    let path = given_path();
    let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut read = READ.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, map)) = read.as_ref()
        && *at == stamp
    {
        return map.clone();
    }
    let map: HashMap<String, String> = files::load(&path).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    *read = Some((stamp, map.clone()));
    map
}

/// Keeps the names cockpit gave, for `given` and the next round.
pub fn keep(given: &HashMap<String, String>) {
    let _ = std::fs::create_dir_all(dir());
    let _ = files::write(&given_path(), &json!(given));
}

// ---------------------------------------------------------------------------------------------------------------------
// Words

/// A terminal title or a label as one plain line (`text::clean`: no escape, control, bidi or zero-width characters,
/// whitespace folded), the glyphs an agent puts in front of it (a spinner, a mark) and behind it left out.
pub fn clean(raw: &str) -> String {
    // A CSI or OSC sequence goes whole; any other control character parts words.
    let spaced: String = crate::text::without_sequences(raw).chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let line = crate::text::clean(&spaced, 400);
    let line = line.trim_start_matches(|c: char| !c.is_alphanumeric());
    // Oh My Pi puts its brand before its state mark: taken off only when something other than a letter follows it.
    let line = match line.strip_prefix('π') {
        Some(rest) if !rest.starts_with(char::is_alphanumeric) => rest.trim_start_matches(|c: char| !c.is_alphanumeric()),
        _ => line,
    };
    let mut words: Vec<&str> = line.split_whitespace().collect();
    while words.last().is_some_and(|w| w.chars().all(|c| !c.is_ascii() && !c.is_alphanumeric())) {
        words.pop();
    }
    words.join(" ")
}

/// Titles that name the agent rather than its work, which an agent shows before it has a task.
const IDLE_TITLES: [&str; 8] = ["claude code", "codex cli", "gemini cli", "opencode", "amp code", "cursor agent", "new session", "untitled"];

const SHELLS: [&str; 5] = ["zsh", "bash", "fish", "sh", "nu"];

/// A title as a name for the work, None when it says nothing of it: empty, a number, the agent's own name (`refused`
/// holds its id, name and aliases), a shell, the folder it works in or a path to it, or a shell prompt.
pub fn task(raw: &str, refused: &[&str], folder: &str) -> Option<String> {
    let title = clean(raw);
    let lower = title.to_lowercase();
    let folder = folder.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_lowercase();
    let named = |n: &str| {
        let n = n.trim().to_lowercase();
        !n.is_empty() && [n.clone(), format!("{n} code"), format!("{n} cli")].contains(&lower)
    };
    let path_like = !lower.contains(char::is_whitespace) && (lower.starts_with(['/', '~']) || lower.contains('/'));
    let prompt_like = title.split_once(':').is_some_and(|(who, _)| who.contains('@') && !who.contains(char::is_whitespace));
    let refuse = placeholder(&title)
        || IDLE_TITLES.contains(&lower.as_str())
        || SHELLS.contains(&lower.as_str())
        || refused.iter().any(|n| named(n))
        || (!folder.is_empty() && (lower == folder || lower == folder.trim_start_matches('.')))
        || path_like
        || prompt_like;
    (!refuse).then_some(title)
}

/// `text` in at most `max` characters: cut at a space when that keeps at least half of them, with "…" when cut, and
/// no filler word or stray punctuation left before it ("Settings page copy…", not "Settings page copy for…").
pub fn cap(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    let Some(at) = cut.rfind(' ').filter(|&i| cut[..i].chars().count() >= max / 2) else { return format!("{}…", cut.trim_end()) };
    let mut words: Vec<&str> = cut[..at].split_whitespace().collect();
    while words.len() > 1 && words.last().is_some_and(|w| FILLER.contains(&w.to_lowercase().as_str())) {
        words.pop();
    }
    format!("{}…", words.join(" ").trim_end_matches([':', ',', ';', '-', '—', '·']))
}

/// Words a request opens with that say what kind of work it is rather than which: dropped at the front only.
const LEAD_VERBS: &[&str] = &[
    "add",
    "adjust",
    "analyze",
    "audit",
    "build",
    "check",
    "clean",
    "configure",
    "create",
    "debug",
    "delete",
    "diagnose",
    "draft",
    "explain",
    "explore",
    "find",
    "fix",
    "help",
    "implement",
    "improve",
    "investigate",
    "look",
    "make",
    "migrate",
    "optimize",
    "plan",
    "refactor",
    "remove",
    "rename",
    "research",
    "review",
    "run",
    "set",
    "setup",
    "show",
    "test",
    "troubleshoot",
    "update",
    "write",
];

/// Words that only spend room in a name: articles, prepositions, particles and the politeness around a request.
const FILLER: &[&str] = &[
    "a", "an", "the", "to", "for", "of", "on", "in", "at", "and", "or", "with", "from", "into", "via", "why", "how", "what", "that", "if", "whether", "is",
    "are", "be", "it", "its", "this", "up", "out", "off", "down", "over", "back", "please", "can", "could", "would", "you", "we", "i", "me", "my", "let's",
    "lets", "just", "some", "so", "per", "by", "as", "about", "then", "also", "when", "while",
];

/// A sentence as a few words for a name, in the order written: its leading verb and filler words dropped (an all-caps
/// identifier such as `IT` or `ENG-412` stays), whole words taken while they fit `max`. None when nothing is left.
pub fn condense(text: &str, max: usize) -> Option<String> {
    condensed(text, max, true)
}

fn condensed(text: &str, max: usize, lead: bool) -> Option<String> {
    let spaced: String = text.chars().map(|c| if "/,;:|()[]{}\"`".contains(c) { ' ' } else { c }).collect();
    let mut words: Vec<&str> = spaced.split_whitespace().map(|w| w.trim_matches(|c: char| ".!?'".contains(c))).filter(|w| !w.is_empty()).collect();
    let lower = |w: &str| w.to_lowercase();
    if lead && words.first().is_some_and(|w| LEAD_VERBS.contains(&lower(w).as_str())) {
        words.remove(0);
    }
    let identifier = |w: &str| w.chars().count() >= 2 && w.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-');
    words.retain(|w| identifier(w) || !FILLER.contains(&lower(w).as_str()));
    let mut out = String::new();
    for w in words {
        if out.is_empty() {
            out = w.chars().take(max).collect();
        } else if out.chars().count() + 1 + w.chars().count() <= max {
            out = format!("{out} {w}");
        } else {
            break;
        }
    }
    let mut chars = out.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().collect::<String>() + chars.as_str())
}

/// The first prompt as a name: its first line, a slash command without its slash, condensed; its leading verb stays when
/// nothing but a number would be left without it ("/review 412").
pub fn opening(text: &str) -> Option<String> {
    let line = clean(text.lines().map(str::trim).find(|l| !l.is_empty())?.trim_start_matches('/'));
    condense(&line, MAX).filter(|n| !placeholder(n)).or_else(|| condensed(&line, MAX, false).filter(|n| !placeholder(n)))
}

// ---------------------------------------------------------------------------------------------------------------------
// The name

/// Where an agent's name comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The person's: the tab's own label, or the name they gave the session in the agent.
    Person,
    /// The agent's own title for its work.
    Agent,
    /// The first prompt, in a few words.
    Prompt,
    /// The agent's name, when nothing says more.
    Default,
}

impl Source {
    /// How the agents view says it (`title_source`).
    pub fn word(self) -> &'static str {
        match self {
            Source::Person => "person",
            Source::Agent => "agent",
            Source::Prompt => "prompt",
            Source::Default => "default",
        }
    }
}

/// What an agent's name is made from.
#[derive(Clone, Debug, Default)]
pub struct Sources<'a> {
    /// Its tab's label, empty without a tab (a driven session).
    pub label: &'a str,
    /// The name cockpit gave that tab last.
    pub given: Option<&'a str>,
    /// The name the person gave the session in the agent itself.
    pub renamed: Option<&'a str>,
    /// The agent's own title for the session, from its transcript.
    pub titled: Option<&'a str>,
    /// Its terminal's title.
    pub terminal: &'a str,
    /// The first thing the person typed.
    pub opening: Option<&'a str>,
    /// Its id, name and aliases, which a title naming the agent says nothing beside.
    pub agent: Vec<&'a str>,
    /// What people call the agent: "Claude Code".
    pub agent_name: &'a str,
    /// The folder it works in.
    pub folder: &'a str,
}

/// The agent's name and where it comes from. With `auto` off it is the tab's label, else its terminal's title, else
/// the agent's name: nothing is read from its transcript.
pub fn pick(s: &Sources, auto: bool) -> (String, Source) {
    let tab = s.label.trim();
    // A label Herdr's own index prefix stands in front of ("[2] Fix the parser") is read without it.
    let bare = tab.strip_prefix('[').and_then(|r| r.split_once(']')).filter(|(n, _)| n.chars().all(|c| c.is_ascii_digit())).map_or(tab, |(_, r)| r.trim());
    // A label that only names what runs there ("Claude", "[2] claude") stays in the tab but is never shown as a name.
    let theirs = !default_label(bare, &s.agent) && (!auto || owner(tab, s.given) == Owner::Someone);
    if theirs && !bare.is_empty() {
        return (bare.to_string(), Source::Person);
    }
    if auto && let Some(name) = s.renamed.map(clean).filter(|n| !n.is_empty()) {
        return (cap(&name, MAX), Source::Person);
    }
    let titled = s.titled.filter(|_| auto).and_then(|t| task(t, &s.agent, s.folder));
    if let Some(title) = titled.or_else(|| task(s.terminal, &s.agent, s.folder)) {
        return (cap(&title, MAX), Source::Agent);
    }
    if auto && let Some(name) = s.opening.and_then(opening) {
        return (name, Source::Prompt);
    }
    let fallback = if s.agent_name.trim().is_empty() { "The agent" } else { s.agent_name };
    (fallback.to_string(), Source::Default)
}

// ---------------------------------------------------------------------------------------------------------------------
// Transcripts

/// What a session's transcript says of its name: the person's rename, the agent's title, the first thing typed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Read {
    pub renamed: Option<String>,
    pub titled: Option<String>,
    pub opening: Option<String>,
}

/// How long a transcript in another format than Claude Code's, read whole, is not read again for its name.
const REREAD: Duration = Duration::from_secs(30);

/// A transcript's names: a Claude Code one followed as it grows, any other read again at most every REREAD.
pub fn read(kind: Transcripts, path: &Path) -> Option<Read> {
    type Followed = HashMap<PathBuf, (Follow, Option<Instant>, Option<Read>)>;
    static FOLLOWED: Mutex<Option<Followed>> = Mutex::new(None);
    let mut guard = FOLLOWED.lock().unwrap_or_else(|e| e.into_inner());
    let followed = guard.get_or_insert_with(HashMap::new);
    if followed.len() > 64 && !followed.contains_key(path) {
        followed.clear();
    }
    let (follow, at, kept) = followed.entry(path.to_path_buf()).or_insert_with(|| (Follow::new(kind, path, Detail::Names), None, None));
    if kind != Transcripts::Claude && at.is_some_and(|t| t.elapsed() < REREAD) {
        return kept.clone();
    }
    *at = Some(Instant::now());
    *kept = follow.read().map(|t| Read { renamed: t.named.clone(), titled: t.titled.clone(), opening: t.opening.clone() });
    kept.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> Vec<&'static str> {
        vec!["claude", "Claude Code", "claude-code"]
    }

    #[test]
    fn a_title_is_cleaned_of_what_the_agent_puts_around_it() {
        assert_eq!(clean("✳ Fixing quotas"), "Fixing quotas");
        assert_eq!(clean("⠂⠐ Daily  quota\tlimiter ✻"), "Daily quota limiter");
        assert_eq!(clean("π > Fix the parser"), "Fix the parser", "oh-my-pi's brand and its state mark");
        assert_eq!(clean("\u{7}\u{1b}Run\ttests\n"), "Run tests", "control characters");
        assert_eq!(clean("πr² formula"), "πr² formula", "a word that merely starts with the brand");
        assert_eq!(clean("Überprüfung der Tests"), "Überprüfung der Tests", "a first letter outside ASCII stays");
        assert_eq!(clean("Fix\u{202e}gnp.exe\u{200b} the\u{2066} parser\u{fe0f}"), "Fixgnp.exe the parser", "bidi, zero-width and format characters go");
        assert_eq!(clean("\u{1b}]0;pwned\u{7}Quota work"), "Quota work", "an escape sequence goes whole");
        let long = Sources { label: "3", renamed: Some(&"Quota work ".repeat(20)), agent_name: "Claude Code", ..Default::default() };
        assert!(pick(&long, true).0.chars().count() <= MAX, "a rename is capped like any name");
    }

    #[test]
    fn a_title_that_names_the_agent_or_its_folder_says_nothing() {
        let a = claude();
        assert_eq!(task("✳ Claude Code", &a, "/r/api"), None);
        assert_eq!(task("claude", &a, "/r/api"), None);
        assert_eq!(task("Codex CLI", &["codex", "Codex"], "/r/api"), None);
        assert_eq!(task("New session", &a, "/r/api"), None);
        assert_eq!(task("untitled", &a, "/r/api"), None);
        assert_eq!(task("42", &a, "/r/api"), None, "Herdr's own number handed back");
        assert_eq!(task("api", &a, "/r/api"), None, "the folder");
        assert_eq!(task("~/r/api", &a, "/r/api"), None, "a path");
        assert_eq!(task("me@laptop:~/acme/api", &a, "/r/api"), None, "a shell prompt");
        assert_eq!(task("zsh", &a, "/r/api"), None);
        assert_eq!(task("", &a, "/r/api"), None);
        assert_eq!(task("Fix build for services/api", &a, "/r/api").as_deref(), Some("Fix build for services/api"), "a sentence ending on the folder");
        assert_eq!(task("⠂ Daily quota limiter", &a, "/r/api").as_deref(), Some("Daily quota limiter"));
    }

    #[test]
    fn a_name_is_capped_at_a_word() {
        assert_eq!(cap("Daily quota limiter", MAX), "Daily quota limiter");
        assert_eq!(cap("Preview environments: shorter default lifetime", MAX), "Preview environments…");
        assert_eq!(cap("Settings page copy for quotas", MAX), "Settings page copy…", "no filler word before the cut");
        assert_eq!(cap("Supercalifragilisticexpialidocious tail", MAX), "Supercalifragilisticexpiali…", "no space keeps half: cut inside");
        assert!(cap("Preview environments: shorter default lifetime", MAX).chars().count() <= MAX);
    }

    #[test]
    fn a_prompt_is_condensed_to_its_words() {
        assert_eq!(condense("Fix the Stripe webhook retries", MAX).as_deref(), Some("Stripe webhook retries"));
        assert_eq!(condense("Adjust the screensaver timeout on the Ubuntu box", MAX).as_deref(), Some("Screensaver timeout Ubuntu"));
        assert_eq!(condense("Investigate IT outage", MAX).as_deref(), Some("IT outage"), "an identifier is not filler");
        assert_eq!(condense("can you look at ENG-412 for me please", MAX).as_deref(), Some("Look ENG-412"));
        assert_eq!(condense("the a of", MAX), None);
        assert_eq!(condense("Supercalifragilisticexpialidocious-and-more stuff", 10).as_deref(), Some("Supercalif"), "the first word cut");
    }

    #[test]
    fn an_opening_reads_its_first_line_and_a_command_without_its_slash() {
        assert_eq!(opening("\n  Store and enforce the daily quotas for ENG-412: count exports\nmore").as_deref(), Some("Store enforce daily quotas"));
        assert_eq!(opening("/review 412").as_deref(), Some("Review 412"), "the verb stays when only a number would be left");
        assert_eq!(opening("/deploy-preview api").as_deref(), Some("Deploy-preview api"));
        assert_eq!(opening("   "), None);
    }

    #[test]
    fn a_placeholder_is_empty_or_a_number() {
        assert!(placeholder(""));
        assert!(placeholder(" 3 "));
        assert!(!placeholder("3a"));
        assert!(!placeholder("quotas"));
    }

    #[test]
    fn cockpit_names_a_tab_only_while_the_label_is_a_placeholder_or_its_own() {
        let ours = Some("Daily quota limiter");
        assert_eq!(owner("3", ours), Owner::Placeholder);
        assert_eq!(owner("", None), Owner::Placeholder);
        assert_eq!(owner("Daily quota limiter", ours), Owner::Cockpit);
        assert_eq!(owner("my tab", ours), Owner::Someone, "the person renamed it");
        assert_eq!(owner("[2] claude", None), Owner::Someone, "another renamer's: never written over");
        // The rounds: a placeholder is named, its own name moves with the work, the person's rename stops it for good,
        // and clearing the label hands it back.
        assert!(writes("3", None, "Daily quota limiter", Source::Agent));
        assert!(writes("Daily quota limiter", ours, "Rate limits", Source::Prompt));
        assert!(!writes("my tab", ours, "Rate limits", Source::Agent));
        assert!(!writes("my tab", Some("Rate limits"), "Rate limits", Source::Agent));
        assert!(writes("", Some("Rate limits"), "Rate limits", Source::Agent));
        assert!(!writes("3", None, "Claude Code", Source::Default), "never the agent's bare name");
        assert!(!writes("Rate limits", Some("Rate limits"), "Rate limits", Source::Agent), "nothing to change");
    }

    #[test]
    fn the_name_follows_its_sources_in_order() {
        let base = Sources { agent: claude(), agent_name: "Claude Code", folder: "/r/api", ..Default::default() };
        let all = Sources {
            label: "quotas",
            renamed: Some("Quota work"),
            titled: Some("Daily quota limiter"),
            terminal: "✳ Daily quota limiter",
            opening: Some("Store and enforce the daily quotas"),
            ..base.clone()
        };
        assert_eq!(pick(&all, true), ("quotas".into(), Source::Person), "the label the person typed first");
        let ours = Sources { given: Some("quotas"), ..all.clone() };
        assert_eq!(pick(&ours, true), ("Quota work".into(), Source::Person), "cockpit's own label: the person's rename");
        let no_rename = Sources { label: "3", renamed: None, ..all.clone() };
        assert_eq!(pick(&no_rename, true), ("Daily quota limiter".into(), Source::Agent));
        let terminal = Sources { label: "3", renamed: None, titled: None, terminal: "⠐ Fixing the CSV export", ..all.clone() };
        assert_eq!(pick(&terminal, true), ("Fixing the CSV export".into(), Source::Agent), "the terminal's title, cleaned");
        let prompt = Sources { label: "", renamed: None, titled: None, terminal: "✳ Claude Code", ..all.clone() };
        assert_eq!(pick(&prompt, true), ("Store enforce daily quotas".into(), Source::Prompt));
        assert_eq!(pick(&base, true), ("Claude Code".into(), Source::Default));
        let program = Sources { label: "[2] claude", ..base.clone() };
        assert_eq!(pick(&program, true), ("Claude Code".into(), Source::Default), "a renamer's label of the program names nothing");
        let indexed = Sources { label: "[1] Debug transient issue", ..base.clone() };
        assert_eq!(pick(&indexed, true), ("Debug transient issue".into(), Source::Person));
    }

    #[test]
    fn a_tab_labelled_with_its_program_a_shell_or_its_agent_s_name_is_not_shown_as_its_name_nor_written_over() {
        let table = crate::agent::builtin();
        let codex = table.iter().find(|a| a.id == "codex");
        let names = agent_names("codex", codex);
        let agent: Vec<&str> = names.iter().map(String::as_str).collect();
        // Herdr 0.9.5 labels a new tab with its number; a tab's program, a shell or the agent's own name say no more.
        for label in ["2", "codex", "Codex", " zsh ", "fish", ""] {
            assert!(default_label(label, &agent), "{label:?}");
            let written = placeholder(label);
            assert_eq!(writes(label, None, "Fix the CSV export", Source::Agent), written, "{label:?}: only a placeholder is written over");
            let s = Sources { label, terminal: "Fix the CSV export", agent: agent.clone(), agent_name: "Codex", ..Default::default() };
            assert_eq!(pick(&s, true), ("Fix the CSV export".into(), Source::Agent), "{label:?} is never shown as the person's");
            assert_eq!(pick(&s, false).1, Source::Agent, "{label:?} with names off");
        }
        let claude: Vec<&str> = claude();
        assert!(default_label("claude-code", &claude) && default_label("Claude Code", &claude));
        assert!(!default_label("claude", &agent), "another agent's name is a name like any other");
        // A person who names a tab "Claude", or a renamer's bare "claude", keeps it: cockpit only shows another name.
        for label in ["Claude", "claude", "[2] claude", "zsh"] {
            assert_eq!(owner(label, Some("Daily quota limiter")), Owner::Someone, "{label:?}");
            assert!(!writes(label, None, "Daily quota limiter", Source::Agent), "{label:?} is never written over");
            let s = Sources { label, titled: Some("Daily quota limiter"), agent: claude.clone(), agent_name: "Claude Code", ..Default::default() };
            assert_eq!(pick(&s, true), ("Daily quota limiter".into(), Source::Agent), "{label:?} is not shown as the name");
        }
        assert!(!default_label("codex review", &agent) && !default_label("quotas", &agent));
        assert_eq!(agent_names("acme", None), ["acme"], "a program the table does not know is its own name");
    }

    #[test]
    fn a_label_someone_chose_is_shown_and_never_written_over() {
        for label in ["api", "~/r/api", "🐛 quotas", "[3] Quota review"] {
            let s = Sources {
                label,
                given: Some("Daily quota limiter"),
                terminal: "A different task",
                agent: claude(),
                agent_name: "Claude Code",
                folder: "/r/api",
                ..Default::default()
            };
            let shown = label.strip_prefix("[3] ").unwrap_or(label).to_string();
            assert_eq!(pick(&s, true), (shown.clone(), Source::Person));
            assert_eq!(pick(&s, false), (shown, Source::Person));
            assert!(!writes(label, s.given, "A different task", Source::Agent));
        }
    }

    #[test]
    fn with_names_off_nothing_is_read_from_the_transcript() {
        let s = Sources {
            label: "3",
            renamed: Some("Quota work"),
            titled: Some("Daily quota limiter"),
            terminal: "",
            opening: Some("Store the quotas"),
            agent: claude(),
            agent_name: "Claude Code",
            folder: "/r/api",
            given: None,
        };
        assert_eq!(pick(&s, false), ("Claude Code".into(), Source::Default));
        let labelled = Sources { label: "Quota work", given: Some("Quota work"), ..s.clone() };
        assert_eq!(pick(&labelled, false), ("Quota work".into(), Source::Person), "off, any label set stands");
    }

    #[test]
    fn the_config_turns_it_off() {
        assert!(on(&Value::Null));
        assert!(on(&json!({"names": true})));
        assert!(!on(&json!({"names": false})));
    }

    #[test]
    fn a_claude_transcript_gives_its_rename_title_and_opening_as_it_grows() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c-1.jsonl");
        let line = |v: Value| format!("{v}\n");
        std::fs::write(
            &path,
            [
                line(
                    json!({"type": "user", "sessionId": "c-1", "message": {"content": "<command-name>/review</command-name><command-args>412</command-args>"}}),
                ),
                line(json!({"type": "user", "sessionId": "c-1", "message": {"content": "Store the quotas"}})),
            ]
            .concat(),
        )
        .unwrap();
        let first = read(Transcripts::Claude, &path).unwrap();
        assert_eq!(first, Read { renamed: None, titled: None, opening: Some("review 412".into()) }, "a command first, as name and args");
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(line(json!({"type": "ai-title", "aiTitle": "Daily quota limiter"})).as_bytes()).unwrap();
        f.write_all(line(json!({"type": "custom-title", "customTitle": "Quota work"})).as_bytes()).unwrap();
        drop(f);
        // A grown file is read on from where the last read stopped; the stamp has to move for that.
        std::thread::sleep(Duration::from_millis(20));
        let grown = read(Transcripts::Claude, &path).unwrap();
        assert_eq!(grown.titled.as_deref(), Some("Daily quota limiter"));
        assert_eq!(grown.renamed.as_deref(), Some("Quota work"));
        assert_eq!(grown.opening.as_deref(), Some("review 412"));
    }
}
