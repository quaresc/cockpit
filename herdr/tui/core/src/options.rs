//! The options a person sets, each listed once with its section, its kind, its default and the version that brought it,
//! and what of them was shown: the update dialog, `cockpit options`, setup and Cockpit settings all read this list.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{agent, files, jsonedit};

/// The version the options below arrive in: the one number to change at release.
pub const NEXT: &str = "1.1.0";

/// The side browser that is cockpit's own Chromium window, which only the Herdr app on macOS shows.
pub const CHROMIUM: &str = "chromium";

/// Whether this machine can run the Chromium side browser.
pub const CHROMIUM_HERE: bool = cfg!(target_os = "macos");

/// The layout an update offers to try, once, to a window on another layout, and the version that brought it.
pub const LAYOUT: (&str, &str) = ("cockpit-chat", "1.0.0");

/// The values a choice takes, each with its name in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choices {
    Fixed(&'static [(&'static str, &'static str)]),
    /// The installed agents by id, and "" for the first one installed.
    Agents,
}

/// What a text field takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// One line of words.
    Line,
    /// One word, with no space in it and not starting with a dash.
    Word,
    /// A time of day, `08:30`.
    Time,
    /// A folder that exists, written from the root or from `~`, the home folder.
    Folder,
    /// A branch name with `{prefix}`, `{key}` and `{slug}` in it.
    Branch,
}

/// How an option is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// On or off.
    Toggle,
    /// One of a few values.
    Choice(Choices),
    /// A short field; `blank` says what an empty one means, and without it an empty one is refused.
    Text { check: Check, blank: Option<&'static str> },
    /// A whole number between two bounds, in a unit.
    Number { min: i64, max: i64, unit: &'static str },
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Toggle => "toggle",
            Kind::Choice(_) => "choice",
            Kind::Text { .. } => "text",
            Kind::Number { .. } => "number",
        }
    }
}

/// An option's value when config.json says nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Def {
    On,
    Off,
    Word(&'static str),
    Count(i64),
}

/// One option: its dotted key in config.json, its section, the version that brought it (None for one cockpit always
/// read), its name, one plain line, its default and how it is set.
#[derive(Debug)]
pub struct Setting {
    pub key: &'static str,
    pub section: &'static str,
    pub since: Option<&'static str>,
    pub title: &'static str,
    pub description: &'static str,
    pub default: Def,
    pub kind: Kind,
}

/// The sections, in the order they are listed.
pub const SECTIONS: [&str; 10] = ["Agents", "Queue and CI", "Chat", "Usage", "Start work", "Sources", "Refresh", "Notifications", "Briefs and recaps", "Apps"];

const fn toggle(key: &'static str, section: &'static str, title: &'static str, description: &'static str, on: bool) -> Setting {
    Setting { key, section, since: None, title, description, default: if on { Def::On } else { Def::Off }, kind: Kind::Toggle }
}

const fn choice(
    key: &'static str,
    section: &'static str,
    title: &'static str,
    description: &'static str,
    default: &'static str,
    values: &'static [(&'static str, &'static str)],
) -> Setting {
    Setting { key, section, since: None, title, description, default: Def::Word(default), kind: Kind::Choice(Choices::Fixed(values)) }
}

const fn text(
    key: &'static str,
    section: &'static str,
    title: &'static str,
    description: &'static str,
    default: &'static str,
    check: Check,
    blank: Option<&'static str>,
) -> Setting {
    Setting { key, section, since: None, title, description, default: Def::Word(default), kind: Kind::Text { check, blank } }
}

const fn number(
    key: &'static str,
    section: &'static str,
    title: &'static str,
    description: &'static str,
    default: i64,
    (min, max, unit): (i64, i64, &'static str),
) -> Setting {
    Setting { key, section, since: None, title, description, default: Def::Count(default), kind: Kind::Number { min, max, unit } }
}

/// A source's interval: at least the 15 seconds the hub holds it to, at most a day.
const fn refresh(key: &'static str, title: &'static str, description: &'static str, default: i64) -> Setting {
    number(key, "Refresh", title, description, default, (15, 86_400, "s"))
}

const fn brought(setting: Setting, version: &'static str) -> Setting {
    Setting { since: Some(version), ..setting }
}

/// Every option, section by section, in the order it is listed and asked.
pub const ALL: &[Setting] = &[
    Setting {
        key: "agents.default",
        section: "Agents",
        since: None,
        title: "The agent cockpit starts",
        description: "Start work, the inbox and one-off jobs start this agent when nothing else says which.",
        default: Def::Word(""),
        kind: Kind::Choice(Choices::Agents),
    },
    toggle(
        "names",
        "Agents",
        "Name agents after their work",
        "Agents, tabs and spaces take the name you gave, else the agent's own title, else its first prompt in a few words.",
        true,
    ),
    brought(
        toggle(
            "queue.drain",
            "Queue and CI",
            "Send queued prompts on their own",
            "A prompt you queue for an agent goes by itself once the agent rests; off, you send each one.",
            true,
        ),
        "1.0.0",
    ),
    brought(
        toggle(
            "ci.tellAgents",
            "Queue and CI",
            "Tell agents when their CI fails",
            "A failed CI run on an agent's pull request is queued to that agent, once per commit.",
            true,
        ),
        "1.0.0",
    ),
    toggle(
        "github.reviewBots",
        "Queue and CI",
        "Hand bots' review comments to agents",
        "Address review gives an agent the threads bots opened too, after the ones people opened.",
        false,
    ),
    brought(
        toggle(
            "chat.nextSteps",
            "Chat",
            "Suggest next prompts",
            "After each turn, a few prompts to send next, written by the agent and billed to it. For an agent that \
             cannot answer with no tools, Claude Code writes them from the conversation's last part, billed to your Claude \
             account.",
            false,
        ),
        "1.0.0",
    ),
    text(
        "chat.nextStepsModel",
        "Chat",
        "Model for next prompts",
        "The model that writes the suggested prompts, such as haiku.",
        "",
        Check::Word,
        Some("the conversation's own model"),
    ),
    choice(
        "agents.pace",
        "Usage",
        "Pace of the 7-day limit",
        "Where the tick on the 7-day bar stands: spread over the whole week, or 20% per weekday and still over the weekend.",
        "week",
        &[("week", "Whole week"), ("weekdays", "Weekdays")],
    ),
    text("reposDir", "Start work", "Folder of your checkouts", "The folder that holds your git checkouts.", "~", Check::Folder, None),
    toggle(
        "worktrees",
        "Start work",
        "Start inbox work in a worktree",
        "The inbox starts work on a branch checked out nowhere in a new worktree, rather than in the checkout itself.",
        false,
    ),
    text(
        "start.branch",
        "Start work",
        "Branch name",
        "The branch Start work makes: {key} is the ticket's key, {slug} a few words of its title or of the prompt.",
        "",
        Check::Branch,
        Some("{prefix}/{key}-{slug}"),
    ),
    text("start.prefix", "Start work", "Branch prefix", "What {prefix} stands for in the branch name, such as your initials.", "", Check::Word, Some("none")),
    toggle("github.enabled", "Sources", "GitHub pull requests", "Your pull requests and the reviews asked of you, read through gh.", true),
    text("github.org", "Sources", "GitHub organization", "Only this organization's pull requests are read.", "", Check::Word, Some("every organization")),
    toggle("ci.enabled", "Sources", "CI runs", "The CI runs of your pull requests, read from GitHub checks or your CI's connector.", true),
    toggle(
        "fetcher",
        "Sources",
        "A Claude session reads what the hub cannot",
        "The hub keeps one Claude Code session in the background for the sources only a Claude connector reaches.",
        true,
    ),
    refresh("refresh.github", "GitHub", "Seconds between two reads of your pull requests.", 30),
    refresh("refresh.tickets", "Tickets", "Seconds between two reads of your tickets.", 60),
    refresh("refresh.ci", "CI", "Seconds between two reads of the CI runs.", 30),
    refresh("refresh.mail", "Mail", "Seconds between two reads of your unread mail.", 60),
    refresh("refresh.calendar", "Calendar", "Seconds between two reads of your meetings; an iCal address left alone is read every 300.", 120),
    refresh("refresh.platform", "Deploys and previews", "Seconds between two reads of the deploys and preview environments.", 60),
    refresh("refresh.monitor", "Release monitor", "Seconds between two reads of the releases' error rate and latency.", 120),
    toggle("notifications", "Notifications", "Notifications", "Cockpit notifies you: an agent that needs you, a failed CI run, a review asked.", true),
    choice(
        "monitor.notify",
        "Notifications",
        "Releases that notify you",
        "Which releases the monitor notifies you about when their errors or latency move.",
        "mine",
        &[("mine", "Mine"), ("all", "All"), ("none", "None")],
    ),
    toggle("brief.enabled", "Briefs and recaps", "Morning brief", "Each morning, your meetings, reviews, CI and tickets in one page.", true),
    text("brief.at", "Briefs and recaps", "Morning brief time", "When the morning brief is written, such as 08:30.", "08:30", Check::Time, None),
    toggle("brief.weekends", "Briefs and recaps", "Morning brief on weekends", "The morning brief is written on Saturday and Sunday too.", false),
    toggle("recap.enabled", "Briefs and recaps", "Recap when you come back", "Coming back to an agent's tab after a while opens a recap of what it did.", true),
    number(
        "recap.awayMinutes",
        "Briefs and recaps",
        "Minutes away before a recap",
        "How long you were away from a tab before coming back opens its recap.",
        10,
        (0, 1_440, "min"),
    ),
    choice(
        "agents.recap",
        "Briefs and recaps",
        "Recap written after a turn",
        "A recap of an agent's turn is written a minute after it ends, or only when you ask for one.",
        "auto",
        &[("auto", "On its own"), ("manual", "When asked")],
    ),
    toggle("meetingBrief.enabled", "Briefs and recaps", "Meeting briefs", "What to read before a meeting, written ahead of it.", true),
    text("meetingBrief.model", "Briefs and recaps", "Model for meeting briefs", "The model that writes the meeting briefs.", "sonnet", Check::Word, None),
    toggle(
        "deployWatch.enabled",
        "Briefs and recaps",
        "Check errors after a deploy",
        "After a deploy of yours, an agent reads the errors through your observability connector.",
        false,
    ),
    number("deployWatch.afterMinutes", "Briefs and recaps", "Minutes after a deploy", "How long after a deploy its errors are read.", 30, (1, 1_440, "min")),
    choice(
        "density",
        "Apps",
        "Rows",
        "Comfortable shows a row's details under the list; compact puts its buttons on the row and fits more rows.",
        "comfortable",
        &[("comfortable", "Comfortable"), ("compact", "Compact")],
    ),
    text("editor", "Apps", "Editor", "The command Open in editor runs, such as code, cursor or zed.", "", Check::Word, Some("code")),
    choice(
        "e2eBrowser",
        "Apps",
        "Browser for E2E tests",
        "Where an agent checks a page: in a browser pane beside it, or in Claude in Chrome.",
        "terminal-browser",
        &[("terminal-browser", "Beside the agent"), ("chrome", "Chrome")],
    ),
    number(
        "browserIdleMinutes",
        "Apps",
        "Minutes before an idle browser closes",
        "A browser pane whose space stayed out of view this long closes, and a Chromium tab out of view this long sleeps; 0 keeps them.",
        60,
        (0, 10_080, "min"),
    ),
    brought(
        choice(
            "sideBrowser",
            "Apps",
            "Side browser",
            "The browser beside an agent: terminal-browser in a pane, or a Chromium window docked in the Herdr app's side panel on macOS.",
            "terminal-browser",
            &[("terminal-browser", "In a terminal pane"), (CHROMIUM, "Chromium, docked")],
        ),
        NEXT,
    ),
    toggle(
        "browser.dock",
        "Apps",
        "Dock the Chromium browser",
        "The Chromium browser sits in the side panel and follows the window; off, it is a window of its own.",
        true,
    ),
    choice(
        "browser.profileScope",
        "Apps",
        "Browser profiles",
        "The sign-ins a worktree's Chromium browser starts with: one profile for all, one per repo, or one per worktree.",
        "shared",
        &[("shared", "One for all"), ("repo", "One per repo"), ("worktree", "One per worktree")],
    ),
    choice(
        "browser.popups",
        "Apps",
        "Pop-up windows",
        "A page's pop-up opens as a tab of the Chromium browser, or is blocked.",
        "tab",
        &[("tab", "Open as a tab"), ("block", "Block")],
    ),
    text("browser.downloads", "Apps", "Downloads folder", "Where the Chromium browser saves what you download.", "", Check::Folder, Some("~/Downloads")),
];

/// What config.json holds that no switch or field sets: its key and one line, in the order it is listed.
pub const IN_FILE: &[(&str, &str)] = &[
    ("spaces", "The pinned spaces, in order: a name, or {name, cwd, icon, key} for one of your own."),
    ("agents.commands", "An agent's command line overridden field by field, by agent id; a new id adds an agent."),
    ("usage.commands", "A command per agent id that prints its limits, for a provider cockpit has no source for."),
    ("chat.nextStepsModel", "Next prompts' model per agent id, as {\"claude\": \"haiku\"}, in place of one name."),
    ("chat.nextStepsAgent", "The agent that writes next prompts for one that cannot answer with no tools."),
    ("start.setup", "A command per repo folder that Start work runs in a new worktree before the agent."),
    ("tickets", "Your tickets' connector: enabled, name, server, keys, urlTemplate, token, and its tools and fields."),
    ("ci", "Your CI's connector: name, server, imageJob, token, and its tools and fields."),
    ("calendar", "Your meetings: enabled, server and calendars, or ics and email for the hub to read them itself."),
    ("mail", "Your unread mail: enabled, name, server, and its tools and fields."),
    ("platform", "Deploys and previews: enabled, server, previewPrefix, the page addresses, tools and fields."),
    ("code", "Pull requests from another forge than GitHub: server, tools, fields."),
    ("servers", "The MCP servers cockpit calls itself, each with its URL or command."),
    ("metabase, datadog", "The connectors show panes refresh from: {server}."),
    ("observability", "Errors and latency from another tool than Datadog: server, tools, fields, queries."),
    ("monitor", "The release monitor: env, queries, the moved thresholds, serviceReplace."),
    ("meetingBrief", "Meeting briefs: skip (events left out by name), tools, agent, and a model per agent id."),
    ("deployWatch", "The check after a deploy: tools, agent."),
    ("runbookTools", "Extra tools a runbook's background refresh may use."),
    ("footer.links", "The links under Claude Code's prompt: on in a terminal, off in the Herdr app."),
    ("artifactGuard", "false lets every claude.ai artifact through, for a machine with no pane to draw in."),
    ("frontend", "tui or app, when cockpit cannot tell which it runs in."),
];

/// Whether `a` is a later version than `b`, number by number.
pub fn is_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (a, b) = (parts(a), parts(b));
    (0..a.len().max(b.len())).map(|i| (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0))).find(|(x, y)| x != y).is_some_and(|(x, y)| x > y)
}

pub fn get(key: &str) -> Option<&'static Setting> {
    ALL.iter().find(|s| s.key == key)
}

/// The options a version later than `version` brought, in the list's order.
pub fn since(version: &str) -> Vec<&'static Setting> {
    ALL.iter().filter(|s| s.since.is_some_and(|since| is_newer(since, version))).collect()
}

/// The option's default as config.json would hold it.
pub fn default_of(setting: &Setting) -> Value {
    match setting.default {
        Def::On => json!(true),
        Def::Off => json!(false),
        Def::Word(word) => json!(word),
        Def::Count(n) => json!(n),
    }
}

/// The values a choice takes with their names, the installed agents read now; empty for another kind.
pub fn choices(setting: &Setting) -> Vec<(String, String)> {
    offered(setting, CHROMIUM_HERE)
}

/// The values a choice takes on a machine that can, or cannot, run the Chromium side browser.
fn offered(setting: &Setting, chromium: bool) -> Vec<(String, String)> {
    match setting.kind {
        Kind::Choice(Choices::Fixed(values)) => {
            let here = |value: &str| chromium || !(setting.key == "sideBrowser" && value == CHROMIUM);
            values.iter().filter(|(value, _)| here(value)).map(|(value, label)| (value.to_string(), label.to_string())).collect()
        }
        Kind::Choice(Choices::Agents) => std::iter::once((String::new(), "The first installed".to_string()))
            .chain(agent::all().into_iter().filter(agent::Agent::installed).map(|a| (a.id, a.name)))
            .collect(),
        _ => Vec::new(),
    }
}

/// What config.json holds at the option's key, Null when nothing.
fn written<'a>(config: &'a Value, setting: &Setting) -> &'a Value {
    setting.key.split('.').fold(config, |node, part| &node[part])
}

/// Whether a value is of the option's kind: a boolean, a string or a whole number.
fn fits(setting: &Setting, value: &Value) -> bool {
    match setting.kind {
        Kind::Toggle => value.is_boolean(),
        Kind::Choice(_) | Kind::Text { .. } => value.is_string(),
        Kind::Number { .. } => value.is_i64() || value.is_u64(),
    }
}

/// The option's value in a config: the one written at its key when it is of its kind, else its default.
pub fn value(config: &Value, setting: &Setting) -> Value {
    let found = written(config, setting);
    if fits(setting, found) { found.clone() } else { default_of(setting) }
}

/// Whether a switch is on in a config.
pub fn is_on(config: &Value, setting: &Setting) -> bool {
    value(config, setting) == true
}

/// Whether config.json holds an object or a list there, which no switch or field shows (a model per agent): set by
/// hand. A value of another wrong kind reads as the default, and the next save replaces it.
pub fn by_hand(config: &Value, setting: &Setting) -> bool {
    let found = written(config, setting);
    (found.is_object() || found.is_array()) && !fits(setting, found)
}

/// A value in words, as the list and the toasts say it: on, off, a choice's name, a number with its unit.
pub fn words(setting: &Setting, value: &Value) -> String {
    match (setting.kind, value) {
        (Kind::Toggle, Value::Bool(on)) => if *on { "on" } else { "off" }.to_string(),
        (Kind::Choice(_), Value::String(picked)) => {
            choices(setting).into_iter().find(|(v, _)| v == picked).map(|(_, label)| label).unwrap_or_else(|| picked.clone())
        }
        (Kind::Text { blank, .. }, Value::String(text)) if text.is_empty() => blank.unwrap_or("empty").to_string(),
        (Kind::Number { unit, .. }, Value::Number(n)) => format!("{n} {unit}").trim_end().to_string(),
        (_, Value::String(text)) => text.clone(),
        (_, other) => other.to_string(),
    }
}

/// The home folder's `~` spelled out.
fn expanded(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => files::home().join(rest.trim_start_matches('/')),
        _ => PathBuf::from(path),
    }
}

/// A text field's value as it is saved, or why it is refused, in the option's own words.
fn checked_text(setting: &Setting, check: Check, blank: Option<&'static str>, text: &str) -> Result<String, String> {
    let title = setting.title;
    let text = text.trim();
    if text.chars().any(char::is_control) || text.chars().count() > 200 {
        return Err(format!("{title} takes one short line"));
    }
    if text.is_empty() {
        return if blank.is_some() { Ok(String::new()) } else { Err(format!("{title} cannot be empty")) };
    }
    match check {
        Check::Line => Ok(text.to_string()),
        Check::Word if text.contains(char::is_whitespace) => Err(format!("{title} takes one word, with no space")),
        Check::Word if text.starts_with('-') => Err(format!("{title} cannot start with a dash")),
        Check::Word => Ok(text.to_string()),
        Check::Time => {
            let parts = text.split_once(':').and_then(|(h, m)| Some((h.parse::<u8>().ok()?, m.parse::<u8>().ok()?)));
            match parts {
                Some((h, m)) if h < 24 && m < 60 && text.len() <= 5 => Ok(format!("{h:02}:{m:02}")),
                _ => Err(format!("{title} takes a time of day, such as 08:30")),
            }
        }
        Check::Folder if !(text.starts_with('/') || text == "~" || text.starts_with("~/")) => {
            Err(format!("{title} is written from the root (/…) or from your home folder (~/…)"))
        }
        Check::Folder if expanded(text).is_dir() => Ok(text.to_string()),
        Check::Folder => Err(format!("{text} is not a folder")),
        Check::Branch => {
            let bare = ["{prefix}", "{key}", "{slug}"].iter().fold(text.to_string(), |name, part| name.replace(part, "x"));
            let fine = |c: char| c.is_ascii_alphanumeric() || "/-_.".contains(c);
            if !bare.chars().all(fine) {
                Err(format!("{title} takes letters, digits, / - _ . and {{prefix}}, {{key}}, {{slug}}"))
            } else if !text.contains("{key}") && !text.contains("{slug}") {
                Err(format!("{title} needs {{key}} or {{slug}}, so two branches differ"))
            } else {
                Ok(text.to_string())
            }
        }
    }
}

/// The value as it is saved for this option, or why it is refused: a boolean for a switch, one of a choice's values, a
/// field's text once checked, a whole number within its bounds.
pub fn checked(setting: &Setting, value: &Value) -> Result<Value, String> {
    let title = setting.title;
    match setting.kind {
        Kind::Toggle => value.as_bool().map(Value::Bool).ok_or_else(|| format!("{title} is on or off")),
        Kind::Choice(_) => {
            let allowed = choices(setting);
            let picked =
                value.as_str().filter(|picked| allowed.iter().any(|(v, _)| v == picked) || (setting.key == "agents.default" && agent::get(picked).is_some()));
            let names = || allowed.iter().map(|(v, _)| if v.is_empty() { "\"\"" } else { v.as_str() }).collect::<Vec<_>>().join(", ");
            picked.map(|picked| json!(picked)).ok_or_else(|| format!("{title} takes one of: {}", names()))
        }
        Kind::Text { check, blank } => {
            let text = value.as_str().ok_or_else(|| format!("{title} takes text"))?;
            checked_text(setting, check, blank, text).map(Value::String)
        }
        Kind::Number { min, max, unit } => {
            let n = value.as_i64().ok_or_else(|| format!("{title} takes a whole number"))?;
            if (min..=max).contains(&n) { Ok(json!(n)) } else { Err(format!("{title} takes {min} to {max} {unit}").trim_end().to_string()) }
        }
    }
}

/// A word typed at a terminal as the option's value: on or off and their usual spellings, a choice's value, a field's
/// text, a number; checked as `checked` does.
pub fn parsed(setting: &Setting, word: &str) -> Result<Value, String> {
    let value = match setting.kind {
        Kind::Toggle => match word.to_lowercase().as_str() {
            "on" | "true" | "yes" | "y" | "1" => json!(true),
            "off" | "false" | "no" | "n" | "0" => json!(false),
            _ => return Err(format!("{word:?} is not on or off")),
        },
        Kind::Number { .. } => word.trim().parse::<i64>().map(|n| json!(n)).map_err(|_| format!("{} takes a whole number", setting.title))?,
        Kind::Choice(_) | Kind::Text { .. } => json!(word),
    };
    checked(setting, &value)
}

/// Where what was shown is kept: `{ran, seen, update}`.
fn state_path() -> PathBuf {
    files::cache().join("options.json")
}

/// What was shown: the version that ran last (`ran`), the options seen (`seen`), and the last update (`update`).
pub fn state() -> Value {
    files::load(&state_path()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

/// The release notes between two versions, newest first, from what the hub's check wrote in update.json.
fn notes_between(update: &Value, from: &str, to: &str) -> Value {
    let notes = update["notes"].as_array().into_iter().flatten();
    Value::Array(notes.filter(|n| n["version"].as_str().is_some_and(|v| is_newer(v, from) && !is_newer(v, to))).cloned().collect())
}

/// The state once `own` ran: a version later than the one noted last (update.json's, before anything was noted) is an
/// update, kept with its notes until it is shown.
pub fn after_run(state: &Value, update: &Value, own: &str, now: u64) -> Value {
    let mut next = state.clone();
    let last = state["ran"].as_str().or_else(|| state.get("ran").is_none().then(|| update["current"].as_str()).flatten());
    if let Some(last) = last.filter(|last| is_newer(own, last)) {
        next["update"] = json!({"from": last, "to": own, "at": now, "notes": notes_between(update, last, own), "shown": false});
    }
    next["ran"] = json!(own);
    next
}

/// Notes the version that runs, once per process start: the hub before its first look at what is published, and
/// `cockpit options`. Gives the state as it stands.
pub fn note_run(own: &str) -> Value {
    let path = state_path();
    files::locked(&path, || {
        let state = state();
        let next = after_run(&state, &files::load(&files::cache().join("update.json")).unwrap_or(Value::Null), own, crate::seen::now_ms());
        if next != state {
            let _ = files::write(&path, &next);
        }
        next
    })
}

/// The state with these options seen and, given its version, the last update shown.
pub fn marked(state: &Value, keys: &[&str], update: Option<&str>) -> Value {
    let mut next = state.clone();
    let mut seen: Vec<Value> = state["seen"].as_array().cloned().unwrap_or_default();
    for key in keys.iter().filter(|k| get(k).is_some()) {
        if !seen.iter().any(|s| s == key) {
            seen.push(json!(key));
        }
    }
    next["seen"] = Value::Array(seen);
    if let Some(to) = update
        && next["update"]["to"] == to
    {
        next["update"]["shown"] = json!(true);
    }
    next
}

/// Marks options as seen and, given its version, the last update as shown.
pub fn mark(keys: &[&str], update: Option<&str>) -> Result<(), String> {
    mark_in(&state_path(), keys, update)
}

fn mark_in(path: &Path, keys: &[&str], update: Option<&str>) -> Result<(), String> {
    files::locked(path, || {
        let state = files::load(path).filter(Value::is_object).unwrap_or_else(|| json!({}));
        files::write(path, &marked(&state, keys, update))
    })
    .map_err(|e| format!("options.json not written: {e}"))
}

/// The config's text with the option at `key` set, every other byte kept; a part of the key that holds something other
/// than an object is refused.
pub fn set_in(text: &str, key: &str, value: &Value) -> Result<String, String> {
    let parts: Vec<&str> = key.split('.').collect();
    let (name, path) = parts.split_last().ok_or("an empty key")?;
    jsonedit::set(text, path, name, value).map_err(|e| format!("{key} cannot be set in config.json ({e})"))
}

/// The value with the option at `key` set, the objects on its way made.
fn set_value(value: &mut Value, key: &str, to: &Value) {
    let parts: Vec<&str> = key.split('.').collect();
    let Some((name, path)) = parts.split_last() else { return };
    let mut node = value;
    for part in path {
        if !node[*part].is_object() {
            node[*part] = json!({});
        }
        node = &mut node[*part];
    }
    node[*name] = to.clone();
}

/// Whether saving this value takes the key out of config.json rather than writing it: the agent "the first
/// installed" is no agent's id, so the file says nothing.
fn removes(setting: &Setting, value: &Value) -> bool {
    matches!(setting.kind, Kind::Choice(Choices::Agents)) && value == ""
}

/// The config's text with the option at `key` taken out, every other byte kept.
fn unset_in(text: &str, key: &str) -> Result<String, String> {
    let parts: Vec<&str> = key.split('.').collect();
    let (name, path) = parts.split_last().ok_or("an empty key")?;
    Ok(jsonedit::remove(text, path, name).map_err(|e| format!("{key} cannot be set in config.json ({e})"))?.unwrap_or_else(|| text.to_string()))
}

/// The value without the option at `key`.
fn unset_value(value: &mut Value, key: &str) {
    let parts: Vec<&str> = key.split('.').collect();
    let Some((name, path)) = parts.split_last() else { return };
    let node = path.iter().try_fold(value, |node, part| node.get_mut(*part));
    if let Some(Value::Object(map)) = node {
        map.remove(*name);
    }
}

/// The value as the resolved copy holds it: a folder with its `~` spelled out, as the session that writes the copy does.
fn resolved_value(setting: &Setting, value: &Value) -> Value {
    match (setting.kind, value.as_str()) {
        (Kind::Text { check: Check::Folder, .. }, Some(path)) => json!(expanded(path).to_string_lossy()),
        _ => value.clone(),
    }
}

/// Sets options in ~/.config/cockpit/config.json, everything else kept as written, and in the resolved copy the apps
/// read until a session writes it again; they count as seen. A value its option does not take is refused, and then
/// nothing is written.
pub fn save(values: &[(&str, Value)]) -> Result<(), String> {
    save_in(values, &files::config_dir().join("config.json"), &files::cache().join("config.json"), &state_path())
}

fn save_in(values: &[(&str, Value)], path: &Path, resolved: &Path, state: &Path) -> Result<(), String> {
    let mut picked: Vec<(&'static Setting, Value)> = Vec::new();
    for (key, value) in values {
        let setting = get(key).ok_or_else(|| format!("no option {key:?}"))?;
        picked.push((setting, checked(setting, value)?));
    }
    files::locked(path, || {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("config.json not read: {e}")),
        };
        let mut next = text.clone();
        for (setting, value) in &picked {
            next = if removes(setting, value) { unset_in(&next, setting.key)? } else { set_in(&next, setting.key, value)? };
        }
        if next != text {
            write_private(path, &next).map_err(|e| format!("config.json not written: {e}"))?;
        }
        files::locked(resolved, || {
            if let Some(mut copy) = files::load(resolved).filter(Value::is_object) {
                for (setting, value) in &picked {
                    if removes(setting, value) {
                        unset_value(&mut copy, setting.key);
                    } else {
                        set_value(&mut copy, setting.key, &resolved_value(setting, value));
                    }
                }
                files::write(resolved, &copy).map_err(|e| format!("resolved config.json not written: {e}"))?;
            }
            Ok::<_, String>(())
        })?;
        mark_in(state, &picked.iter().map(|(setting, _)| setting.key).collect::<Vec<_>>(), None)
    })
}

/// The config written through a file only its owner reads, as it may hold tokens. A config.json that is a link stays
/// one: the file it points to is written. A write that fails leaves no temporary file behind.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let linked = path.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink());
    let target = if linked { std::fs::canonicalize(path)? } else { path.to_path_buf() };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = target.with_file_name(format!(".config.json.{}-{n}.tmp", std::process::id()));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, &target));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// One option as the views and `--json` read it: its section, kind, value and default, what a choice or a number
/// takes, whether it was seen and whether the last update brought it.
fn shown(config: &Value, setting: &Setting, seen: &[&str], from: Option<&str>) -> Value {
    let mut option = json!({
        "key": setting.key,
        "section": setting.section,
        "since": setting.since,
        "title": setting.title,
        "description": setting.description,
        "default": default_of(setting),
        "kind": setting.kind.name(),
        "value": value(config, setting),
        "by_hand": by_hand(config, setting),
        "seen": setting.since.is_none() || seen.contains(&setting.key),
        "new": setting.since.is_some_and(|since| from.is_some_and(|f| is_newer(since, f))),
    });
    match setting.kind {
        Kind::Toggle => {}
        Kind::Choice(_) => {
            let mut list = choices(setting);
            // A value written by hand that the list does not hold (an agent not installed) still shows as picked.
            if let Some(picked) = option["value"].as_str().filter(|picked| !list.iter().any(|(v, _)| v == picked)) {
                list.push((picked.to_string(), picked.to_string()));
            }
            option["choices"] = list.into_iter().map(|(value, label)| json!({"value": value, "label": label})).collect();
        }
        Kind::Text { blank, .. } => option["blank"] = json!(blank),
        Kind::Number { min, max, unit } => {
            option["min"] = json!(min);
            option["max"] = json!(max);
            option["unit"] = json!(unit);
        }
    }
    option
}

/// Every option section by section, as `shown` gives each; the sections in order; what config.json holds that no field
/// sets (`in_file`); the last update with its notes; and the layout it offers to try (`layout`).
pub fn view(config: &Value, state: &Value) -> Value {
    let seen: Vec<&str> = state["seen"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let from = state["update"]["from"].as_str();
    let options: Vec<Value> = ALL.iter().map(|s| shown(config, s, &seen, from)).collect();
    let in_file: Vec<Value> = IN_FILE.iter().map(|(key, line)| json!({"key": key, "description": line})).collect();
    let layout = from.filter(|f| is_newer(LAYOUT.1, f)).map(|_| LAYOUT.0);
    json!({
        "next": NEXT,
        "sections": SECTIONS,
        "options": options,
        "in_file": in_file,
        "path": files::config_dir().join("config.json"),
        "update": state.get("update").cloned().unwrap_or(Value::Null),
        "layout": layout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_option_is_listed_once_in_its_section_with_one_line_and_a_default_of_its_kind() {
        for (i, s) in ALL.iter().enumerate() {
            assert!(ALL.iter().skip(i + 1).all(|o| o.key != s.key), "{} twice", s.key);
            assert!(!s.title.is_empty() && s.description.ends_with('.') && !s.description.contains("  "), "{}", s.key);
            assert!(SECTIONS.contains(&s.section), "{} is in no section", s.key);
            assert!(fits(s, &default_of(s)), "{}'s default is not of its kind", s.key);
            // A default is a value the option takes, or the blank a field says it means.
            assert_eq!(checked(s, &default_of(s)).ok(), Some(default_of(s)), "{}", s.key);
        }
        // Sections follow one another in their order, each in one run.
        let order: Vec<usize> = ALL.iter().map(|s| SECTIONS.iter().position(|name| *name == s.section).unwrap()).collect();
        assert!(order.windows(2).all(|w| w[0] <= w[1]), "the options are listed section by section");
        assert!(SECTIONS.iter().all(|name| ALL.iter().any(|s| s.section == *name)), "no empty section");
        assert_eq!(since("0.40.3").iter().map(|s| s.key).collect::<Vec<_>>(), ["queue.drain", "ci.tellAgents", "chat.nextSteps", "sideBrowser"]);
        assert_eq!(since("1.0.1").iter().map(|s| s.key).collect::<Vec<_>>(), ["sideBrowser"]);
        assert_eq!(get("chat.nextSteps").map(|s| s.default), Some(Def::Off));
        assert!(get("nope").is_none());
        for (key, line) in IN_FILE {
            assert!(!key.is_empty() && line.ends_with('.'), "{key}");
        }
    }

    #[test]
    fn chromium_is_offered_only_where_it_runs() {
        let side = get("sideBrowser").unwrap();
        assert_eq!(offered(side, true).iter().map(|(v, _)| v.as_str()).collect::<Vec<_>>(), ["terminal-browser", CHROMIUM]);
        assert_eq!(offered(side, false).iter().map(|(v, _)| v.as_str()).collect::<Vec<_>>(), ["terminal-browser"]);
        assert_eq!(offered(get("browser.popups").unwrap(), false).len(), 2);
    }

    #[test]
    fn next_steps_say_who_writes_them_and_who_pays() {
        let line = get("chat.nextSteps").unwrap().description;
        assert!(line.contains("Claude Code writes them from the conversation's last part, billed to your Claude account"), "{line}");
        assert!(!line.contains("  "), "one line, no gaps from the source's wrapping: {line}");
    }

    #[test]
    fn versions_compare_number_by_number() {
        assert!(is_newer("0.41.0", "0.40.3") && is_newer("0.40.10", "0.40.9") && is_newer("1.0", "0.99.9") && is_newer("v0.41.1", "0.41"));
        assert!(!is_newer("0.40.3", "0.40.3") && !is_newer("0.40.3", "0.41.0") && !is_newer("0.41", "0.41.0"));
        assert_eq!(since("0.40.3").len(), ALL.iter().filter(|s| s.since.is_some()).count());
        assert!(since(NEXT).is_empty());
    }

    #[test]
    fn a_value_is_the_one_written_else_the_default() {
        let drain = get("queue.drain").unwrap();
        assert!(is_on(&json!({}), drain));
        assert!(!is_on(&json!({"queue": {"drain": false}}), drain));
        assert!(is_on(&json!({"queue": "odd"}), drain), "a value that is not a boolean reads as the default");
        assert!(!is_on(&Value::Null, get("chat.nextSteps").unwrap()));
        let pace = get("agents.pace").unwrap();
        assert_eq!(value(&json!({}), pace), "week");
        assert_eq!(value(&json!({"agents": {"pace": "weekdays"}}), pace), "weekdays");
        let every = get("refresh.github").unwrap();
        assert_eq!((value(&json!({}), every), value(&json!({"refresh": {"github": 45}}), every)), (json!(30), json!(45)));
        assert_eq!(value(&json!({"refresh": {"github": "often"}}), every), json!(30));
        // A model per agent is no field's text: it reads as the default and says it is set by hand.
        let model = get("chat.nextStepsModel").unwrap();
        let per_agent = json!({"chat": {"nextStepsModel": {"claude": "haiku"}}});
        assert_eq!((value(&per_agent, model), by_hand(&per_agent, model)), (json!(""), true));
        assert!(!by_hand(&json!({}), model) && !by_hand(&json!({"chat": {"nextStepsModel": "haiku"}}), model));
        // A wrong scalar is no setting by hand: it reads as the default, which a save replaces.
        assert!(!by_hand(&json!({"chat": {"nextStepsModel": 7}}), model) && !by_hand(&json!({"refresh": {"github": "often"}}), every));
    }

    #[test]
    fn a_switch_takes_a_boolean_and_a_choice_one_of_its_values() {
        let drain = get("queue.drain").unwrap();
        assert_eq!(checked(drain, &json!(false)), Ok(json!(false)));
        assert_eq!(checked(drain, &json!("off")).unwrap_err(), "Send queued prompts on their own is on or off");
        let pace = get("agents.pace").unwrap();
        assert_eq!(checked(pace, &json!("weekdays")), Ok(json!("weekdays")));
        assert_eq!(checked(pace, &json!("monthly")).unwrap_err(), "Pace of the 7-day limit takes one of: week, weekdays");
        assert!(checked(pace, &json!(true)).is_err());
        // The agent cockpit starts: the first installed, or an agent of the table, installed or not.
        let agent = get("agents.default").unwrap();
        assert_eq!(checked(agent, &json!("")), Ok(json!("")));
        assert_eq!(checked(agent, &json!("codex")), Ok(json!("codex")));
        assert!(checked(agent, &json!("nobody")).unwrap_err().starts_with("The agent cockpit starts takes one of: \"\""));
        assert_eq!(choices(agent)[0], (String::new(), "The first installed".to_string()));
    }

    #[test]
    fn a_field_takes_what_its_check_allows_and_says_why_not_in_its_own_words() {
        let at = get("brief.at").unwrap();
        assert_eq!(checked(at, &json!(" 8:05 ")), Ok(json!("08:05")));
        for refused in ["25:00", "08:60", "8h30", "morning", "08:30:00"] {
            assert_eq!(checked(at, &json!(refused)).unwrap_err(), "Morning brief time takes a time of day, such as 08:30", "{refused}");
        }
        assert_eq!(checked(at, &json!("")).unwrap_err(), "Morning brief time cannot be empty");
        let branch = get("start.branch").unwrap();
        assert_eq!(checked(branch, &json!("{prefix}/{key}-{slug}")), Ok(json!("{prefix}/{key}-{slug}")));
        assert_eq!(checked(branch, &json!("")), Ok(json!("")), "empty is the default template");
        assert!(checked(branch, &json!("my branch/{slug}")).unwrap_err().contains("takes letters, digits"));
        assert_eq!(checked(branch, &json!("work")).unwrap_err(), "Branch name needs {key} or {slug}, so two branches differ");
        let org = get("github.org").unwrap();
        assert_eq!(checked(org, &json!("acme")), Ok(json!("acme")));
        assert_eq!(checked(org, &json!("acme inc")).unwrap_err(), "GitHub organization takes one word, with no space");
        // The editor is one command, as Open in editor runs it: no switches.
        let editor = get("editor").unwrap();
        assert_eq!(checked(editor, &json!("cursor")), Ok(json!("cursor")));
        assert_eq!(checked(editor, &json!("code --wait")).unwrap_err(), "Editor takes one word, with no space");
        assert_eq!(checked(editor, &json!("--wait")).unwrap_err(), "Editor cannot start with a dash");
        assert_eq!(checked(editor, &json!("a\nb")).unwrap_err(), "Editor takes one short line");
        assert_eq!(checked(editor, &json!("x".repeat(201))).unwrap_err(), "Editor takes one short line");
        assert!(checked(editor, &json!(7)).unwrap_err().contains("takes text"));
        let repos = get("reposDir").unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().to_string_lossy().to_string();
        assert_eq!(checked(repos, &json!(path)), Ok(json!(path)));
        assert_eq!(checked(repos, &json!(format!("{path}/nowhere"))).unwrap_err(), format!("{path}/nowhere is not a folder"));
        for relative in ["code", "./code", "~me/code", "../"] {
            assert_eq!(
                checked(repos, &json!(relative)).unwrap_err(),
                "Folder of your checkouts is written from the root (/…) or from your home folder (~/…)",
                "{relative}"
            );
        }
        assert_eq!(checked(repos, &json!("~")), Ok(json!("~")));
        assert_eq!(resolved_value(repos, &json!("~/code")), json!(files::home().join("code").to_string_lossy()));
    }

    #[test]
    fn a_number_stays_within_its_bounds() {
        let every = get("refresh.github").unwrap();
        assert_eq!(checked(every, &json!(15)), Ok(json!(15)));
        for refused in [json!(14), json!(86_401), json!(-1)] {
            assert_eq!(checked(every, &refused).unwrap_err(), "GitHub takes 15 to 86400 s");
        }
        assert_eq!(checked(every, &json!(30.5)).unwrap_err(), "GitHub takes a whole number");
        assert_eq!(checked(every, &json!("30")).unwrap_err(), "GitHub takes a whole number");
        assert_eq!(checked(get("browserIdleMinutes").unwrap(), &json!(0)), Ok(json!(0)), "0 keeps the browser");
    }

    #[test]
    fn a_word_typed_at_a_terminal_reads_as_the_option_s_kind() {
        assert_eq!(parsed(get("queue.drain").unwrap(), "Off"), Ok(json!(false)));
        assert_eq!(parsed(get("queue.drain").unwrap(), "maybe").unwrap_err(), "\"maybe\" is not on or off");
        assert_eq!(parsed(get("refresh.ci").unwrap(), "45"), Ok(json!(45)));
        assert!(parsed(get("refresh.ci").unwrap(), "soon").unwrap_err().contains("whole number"));
        assert_eq!(parsed(get("density").unwrap(), "compact"), Ok(json!("compact")));
        assert!(parsed(get("density").unwrap(), "tiny").unwrap_err().contains("takes one of: comfortable, compact"));
        assert_eq!(parsed(get("brief.at").unwrap(), "9:00"), Ok(json!("09:00")));
    }

    #[test]
    fn a_value_reads_in_words() {
        assert_eq!(words(get("queue.drain").unwrap(), &json!(true)), "on");
        assert_eq!(words(get("agents.pace").unwrap(), &json!("weekdays")), "Weekdays");
        assert_eq!(words(get("refresh.github").unwrap(), &json!(45)), "45 s");
        assert_eq!(words(get("start.branch").unwrap(), &json!("")), "{prefix}/{key}-{slug}");
        assert_eq!(words(get("editor").unwrap(), &json!("zed")), "zed");
    }

    #[test]
    fn a_later_version_than_the_last_noted_is_an_update_with_its_notes() {
        let published = json!({"current": "0.39.0", "notes": [
            {"version": "0.41.1", "changes": ["later"]},
            {"version": "0.41.0", "changes": ["new"]},
            {"version": "0.40.0", "changes": ["older"]},
            {"version": "0.39.0", "changes": ["running"]},
        ]});
        let noted = after_run(&json!({"ran": "0.40.0", "seen": ["queue.drain"]}), &published, "0.41.0", 7);
        assert_eq!(noted["ran"], "0.41.0");
        assert_eq!(noted["seen"], json!(["queue.drain"]));
        assert_eq!(noted["update"], json!({"from": "0.40.0", "to": "0.41.0", "at": 7, "notes": [{"version": "0.41.0", "changes": ["new"]}], "shown": false}));
        // The same version again changes nothing; an older one is no update.
        assert_eq!(after_run(&noted, &published, "0.41.0", 9), noted);
        assert_eq!(after_run(&noted, &published, "0.40.0", 9)["update"], noted["update"]);
        // Before anything was noted, the version update.json was written by is the one that ran.
        let first = after_run(&json!({}), &published, "0.41.0", 7);
        assert_eq!((first["update"]["from"].as_str(), first["update"]["notes"].as_array().map(Vec::len)), (Some("0.39.0"), Some(2)));
        // A first install has nothing to say.
        assert!(after_run(&json!({}), &Value::Null, "0.41.0", 7).get("update").is_none());
    }

    #[test]
    fn marking_notes_the_options_seen_and_the_update_shown_once() {
        let state = json!({"ran": "0.41.0", "update": {"from": "0.40.0", "to": "0.41.0", "shown": false}});
        let marked = marked(&state, &["queue.drain", "made.up", "queue.drain"], Some("0.41.0"));
        assert_eq!(marked["seen"], json!(["queue.drain"]));
        assert_eq!(marked["update"]["shown"], true);
        assert_eq!(super::marked(&state, &[], Some("0.40.9"))["update"]["shown"], false, "another version's update stays to show");
    }

    #[test]
    fn setting_an_option_keeps_the_rest_of_the_file_as_written() {
        let text = "{\n  \"reposDir\": \"~/code\",\n  \"queue\": {\"drain\": true},\n  \"ci\": {\n    \"enabled\": true\n  }\n}\n";
        let done = set_in(&set_in(text, "queue.drain", &json!(false)).unwrap(), "ci.tellAgents", &json!(false)).unwrap();
        assert_eq!(
            done,
            "{\n  \"reposDir\": \"~/code\",\n  \"queue\": {\"drain\": false},\n  \"ci\": {\n    \"enabled\": true,\n    \"tellAgents\": false\n  }\n}\n"
        );
        assert_eq!(set_in("", "chat.nextSteps", &json!(true)).unwrap(), "{\n  \"chat\": {\n    \"nextSteps\": true\n  }\n}\n");
        let typed = set_in(&set_in(text, "refresh.github", &json!(45)).unwrap(), "brief.at", &json!("09:00")).unwrap();
        assert!(typed.starts_with("{\n  \"reposDir\": \"~/code\",\n  \"queue\": {\"drain\": true},\n  \"ci\": {\n    \"enabled\": true\n  },\n"), "{typed}");
        assert_eq!(jsonedit::plain(&typed).unwrap()["refresh"]["github"], 45);
        assert_eq!(jsonedit::plain(&typed).unwrap()["brief"]["at"], "09:00");
        let refused = set_in("{\"chat\": false}", "chat.nextSteps", &json!(true)).unwrap_err();
        assert!(refused.contains("chat.nextSteps cannot be set"), "{refused}");
        let mut copy = json!({"chat": false, "queue": {"drain": true}});
        set_value(&mut copy, "chat.nextSteps", &json!(true));
        set_value(&mut copy, "queue.drain", &json!(false));
        assert_eq!(copy, json!({"chat": {"nextSteps": true}, "queue": {"drain": false}}));
    }

    #[test]
    fn simultaneous_saves_keep_each_option_in_both_configs() {
        for _ in 0..12 {
            let root = tempfile::tempdir().unwrap();
            let raw = root.path().join("raw/config.json");
            let resolved = root.path().join("resolved/config.json");
            let state = root.path().join("options.json");
            let initial = json!({"queue": {"drain": true}, "ci": {"tellAgents": true}, "chat": {"nextSteps": false}, "repo": "acme/api"});
            files::write(&raw, &initial).unwrap();
            files::write(&resolved, &initial).unwrap();
            let ready = std::sync::Barrier::new(3);
            std::thread::scope(|threads| {
                for (key, on) in [("queue.drain", false), ("ci.tellAgents", false), ("chat.nextSteps", true)] {
                    let (raw, resolved, state, ready) = (&raw, &resolved, &state, &ready);
                    threads.spawn(move || {
                        ready.wait();
                        save_in(&[(key, json!(on))], raw, resolved, state).unwrap();
                    });
                }
            });
            let expected = json!({"queue": {"drain": false}, "ci": {"tellAgents": false}, "chat": {"nextSteps": true}, "repo": "acme/api"});
            assert_eq!(files::load(&raw), Some(expected.clone()));
            assert_eq!(files::load(&resolved), Some(expected));
            assert_eq!(files::load(&state).unwrap()["seen"].as_array().unwrap().len(), 3);
        }
    }

    #[test]
    fn a_save_writes_each_kind_keeps_unknown_keys_and_only_its_owner_reads_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let (raw, resolved, state) = (root.path().join("config.json"), root.path().join("cache/config.json"), root.path().join("options.json"));
        let written = "{\n  // mine\n  \"tickets\": {\"token\": \"lin_api_x\", \"made\": {\"up\": [1, 2]}},\n  \"unknown\": {\"kept\": true}\n}\n";
        std::fs::write(&raw, written).unwrap();
        files::write(&resolved, &json!({"reposDir": "/old", "unknown": {"kept": true}})).unwrap();
        let repos = tempfile::tempdir().unwrap();
        let folder = repos.path().to_string_lossy().to_string();
        let values = [
            ("refresh.github", json!(45)),
            ("agents.pace", json!("weekdays")),
            ("brief.at", json!("9:00")),
            ("names", json!(false)),
            ("reposDir", json!(folder)),
        ];
        save_in(&values, &raw, &resolved, &state).unwrap();
        let text = std::fs::read_to_string(&raw).unwrap();
        assert!(
            text.starts_with("{\n  // mine\n  \"tickets\": {\"token\": \"lin_api_x\", \"made\": {\"up\": [1, 2]}},\n  \"unknown\": {\"kept\": true},\n"),
            "{text}"
        );
        let saved = jsonedit::plain(&text).unwrap();
        assert_eq!(
            (&saved["refresh"]["github"], &saved["agents"]["pace"], &saved["brief"]["at"], &saved["names"]),
            (&json!(45), &json!("weekdays"), &json!("09:00"), &json!(false))
        );
        assert_eq!(std::fs::metadata(&raw).unwrap().permissions().mode() & 0o777, 0o600);
        let copy = files::load(&resolved).unwrap();
        assert_eq!((&copy["refresh"]["github"], &copy["unknown"]["kept"], &copy["reposDir"]), (&json!(45), &json!(true), &json!(folder)));
    }

    #[test]
    fn the_first_installed_agent_takes_the_key_out_of_the_file() {
        let root = tempfile::tempdir().unwrap();
        let (raw, resolved, state) = (root.path().join("config.json"), root.path().join("resolved.json"), root.path().join("options.json"));
        std::fs::write(&raw, "{\n  \"agents\": {\"default\": \"codex\", \"pace\": \"week\"}\n}\n").unwrap();
        files::write(&resolved, &json!({"agents": {"default": "codex", "pace": "week"}})).unwrap();
        save_in(&[("agents.default", json!(""))], &raw, &resolved, &state).unwrap();
        assert_eq!(std::fs::read_to_string(&raw).unwrap(), "{\n  \"agents\": {\"pace\": \"week\"}\n}\n");
        assert_eq!(files::load(&resolved), Some(json!({"agents": {"pace": "week"}})));
        // Nothing to take out is no change.
        save_in(&[("agents.default", json!(""))], &raw, &resolved, &state).unwrap();
        assert_eq!(std::fs::read_to_string(&raw).unwrap(), "{\n  \"agents\": {\"pace\": \"week\"}\n}\n");
        assert_eq!(value(&json!({}), get("agents.default").unwrap()), "");
    }

    #[test]
    fn a_linked_config_stays_a_link_and_a_failed_write_leaves_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("dotfiles/cockpit.json");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "{\"names\": true}\n").unwrap();
        let link = root.path().join("config/config.json");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        save_in(&[("names", json!(false))], &link, &root.path().join("resolved.json"), &root.path().join("options.json")).unwrap();
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink(), "still a link");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "{\"names\": false}\n");
        // The file's folder cannot take the new file: the write fails and nothing is left in it.
        let dir = real.parent().unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let failed = write_private(&real, "{}");
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(failed.is_err());
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 1, "no temporary file");
    }

    #[test]
    fn a_refused_value_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let (raw, resolved, state) = (root.path().join("config.json"), root.path().join("resolved.json"), root.path().join("options.json"));
        std::fs::write(&raw, "{\"names\": true}\n").unwrap();
        // One value its option refuses stops the whole save, the good ones with it.
        let refused = save_in(&[("names", json!(false)), ("refresh.github", json!(3))], &raw, &resolved, &state).unwrap_err();
        assert_eq!(refused, "GitHub takes 15 to 86400 s");
        assert!(save_in(&[("names", json!("no"))], &raw, &resolved, &state).unwrap_err().contains("is on or off"));
        assert_eq!(save_in(&[("made.up", json!(true))], &raw, &resolved, &state).unwrap_err(), "no option \"made.up\"");
        assert_eq!(std::fs::read_to_string(&raw).unwrap(), "{\"names\": true}\n");
        assert!(!state.exists(), "nothing counts as seen");
    }

    #[test]
    fn an_unreadable_config_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        let raw = root.path().join("config.json");
        std::fs::write(&raw, [0xff]).unwrap();
        let result = save_in(&[("chat.nextSteps", json!(true))], &raw, &root.path().join("resolved.json"), &root.path().join("options.json"));
        assert!(result.unwrap_err().contains("config.json not read"));
        assert_eq!(std::fs::read(raw).unwrap(), [0xff]);
    }

    #[test]
    fn the_view_says_what_the_last_update_brought() {
        let state = json!({"seen": ["ci.tellAgents"], "update": {"from": "0.40.3", "to": NEXT, "notes": [], "shown": false}});
        let view = view(&json!({"chat": {"nextSteps": true}}), &state);
        let options = view["options"].as_array().unwrap();
        assert_eq!(options.len(), ALL.len());
        let option = |key: &str| options.iter().find(|o| o["key"] == key).unwrap();
        let next = option("chat.nextSteps");
        assert_eq!(
            (next["value"].as_bool(), next["default"].as_bool(), next["new"].as_bool(), next["seen"].as_bool()),
            (Some(true), Some(false), Some(true), Some(false))
        );
        assert_eq!(option("ci.tellAgents")["seen"], true);
        assert_eq!((view["layout"].as_str(), view["update"]["to"].as_str()), (Some(LAYOUT.0), Some(NEXT)));
        let later = super::view(&json!({}), &json!({"update": {"from": NEXT, "to": "9.0.0"}}));
        assert!(
            later["layout"].is_null() && later["options"].as_array().unwrap().iter().all(|o| o["new"] == false),
            "an update past the layout's version offers nothing it had"
        );
        assert!(super::view(&json!({}), &json!({}))["update"].is_null());
    }

    #[test]
    fn the_view_lists_each_option_with_its_section_kind_and_what_it_takes() {
        let view = view(&json!({"agents": {"default": "made-up-agent"}, "meetingBrief": {"model": {"claude": "haiku"}}}), &json!({}));
        assert_eq!(view["sections"], json!(SECTIONS));
        let option = |key: &str| view["options"].as_array().unwrap().iter().find(|o| o["key"] == key).unwrap().clone();
        // An option cockpit always read is neither new nor unseen, and names no version.
        let names = option("names");
        assert_eq!(
            (&names["section"], &names["kind"], &names["since"], &names["seen"], &names["new"]),
            (&json!("Agents"), &json!("toggle"), &Value::Null, &json!(true), &json!(false))
        );
        assert_eq!((&option("queue.drain")["since"], &option("sideBrowser")["since"]), (&json!("1.0.0"), &json!(NEXT)));
        let pace = option("agents.pace");
        assert_eq!(pace["choices"], json!([{"value": "week", "label": "Whole week"}, {"value": "weekdays", "label": "Weekdays"}]));
        let every = option("refresh.github");
        assert_eq!(
            (&every["kind"], &every["min"], &every["max"], &every["unit"], &every["default"]),
            (&json!("number"), &json!(15), &json!(86_400), &json!("s"), &json!(30))
        );
        assert_eq!(option("start.branch")["blank"], "{prefix}/{key}-{slug}");
        assert!(option("brief.at")["blank"].is_null());
        // An agent written by hand that is not installed still shows as the one picked.
        let agent = option("agents.default");
        assert_eq!(agent["value"], "made-up-agent");
        assert!(agent["choices"].as_array().unwrap().iter().any(|c| c["value"] == "made-up-agent"));
        assert_eq!(option("meetingBrief.model")["by_hand"], true);
        let in_file = view["in_file"].as_array().unwrap();
        assert!(in_file.iter().any(|f| f["key"] == "agents.commands") && in_file.iter().any(|f| f["key"] == "usage.commands"));
        assert!(view["path"].as_str().unwrap().ends_with(".config/cockpit/config.json"));
    }
}
