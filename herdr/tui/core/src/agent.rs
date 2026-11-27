//! The coding agents cockpit starts, resumes, forks and runs headless, one entry per agent Herdr integrates, with argv
//! templates the config's `agents.commands` overrides. A template argument whose `{prompt}`, `{session}` or `{model}`
//! has no value is dropped whole, so `start(None)` opens the agent at its prompt and `"-i {prompt}"` goes as one.

use std::path::Path;

use serde_json::Value;

use crate::files;

/// How an agent's past sessions can be read, for search, recaps, costs and context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Transcripts {
    #[default]
    None,
    /// `~/.claude/projects/<cwd>/<session>.jsonl`
    Claude,
    /// `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`
    Codex,
    /// `~/.pi/agent/sessions/--<cwd>--/<time>_<session>.jsonl`
    Pi,
    /// oh-my-pi: pi's format under `~/.omp/agent/sessions`
    Omp,
    /// `~/.qwen/projects/<cwd>/chats/<session>.jsonl`
    Qwen,
    /// `~/.copilot/session-state/<session>/events.jsonl`
    Copilot,
    /// `~/.gemini/tmp/<project>/chats/session-<time>-<session start>.jsonl`
    Gemini,
}

/// Where an agent's plan is read whole while it asks to carry it out: the argument of the tool it asks through, the
/// Markdown file that argument names, or its last message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// The call's `arg` holds the plan as Markdown.
    Argument { tool: &'static str, arg: &'static str },
    /// The call's `arg` is the path of the Markdown file holding the plan.
    File { tool: &'static str, arg: &'static str },
    /// The plan is its last message, written in its plan mode.
    LastMessage,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    /// Herdr's integration id, also its `program` name (`claude`, `codex`, `opencode`).
    pub id: String,
    /// What people read: "Claude Code", "Codex".
    pub name: String,
    /// The executable; resolved by `path()`.
    pub bin: String,
    /// Other program names Herdr may report for it.
    pub aliases: Vec<String>,
    /// Interactive, optionally with a first message.
    pub start: Vec<String>,
    /// Non-interactive: prints its answer and exits. Empty when the agent has none.
    pub run: Vec<String>,
    /// Appended to `run` (and `start`) when a model is given.
    pub model: Vec<String>,
    /// Interactive, on a past session. Empty when unsupported.
    pub resume: Vec<String>,
    /// A new session branched from a past one. Empty when unsupported.
    pub fork: Vec<String>,
    pub transcripts: Transcripts,
    /// Where its plan is read whole while it asks to carry one out. None where it has no plan mode, or none measured
    /// or documented.
    pub plan: Option<Plan>,
    /// Non-interactive, on a new branch of a past session. Empty when unsupported.
    pub run_fork: Vec<String>,
    /// Lets an unattended run edit files in its folder without asking. Empty when it has no such switch.
    pub edits: Vec<String>,
    /// One more folder an unattended run may write to, `{dir}`. Empty when it has no such switch.
    pub add_dir: Vec<String>,
    /// Adds `{text}` to its system prompt. Empty when it has none: the text then goes before the prompt.
    pub system: Vec<String>,
    /// Model names, or prefixes, that are its own: a model another agent claims is never passed to it.
    pub models: Vec<String>,
    /// Words its own error holds when nobody is signed in to it (config: `signedOut`). Empty when not known.
    pub signed_out: Vec<String>,
    /// How a permission mode is passed at start, `{mode}` in Claude Code's words (default, acceptEdits, plan, …). None
    /// where it was not measured: what needs it is not offered.
    pub mode: Option<Vec<String>>,
    /// The keys that stop a turn in its own terminal, as hc and herdr name them ("esc"). None where not measured.
    pub interrupt: Option<Vec<String>>,
    /// The keys that empty its input line ("ctrl+u"). None where not measured: ctrl+u is pressed then.
    pub clear: Option<Vec<String>>,
    /// How its permission prompts read: patterns (regex) of the text above a menu that asks before a tool runs, edits
    /// go in, a plan is carried out or a folder is trusted. Empty where not measured.
    pub permission: Vec<String>,
    /// The switches that drive it over its streaming protocol, permission asks on the same pipe. None where not
    /// measured: cockpit drives no session of it.
    pub stream: Option<Vec<String>>,
    /// How a new driven session takes the id cockpit picks for it, `{session}`.
    pub new_session: Vec<String>,
    /// The models its pickers offer, as it takes them (aliases or full names). Empty where none is listed.
    pub offered: Vec<String>,
    /// The effort levels it takes, lowest first. Empty where it has none or they were not measured.
    pub efforts: Vec<String>,
    /// Its lightest model, which errands such as next steps run on. None where not measured.
    pub light: Option<String>,
    /// The keys that move it to its next permission mode in its terminal (Claude Code's ⇧⇥). None where not measured.
    pub cycle: Option<Vec<String>>,
    /// What typed in its terminal turns its focus view on or off (Claude Code's `/focus`). None where it has none.
    pub focus: Option<String>,
    /// How an effort level is passed at start, `{effort}`. None where not measured.
    pub effort: Option<Vec<String>>,
    /// How fast mode is set at start: `{fast}` as true or false, `{speed}` as fast or default. Switches without a value
    /// turn it on and are left out for the agent's default.
    pub fast: Option<Vec<String>>,
    /// The settings a session cockpit drives takes while it runs; the others take effect at its next message.
    pub live: Vec<Setting>,
    /// How each setting changes in its own terminal while it runs.
    pub typing: Typing,
    /// The tools that start a subagent, a background job or a workflow, where measured. None where it has none known.
    pub tasks: Option<Tasks>,
    /// The environment variable that names the config folder it runs on, which picks its account (Claude Code's
    /// `CLAUDE_CONFIG_DIR`). None where it has none known.
    pub account_env: Option<String>,
}

/// The tools of an agent that start work beside its turn: a subagent, a background job, a workflow. Each list names
/// the tools as the transcript spells them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tasks {
    pub spawn: Vec<String>,
    /// Background only when the call's input says so (`run_in_background: true`).
    pub background: Vec<String>,
    pub workflow: Vec<String>,
}

/// What a person picks beside a prompt: the model, the effort level, fast mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Setting {
    Model,
    Effort,
    Fast,
}

impl Setting {
    pub const ALL: [Setting; 3] = [Setting::Model, Setting::Effort, Setting::Fast];

    /// Its name in acts and views: model, effort, fast.
    pub fn id(self) -> &'static str {
        match self {
            Setting::Model => "model",
            Setting::Effort => "effort",
            Setting::Fast => "fast",
        }
    }

    pub fn of(id: &str) -> Option<Setting> {
        Setting::ALL.into_iter().find(|s| s.id() == id)
    }
}

/// How a running agent takes a new setting in its own terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Typed {
    /// A command typed as its next prompt, the value filled in: `{model}`, `{effort}`, or `{fast}` as on or off.
    Command(String),
    /// Changed only by hand in its terminal, through this command's own picker or toggle.
    ByHand(String),
}

/// How each setting changes in an agent's own terminal while it runs; None where it cannot or it was not measured.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Typing {
    pub model: Option<Typed>,
    pub effort: Option<Typed>,
    pub fast: Option<Typed>,
}

impl Typing {
    pub fn of(&self, setting: Setting) -> Option<&Typed> {
        match setting {
            Setting::Model => self.model.as_ref(),
            Setting::Effort => self.effort.as_ref(),
            Setting::Fast => self.fast.as_ref(),
        }
    }
}

/// One model a picker offers: what the agent takes, what people read, and what it supports where its catalog says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Offer {
    pub id: String,
    pub label: String,
    /// Its own effort levels, where they differ from the agent's.
    pub efforts: Option<Vec<String>>,
    /// Whether it has a fast mode, where its catalog says.
    pub fast: Option<bool>,
    /// The model an alias stands for, where its catalog says ("opus" is "claude-opus-5-5").
    pub resolved: Option<String>,
    /// One line on what it is for, where its catalog says.
    pub description: Option<String>,
}

impl Offer {
    /// A model known by its id alone, named as people read it ("opus" is "Opus").
    pub fn named(id: String) -> Offer {
        Offer { label: label_of(&id), id, ..Offer::default() }
    }

    /// An offer as `json` writes it, or a bare id; None for an id no agent could take as one word (`model_id`).
    pub fn of_json(v: &Value) -> Option<Offer> {
        if let Some(id) = v.as_str() {
            return model_id(id).then(|| Offer::named(id.to_string()));
        }
        let id = v["id"].as_str().filter(|id| model_id(id))?.to_string();
        let label = shown_label(v["label"].as_str(), &id);
        let efforts = v["efforts"].as_array().map(|l| l.iter().filter_map(|e| e.as_str().map(str::to_string)).collect());
        let resolved = v["resolved"].as_str().filter(|r| model_id(r)).map(str::to_string);
        let description = v["description"].as_str().map(|d| crate::text::clean(&crate::text::without_sequences(d), 120)).filter(|d| !d.trim().is_empty());
        Some(Offer { id, label, efforts, fast: v["fast"].as_bool(), resolved, description })
    }

    pub fn json(&self) -> Value {
        let mut out = serde_json::json!({"id": self.id, "label": self.label});
        if let Some(efforts) = &self.efforts {
            out["efforts"] = serde_json::json!(efforts);
        }
        if let Some(fast) = self.fast {
            out["fast"] = serde_json::json!(fast);
        }
        if let Some(resolved) = &self.resolved {
            out["resolved"] = serde_json::json!(resolved);
        }
        if let Some(description) = &self.description {
            out["description"] = serde_json::json!(description);
        }
        out
    }
}

/// What a person picked for a new agent beside its prompt; None leaves each to the agent.
#[derive(Clone, Debug, Default)]
pub struct Picked<'a> {
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub fast: Option<bool>,
}

/// What a headless run gets beyond its prompt.
#[derive(Clone, Debug, Default)]
pub struct Headless<'a> {
    pub model: Option<&'a str>,
    /// An effort level, passed where the agent takes it.
    pub effort: Option<&'a str>,
    /// A past session to branch from.
    pub fork: Option<&'a str>,
    /// May edit files in its folder, and in `dirs`, without asking.
    pub edits: bool,
    pub dirs: Vec<String>,
    /// Instructions beside the prompt.
    pub system: Option<&'a str>,
    /// Kept aside: out of the agent's history, hooks, plugins and MCP servers, acting on nothing (its `aside` switches).
    pub aside: bool,
}

fn v(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// One row of the table: the argv templates, then what an unattended run may get.
struct Row<'a> {
    id: &'a str,
    name: &'a str,
    bin: &'a str,
    aliases: &'a [&'a str],
    start: &'a [&'a str],
    run: &'a [&'a str],
    model: &'a [&'a str],
    resume: &'a [&'a str],
    fork: &'a [&'a str],
    run_fork: &'a [&'a str],
    edits: &'a [&'a str],
    add_dir: &'a [&'a str],
    system: &'a [&'a str],
    models: &'a [&'a str],
    transcripts: Transcripts,
}

impl Row<'_> {
    fn agent(&self) -> Agent {
        Agent {
            id: self.id.into(),
            name: self.name.into(),
            bin: self.bin.into(),
            aliases: v(self.aliases),
            start: v(self.start),
            run: v(self.run),
            model: v(self.model),
            resume: v(self.resume),
            fork: v(self.fork),
            transcripts: self.transcripts,
            plan: None,
            run_fork: v(self.run_fork),
            edits: v(self.edits),
            add_dir: v(self.add_dir),
            system: v(self.system),
            models: v(self.models),
            signed_out: v(signed_out(self.id)),
            mode: None,
            interrupt: None,
            clear: None,
            permission: vec![],
            stream: None,
            new_session: vec![],
            offered: vec![],
            efforts: vec![],
            light: None,
            cycle: None,
            focus: None,
            effort: None,
            fast: None,
            live: vec![],
            typing: Typing::default(),
            tasks: None,
            account_env: None,
        }
    }
}

/// What an agent prints when nobody is signed in to it, where it is known.
fn signed_out(id: &str) -> &'static [&'static str] {
    match id {
        "claude" => &["not logged in", "/login", "invalid api key"],
        _ => NONE,
    }
}

const NONE: &[&str] = &[];

/// An agent's id and three lists of words measured for it.
type Lists<'a> = (&'a str, &'a [&'a str], &'a [&'a str], &'a [&'a str]);

/// What was measured of an agent beyond its argv: (id, how a permission mode is passed at start, the keys that stop a
/// turn in its terminal, the keys that empty its input line). An empty list is not measured, and an agent not listed
/// has none of them.
type ControlRow = (&'static str, &'static [&'static str], &'static [&'static str], &'static [&'static str]);
const CONTROLS: &[ControlRow] = &[
    // Claude Code 2.1.311: `--permission-mode` (driven sessions start with it); esc is `chat:cancel` in its keymap,
    // its "esc to interrupt", while ctrl+c twice quits; ctrl+u empties the line ("Ctrl+Y to paste deleted text").
    ("claude", &["--permission-mode", "{mode}"], &["esc"], &["ctrl+u"]),
    // Codex 0.162: esc ends the turn ("Conversation interrupted"); its sandbox and approval switches are not one mode;
    // ctrl+u is the readline clear its composer documents.
    ("codex", NONE, &["esc"], &["ctrl+u"]),
];

/// How an agent is driven over its streaming protocol, as measured: (id, the switches that open the stream, how a new
/// session takes its id). An agent not listed is not driven.
const STREAM: &[(&str, &[&str], &[&str])] = &[
    // Claude Code 2.1.322: stream-json both ways, each prompt replayed, permission asks as control requests on stdout;
    // `--session-id` names a new session and `--resume` (its `resume` column) takes it up again under the same id.
    (
        "claude",
        &["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--replay-user-messages", "--permission-prompt-tool", "stdio"],
        &["--session-id", "{session}"],
    ),
];

/// Where each agent's plan is read while it asks to carry one out.
const PLANS: &[(&str, Plan)] = &[
    // Claude Code 2.1.330 (measured): ExitPlanMode's `plan`, the screen scrolling the plan in a box.
    ("claude", Plan::Argument { tool: "ExitPlanMode", arg: "plan" }),
    // Codex 0.96 (developers.openai.com /plan): the plan is the last message of a turn in plan collaboration mode.
    ("codex", Plan::LastMessage),
    // Qwen Code (qwenlm.github.io exit-plan-mode): `exit_plan_mode(plan=…)` presents the plan for approval.
    ("qwen", Plan::Argument { tool: "exit_plan_mode", arg: "plan" }),
    // Gemini CLI (geminicli.com tools/planning): `exit_plan_mode(plan_path=…)` names the plan file under its plans folder.
    ("gemini", Plan::File { tool: "exit_plan_mode", arg: "plan_path" }),
];

/// How each agent's permission prompts read, as measured: (id, patterns of the text above the menu).
const PERMISSION: &[(&str, &[&str])] = &[
    // Claude Code 2.1.322: a tool (Bash, Edit, Write, WebFetch, MCP), a plan carried out, a folder trusted.
    (
        "claude",
        &[
            r"^Do you want to (proceed|continue|make this edit|create|overwrite|delete|allow|use|run)\b",
            r"^Would you like to proceed\?",
            r"Is this a project you created or one you trust\?",
            r"^Do you trust the files in this folder\?",
        ],
    ),
    // Codex 0.162: a command, edits, permissions, input to a running terminal.
    ("codex", &[r"^Would you like to (run the following command|make the following edits|grant these permissions|send input to the existing terminal)\?"]),
];

/// Verified text-only runs: no tools, customizations, MCP servers or session writes; other agents are unsupported.
const ASIDE: &[(&str, &[&str])] = &[
    // Claude Code 2.1.325 --help: safe mode (no plugins, hooks, MCP or other customizations), an empty tool set, strict
    // MCP and no persistence; the person's own sign-in still serves the run.
    (
        "claude",
        &[
            "--no-session-persistence",
            "--setting-sources",
            "",
            "--safe-mode",
            "--tools",
            "",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--no-chrome",
            "--permission-mode",
            "plan",
            "--max-turns",
            "1",
        ],
    ),
];

/// What a slash command typed to a driven session does, as measured: (id, the commands cockpit applies as a setting,
/// the commands that only a terminal can show). A command in neither list goes to the agent as words when it lists it.
type SlashRow = (&'static str, &'static [(&'static str, Setting)], &'static [&'static str]);
const DRIVEN_SLASH: &[SlashRow] = &[
    // Claude Code 2.1.311 in -p mode: `/model`, `/effort` and `/fast` are its stream's `set_model` and settings
    // requests; these open a dialog or leave the session, which a stream has no screen for.
    (
        "claude",
        &[("model", Setting::Model), ("effort", Setting::Effort), ("fast", Setting::Fast)],
        &[
            "resume",
            "config",
            "permissions",
            "login",
            "logout",
            "vim",
            "theme",
            "terminal-setup",
            "exit",
            "quit",
            "doctor",
            "bug",
            "mobile",
            "statusline",
            "hooks",
            "mcp",
            "agents",
            "plugin",
        ],
    ),
];

/// The setting a slash command sets on a driven session of agent `id`, if it is one cockpit applies itself.
pub fn slash_setting(id: &str, command: &str) -> Option<Setting> {
    let own = get(id).map(|a| a.id).unwrap_or_else(|| id.to_string());
    DRIVEN_SLASH.iter().find(|(a, _, _)| *a == own).and_then(|(_, picks, _)| picks.iter().find(|(c, _)| *c == command).map(|(_, s)| *s))
}

/// Whether a slash command needs the agent's own terminal to show what it opens.
pub fn slash_needs_terminal(id: &str, command: &str) -> bool {
    let own = get(id).map(|a| a.id).unwrap_or_else(|| id.to_string());
    DRIVEN_SLASH.iter().find(|(a, _, _)| *a == own).is_some_and(|(_, _, refused)| refused.contains(&command))
}

/// Where each agent reads the commands and skills a person adds, as path patterns with `{home}` and `{cwd}`, `*` one
/// name and `**` any folders: their front matter describes them.
const COMMAND_FILES: &[(&str, &[&str])] = &[
    // Claude Code 2.1: commands and skills of the person, the project, and the plugins it installed.
    (
        "claude",
        &[
            "{home}/.claude/commands/**/*.md",
            "{cwd}/.claude/commands/**/*.md",
            "{home}/.claude/skills/*/SKILL.md",
            "{cwd}/.claude/skills/*/SKILL.md",
            "{home}/.claude/plugins/**/commands/*.md",
            "{home}/.claude/plugins/**/skills/*/SKILL.md",
        ],
    ),
    // Codex 0.9x: custom prompts, one Markdown file each.
    ("codex", &["{home}/.codex/prompts/*.md"]),
    // Gemini CLI 0.x: TOML commands of the person and the project.
    ("gemini", &["{home}/.gemini/commands/**/*.toml", "{cwd}/.gemini/commands/**/*.toml"]),
    // OpenCode and Kilo: Markdown commands of the person and the project.
    ("opencode", &["{home}/.config/opencode/command/*.md", "{cwd}/.opencode/command/*.md"]),
    ("kilo", &["{home}/.config/kilo/command/*.md", "{cwd}/.kilo/command/*.md"]),
];

/// The path patterns agent `id` reads its added commands from; none for an agent not measured.
pub fn command_files(id: &str) -> Vec<String> {
    let own = get(id).map(|a| a.id).unwrap_or_else(|| id.to_string());
    COMMAND_FILES.iter().find(|(a, _)| *a == own).map(|(_, files)| v(files)).unwrap_or_default()
}

/// The agents cockpit's plugin serves: its tools and the statusline's limits reach them through it, not an MCP config.
const PLUGIN: &[&str] = &["claude"];

/// The models only a routine of an agent takes, beside the ones it offers everywhere: Claude Code's `lightest` is haiku
/// at low effort with nothing loaded but the tools the routine lists.
const ROUTINE_ONLY: &[(&str, &[&str])] = &[("claude", &["lightest"])];

/// How an agent's list of its models reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listing {
    /// One model per line, its first word.
    Lines,
    /// A header line, then `provider  model  …` columns: `provider/model`.
    Table,
    /// `{"models": [{"selector": "provider/id"}]}`.
    Json,
    /// `{models: [{slug, display_name, visibility, supported_reasoning_levels, additional_speed_tiers}]}`.
    Codex,
    /// Not a command's text: its catalog request over its stream (`CATALOG`), answered in events `catalog` reads.
    Probe,
}

/// How an agent lists the models it can use, as its own help says: (id, the switches after its executable, how the
/// answer reads). An agent not listed has no such command.
const MODEL_LIST: &[(&str, &[&str], Listing)] = &[
    // pi 1.5.2 `src/cli/list-models.ts`: `provider  model  context  max-out  thinking  images`, two spaces apart.
    ("pi", &["--list-models"], Listing::Table),
    // oh-my-pi 18.20.2 `models-cli.ts`.
    ("omp", &["models", "--json"], Listing::Json),
    // OpenCode 1.19.5 and Kilo 7.8.27: one `provider/model` per line.
    ("opencode", &["models"], Listing::Lines),
    ("kilo", &["models"], Listing::Lines),
    // Antigravity CLI: the slug, then its name padded.
    ("antigravity-cli", &["models"], Listing::Lines),
    // Maki 0.7.2: one spec per line.
    ("maki", &["models"], Listing::Lines),
    // Codex 0.167: its catalog, each model with its effort levels and speed tiers.
    ("codex", &["debug", "models"], Listing::Codex),
    // Claude Code 2.1.311: no listing command; its `list_models` request over the stream answers instead.
    ("claude", NONE, Listing::Probe),
];

/// How an agent's catalog is asked over its stream, where its table has one: (id, the control request's subtype, the
/// switches that open the stream for that one request and keep nothing of it). Its answer reads as `catalog` says.
const CATALOG: &[(&str, &str, &[&str])] = &[
    // Claude Code 2.1.311: `list_models` as the first stdin line answers `{models: [{value, resolvedModel, displayName,
    // description, supportsEffort, supportedEffortLevels, supportsFastMode}]}`, no session written.
    ("claude", "list_models", &["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--no-session-persistence"]),
];

/// How an agent prints its version, where its help says: (id, the switches). What it printed keys what was read of it
/// (its catalog, its help): asked again only once the version changed.
const VERSIONED: &[(&str, &[&str])] = &[
    // Claude Code 2.1.311: "2.1.311 (Claude Code)".
    ("claude", &["--version"]),
    // Codex 0.167 and Gemini CLI 0.68: `--version` prints the version alone.
    ("codex", &["--version"]),
    ("gemini", &["--version"]),
];

/// The most models a listing gives.
pub const MAX_MODELS: usize = 500;

/// The models a listing names, in its order, each once and each a well-formed id (`model_id`), MAX_MODELS at most;
/// None when it names none, as an error printed in its place does.
pub fn listed_models(out: &str, how: Listing) -> Option<Vec<Offer>> {
    let mut found: Vec<String> = match how {
        Listing::Codex => return codex_catalog(out).filter(|offers| !offers.is_empty()),
        // A probe answers in events, read by `catalog`, never as text.
        Listing::Probe => return None,
        Listing::Lines => out.lines().filter_map(|l| l.split_whitespace().next()).map(str::to_string).collect(),
        Listing::Table => out
            .lines()
            .skip(1)
            .filter_map(|l| {
                let mut cells = l.split_whitespace();
                Some(format!("{}/{}", cells.next()?, cells.next()?))
            })
            .collect(),
        Listing::Json => serde_json::from_str::<Value>(out)
            .ok()
            .and_then(|v| v["models"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|m| m["selector"].as_str().map(str::to_string))
            .collect(),
    };
    let mut seen = std::collections::HashSet::new();
    found.retain(|m| model_id(m) && seen.insert(m.clone()));
    found.truncate(MAX_MODELS);
    (!found.is_empty()).then(|| found.into_iter().map(Offer::named).collect())
}

/// What a person picks beside a prompt for one agent, as measured; an empty list is not measured.
struct Measured {
    id: &'static str,
    /// The models its pickers offer.
    offered: &'static [&'static str],
    /// Its effort levels, lowest first.
    efforts: &'static [&'static str],
    /// Its lightest model, as it takes it; empty where not measured.
    light: &'static str,
    /// How an effort level is passed at start.
    effort: &'static [&'static str],
    /// How fast mode is set at start.
    fast: &'static [&'static str],
}

const PICKS: &[Measured] = &[
    // Claude Code 2.1.325: `--model` takes these aliases and `--effort` these levels; `--settings` lays `fastMode` over its
    // settings, the same layer its stream's `apply_flag_settings` writes.
    Measured {
        id: "claude",
        offered: &["opus", "sonnet", "haiku", "fable"],
        efforts: &["low", "medium", "high", "xhigh", "max"],
        light: "haiku",
        effort: &["--effort", "{effort}"],
        fast: &["--settings", "{\"fastMode\":{fast}}"],
    },
    // Codex 0.167: `-c` sets `model_reasoning_effort`, and `service_tier = "fast"` its fast tier; its listing names the models.
    Measured {
        id: "codex",
        offered: NONE,
        efforts: &["low", "medium", "high", "xhigh", "max", "ultra"],
        light: "",
        effort: &["-c", "model_reasoning_effort=\"{effort}\""],
        fast: &["-c", "service_tier=\"{speed}\""],
    },
];

/// How each agent moves to its next permission mode and toggles its focus view in its own terminal, as measured: Claude
/// Code 2.1.311 cycles auto, manual, accept edits and plan on ⇧⇥, and `/focus` says "Focus view enabled" or "disabled".
const TERMINAL: &[(&str, &[&str], &str)] = &[("claude", &["shift+tab"], "/focus")];

/// The settings a running session accepts and when each applies: the next model call or by the next turn.
const LIVE: &[(&str, &[(Setting, &str)])] = &[("claude", &[(Setting::Model, "call"), (Setting::Effort, "turn"), (Setting::Fast, "turn")])];

/// The tools that start work beside a turn, as measured: (id, subagents, background jobs, workflows). A background
/// tool starts one only when its input has `run_in_background: true`.
const TASKS: &[Lists] = &[
    // Claude Code 2.1.311: `Agent` (`Task` in older sessions) spawns a subagent whose result names its `agentId`, a
    // `Bash` call with `run_in_background` answers with a `backgroundTaskId`, and `Workflow` runs a script.
    ("claude", &["Agent", "Task"], &["Bash"], &["Workflow"]),
    // Codex: `spawn_agent` is its spawn tool by name only, not measured against a rollout.
    ("codex", &["spawn_agent"], NONE, NONE),
];

/// How each setting changes in an agent's own terminal while it runs, as measured.
fn typing(id: &str) -> Typing {
    let typed = |c: &str| Some(Typed::Command(c.into()));
    match id {
        // Claude Code 2.1.325: `/model [model]`, `/effort [low|…|max|auto]` and `/fast [on|off]` each take the value.
        "claude" => Typing { model: typed("/model {model}"), effort: typed("/effort {effort}"), fast: typed("/fast {fast}") },
        // Codex 0.167: `/model [model] [--reasoning=<level>]` takes both; `/fast` only toggles, so it is changed by hand.
        "codex" => Typing { model: typed("/model {model}"), effort: typed("/model --reasoning={effort}"), fast: Some(Typed::ByHand("/fast".into())) },
        _ => Typing::default(),
    }
}

fn control(list: &[&str]) -> Option<Vec<String>> {
    (!list.is_empty()).then(|| v(list))
}

/// The built-in table, before the config's overrides. Agents Herdr integrates; see `herdr integration --help`.
/// The variable each agent reads its config folder from, which picks the account it runs on.
const ACCOUNT_ENV: &[(&str, &str)] = &[("claude", "CLAUDE_CONFIG_DIR")];

pub fn builtin() -> Vec<Agent> {
    ROWS.iter()
        .map(|row| {
            let mut agent = row.agent();
            if let Some((_, mode, interrupt, clear)) = CONTROLS.iter().find(|(id, _, _, _)| *id == row.id) {
                (agent.mode, agent.interrupt, agent.clear) = (control(mode), control(interrupt), control(clear));
            }
            if let Some((_, shapes)) = PERMISSION.iter().find(|(id, _)| *id == row.id) {
                agent.permission = v(shapes);
            }
            agent.plan = PLANS.iter().find(|(id, _)| *id == row.id).map(|(_, plan)| *plan);
            if let Some((_, stream, new_session)) = STREAM.iter().find(|(id, _, _)| *id == row.id) {
                (agent.stream, agent.new_session) = (Some(v(stream)), v(new_session));
            }
            if let Some(p) = PICKS.iter().find(|p| p.id == row.id) {
                (agent.offered, agent.efforts, agent.effort, agent.fast) = (v(p.offered), v(p.efforts), control(p.effort), control(p.fast));
                agent.light = (!p.light.is_empty()).then(|| p.light.to_string());
            }
            if let Some((_, cycle, focus)) = TERMINAL.iter().find(|(id, _, _)| *id == row.id) {
                (agent.cycle, agent.focus) = (control(cycle), (!focus.is_empty()).then(|| focus.to_string()));
            }
            agent.live = LIVE.iter().find(|(id, _)| *id == row.id).map(|(_, l)| l.iter().map(|(setting, _)| *setting).collect()).unwrap_or_default();
            agent.typing = typing(row.id);
            agent.tasks = TASKS.iter().find(|(id, _, _, _)| *id == row.id).map(|(_, spawn, background, workflow)| Tasks {
                spawn: v(spawn),
                background: v(background),
                workflow: v(workflow),
            });
            agent.account_env = ACCOUNT_ENV.iter().find(|(id, _)| *id == row.id).map(|(_, var)| var.to_string());
            agent
        })
        .collect()
}

/// The task tools of the agent that writes transcripts of this format, from the table; None where none is measured.
pub fn tasks_of(kind: Transcripts) -> Option<Tasks> {
    builtin().into_iter().find(|a| a.transcripts == kind && kind != Transcripts::None).and_then(|a| a.tasks)
}

/// `{opts}` marks where switches go when they must precede a subcommand; elsewhere they follow the template.
const ROWS: &[Row] = &[
    Row {
        id: "claude",
        name: "Claude Code",
        bin: "claude",
        aliases: &["claude-code"],
        start: &["{bin}", "{prompt}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["--model", "{model}"],
        resume: &["{bin}", "--resume", "{session}", "{prompt}"],
        fork: &["{bin}", "--resume", "{session}", "--fork-session", "{prompt}"],
        run_fork: &["{bin}", "-p", "--resume", "{session}", "--fork-session", "{prompt}"],
        edits: &["--permission-mode", "acceptEdits"],
        add_dir: &["--add-dir", "{dir}"],
        system: &["--append-system-prompt", "{text}"],
        models: &["haiku", "sonnet", "opus", "lightest", "light", "claude-"],
        transcripts: Transcripts::Claude,
    },
    Row {
        id: "codex",
        name: "Codex",
        bin: "codex",
        aliases: NONE,
        start: &["{bin}", "{prompt}"],
        run: &["{bin}", "exec", "--skip-git-repo-check", "{opts}", "{prompt}"],
        model: &["--model", "{model}"],
        resume: &["{bin}", "resume", "{session}", "{prompt}"],
        fork: &["{bin}", "fork", "{session}", "{prompt}"],
        run_fork: &["{bin}", "exec", "--skip-git-repo-check", "{opts}", "fork", "{session}", "{prompt}"],
        edits: &["--sandbox", "workspace-write"],
        add_dir: &["--add-dir", "{dir}"],
        system: NONE,
        models: &["gpt-", "o1", "o3", "o4", "codex-"],
        transcripts: Transcripts::Codex,
    },
    // pi 1.5.2 (badlogic/pi-mono `args.ts`): it asks before no tool, so an unattended run needs no switch to edit.
    Row {
        id: "pi",
        name: "Pi",
        bin: "pi",
        aliases: NONE,
        start: &["{bin}", "{prompt}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["--model", "{model}"],
        resume: &["{bin}", "--session", "{session}", "{prompt}"],
        fork: &["{bin}", "--fork", "{session}", "{prompt}"],
        run_fork: NONE,
        edits: NONE,
        add_dir: NONE,
        system: &["--append-system-prompt", "{text}"],
        models: NONE,
        transcripts: Transcripts::Pi,
    },
    // oh-my-pi 18.20.2 (`docs/cli-reference.md`, `approval-mode.md`): write mode approves reads and writes, not commands.
    Row {
        id: "omp",
        name: "Oh My Pi",
        bin: "omp",
        aliases: NONE,
        start: &["{bin}", "{prompt}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["--model", "{model}"],
        resume: &["{bin}", "--resume={session}", "{prompt}"],
        fork: &["{bin}", "--fork", "{session}", "{prompt}"],
        run_fork: NONE,
        edits: &["--approval-mode", "write"],
        add_dir: &["--add-dir", "{dir}"],
        system: &["--append-system-prompt", "{text}"],
        models: NONE,
        transcripts: Transcripts::Omp,
    },
    // Copilot CLI 1.0.112 (its command reference): `write` is file creation and edits; it has no system prompt switch.
    Row {
        id: "copilot",
        name: "GitHub Copilot CLI",
        bin: "copilot",
        aliases: &["github-copilot", "ghcs"],
        start: &["{bin}", "-i {prompt}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["--model={model}"],
        resume: &["{bin}", "--resume={session}", "-i {prompt}"],
        fork: NONE,
        run_fork: NONE,
        edits: &["--allow-tool=write"],
        add_dir: &["--add-dir={dir}"],
        system: NONE,
        models: NONE,
        transcripts: Transcripts::Copilot,
    },
    // Devin CLI 3000.12.57 (docs.devin.ai permissions): accept-edits approves edits in the workspace; no add-dir switch.
    Row {
        id: "devin",
        name: "Devin",
        bin: "devin",
        aliases: &["devin-cli"],
        start: &["{bin}", "-- {prompt}"],
        run: &["{bin}", "-p", "-- {prompt}"],
        model: &["--model", "{model}"],
        resume: &["{bin}", "--resume", "{session}"],
        fork: NONE,
        run_fork: NONE,
        edits: &["--permission-mode", "accept-edits"],
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Droid (docs.factory.com `droid exec`): `--auto low` allows project edits; `exec` resumes with `-s`, not `--resume`.
    Row {
        id: "droid",
        name: "Droid",
        bin: "droid",
        aliases: NONE,
        start: &["{bin}", "{prompt}"],
        run: &["{bin}", "exec", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--resume={session}"],
        fork: &["{bin}", "--fork", "{session}"],
        run_fork: NONE,
        edits: &["--auto", "low"],
        add_dir: NONE,
        system: &["--append-system-prompt", "{text}"],
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Kimi Code 1.52.0 (`kimi-command.md`): only `--print` is headless, and it approves every tool call (`--afk`).
    Row {
        id: "kimi",
        name: "Kimi Code",
        bin: "kimi",
        aliases: &["kimi-code"],
        start: &["{bin}"],
        run: &["{bin}", "--print", "-p", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--session", "{session}"],
        fork: NONE,
        run_fork: NONE,
        edits: NONE,
        add_dir: &["--add-dir", "{dir}"],
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    // OpenCode 1.19.5 (`permissions.mdx`): a run edits without asking by default; no add-dir or system prompt switch.
    Row {
        id: "opencode",
        name: "OpenCode",
        bin: "opencode",
        aliases: &["opencode2", "open-code"],
        start: &["{bin}", "--prompt {prompt}"],
        run: &["{bin}", "run", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--session", "{session}", "--prompt {prompt}"],
        fork: &["{bin}", "--session", "{session}", "--fork", "--prompt {prompt}"],
        run_fork: &["{bin}", "run", "--session", "{session}", "--fork", "{prompt}"],
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Kilo 7.8.27 (its CLI reference): as OpenCode, a run edits without asking by default.
    Row {
        id: "kilo",
        name: "Kilo Code",
        bin: "kilo",
        aliases: &["kilo-code"],
        start: &["{bin}", "--prompt {prompt}"],
        run: &["{bin}", "run", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--session", "{session}", "--prompt {prompt}"],
        fork: &["{bin}", "--session", "{session}", "--fork", "--prompt {prompt}"],
        run_fork: &["{bin}", "run", "--session", "{session}", "--fork", "{prompt}"],
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Hermes 0.23.1 (`_parser.py`): `-z` bypasses approvals; file writes never ask, so it needs no edits switch.
    Row {
        id: "hermes",
        name: "Hermes",
        bin: "hermes",
        aliases: &["hermes-agent"],
        start: &["{bin}", "chat", "-q {prompt}"],
        run: &["{bin}", "-z", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--resume", "{session}"],
        fork: NONE,
        run_fork: NONE,
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Qoder CLI 1.1.82 (docs.qoder.com, whose command is now `qoder`): accept_edits approves edits in its folders.
    Row {
        id: "qodercli",
        name: "Qoder CLI",
        bin: "qodercli",
        aliases: &["qoder", "qoderclicn", "qodercn"],
        start: &["{bin}", "-i {prompt}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--resume={session}", "-i {prompt}"],
        fork: &["{bin}", "--resume={session}", "--fork-session", "-i {prompt}"],
        run_fork: &["{bin}", "--resume={session}", "--fork-session", "-p", "{prompt}"],
        edits: &["--permission-mode", "accept_edits"],
        add_dir: &["--add-dir", "{dir}"],
        system: &["--append-system-prompt", "{text}"],
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Qwen Code 0.27.3 (`top-level-options.ts`): auto-edit approves file edits; additional folders use `--include-directories`.
    Row {
        id: "qwen",
        name: "Qwen Code",
        bin: "qwen",
        aliases: &["qwen-code"],
        start: &["{bin}", "-i {prompt}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--resume={session}", "-i {prompt}"],
        fork: &["{bin}", "--resume={session}", "--fork-session", "-i {prompt}"],
        run_fork: &["{bin}", "--resume={session}", "--fork-session", "-p", "{prompt}"],
        edits: &["--approval-mode", "auto-edit"],
        add_dir: &["--include-directories", "{dir}"],
        system: &["--append-system-prompt", "{text}"],
        models: &["qwen"],
        transcripts: Transcripts::Qwen,
    },
    // Letta Code 0.34.61 (docs.letta.com permissions): acceptEdits allows edits, asks for the rest; no add-dir switch.
    Row {
        id: "letta",
        name: "Letta Code",
        bin: "letta",
        aliases: &["letta-code"],
        start: &["{bin}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--conversation", "{session}"],
        fork: NONE,
        run_fork: NONE,
        edits: &["--permission-mode", "acceptEdits"],
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Cursor CLI 2026.10.22 (cursor.com headless): print mode writes files in its workspace without a switch.
    Row {
        id: "cursor",
        name: "Cursor Agent",
        bin: "cursor-agent",
        aliases: &["cursor-agent"],
        start: &["{bin}", "{prompt}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["--model", "{model}"],
        resume: &["{bin}", "--resume={session}", "{prompt}"],
        fork: NONE,
        run_fork: NONE,
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Mastra Code 0.50.0 (`headless.mdx`): its default permission mode approves everything; no narrower switch.
    Row {
        id: "mastracode",
        name: "Mastra Code",
        bin: "mastracode",
        aliases: &["mastra-code"],
        start: &["{bin}", "--tui-prompt {prompt}"],
        run: &["{bin}", "--prompt {prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--thread", "{session}", "--tui-prompt {prompt}"],
        fork: &["{bin}", "--thread", "{session}", "--clone-thread", "--tui-prompt {prompt}"],
        run_fork: NONE,
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Antigravity CLI 1.4.4 (antigravity.google headless): accept-edits approves workspace edits; no system prompt switch.
    Row {
        id: "antigravity-cli",
        name: "Antigravity",
        bin: "agy",
        aliases: &["agy", "antigravity"],
        start: &["{bin}", "-i {prompt}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["--model", "{model}"],
        resume: &["{bin}", "--conversation", "{session}", "-i {prompt}"],
        fork: NONE,
        run_fork: NONE,
        edits: &["--mode", "accept-edits"],
        add_dir: &["--add-dir", "{dir}"],
        system: NONE,
        models: &["gemini-"],
        transcripts: Transcripts::None,
    },
    // Grok Build 1.0.77 (docs.x.ai reference and enterprise pages): `--rules` appends to its system prompt.
    Row {
        id: "grok",
        name: "Grok",
        bin: "grok",
        aliases: &["grok-build"],
        start: &["{bin}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--resume={session}"],
        fork: &["{bin}", "--resume={session}", "--fork-session"],
        run_fork: &["{bin}", "--resume={session}", "--fork-session", "-p", "{prompt}"],
        edits: &["--permission-mode", "acceptEdits"],
        add_dir: NONE,
        system: &["--rules", "{text}"],
        models: &["grok-"],
        transcripts: Transcripts::None,
    },
    // Gemini CLI 0.68.0 (google-gemini/gemini-cli `config.ts`, `session-management.md`): no fork; headless asks are
    // denied, so auto_edit allows edits and refuses commands; its system prompt only goes whole, through a variable.
    Row {
        id: "gemini",
        name: "Gemini CLI",
        bin: "gemini",
        aliases: &["gemini-cli"],
        start: &["{bin}", "-i {prompt}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--resume", "{session}"],
        fork: NONE,
        run_fork: NONE,
        edits: &["--approval-mode", "auto_edit"],
        add_dir: &["--include-directories", "{dir}"],
        system: NONE,
        models: &["gemini-", "pro", "flash", "flash-lite"],
        transcripts: Transcripts::Gemini,
    },
    // Cline CLI 3.0.74 (cline/cline `apps/cli/src/commands/program.ts`): a prompt alone is one turn in yolo mode, so a
    // run approves every tool; `--id` opens the TUI and drops a prompt; `--system` replaces its system prompt.
    Row {
        id: "cline",
        name: "Cline",
        bin: "cline",
        aliases: NONE,
        start: &["{bin}", "-i {prompt}"],
        run: &["{bin}", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--id", "{session}"],
        fork: NONE,
        run_fork: NONE,
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Kiro CLI 3.3 (kiro.dev cli-commands, headless): a run needs KIRO_API_KEY on a paid plan; `--trust-tools=write`
    // trusts writes and deletes in a headless run; forks only from inside a session.
    Row {
        id: "kiro",
        name: "Kiro CLI",
        bin: "kiro-cli",
        aliases: &["kiro-cli"],
        start: &["{bin}", "chat", "{prompt}"],
        run: &["{bin}", "chat", "--no-interactive", "{opts}", "{prompt}"],
        model: &["--model", "{model}"],
        resume: &["{bin}", "chat", "--resume-id", "{session}"],
        fork: NONE,
        run_fork: NONE,
        edits: &["--trust-tools=write"],
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Amp (ampcode.com cli and execute-mode docs, 2026-11): a first message only through its input, so a start opens it
    // bare; it asks before no tool; its threads live on its server.
    Row {
        id: "amp",
        name: "Amp",
        bin: "amp",
        aliases: NONE,
        start: &["{bin}"],
        run: &["{bin}", "-x", "{prompt}"],
        model: &["--model", "{model}"],
        resume: &["{bin}", "threads", "continue", "{session}"],
        fork: NONE,
        run_fork: NONE,
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Maki 0.7.2 (tontinton/maki `src/cli.rs`, its headless and permissions docs): file writes in its folder are allowed
    // already; `--add-dir` is accepted and ignored.
    Row {
        id: "maki",
        name: "Maki",
        bin: "maki",
        aliases: NONE,
        start: &["{bin}", "{prompt}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--resume", "{session}", "{prompt}"],
        fork: &["{bin}", "--resume", "{session}", "--fork-session", "{prompt}"],
        run_fork: &["{bin}", "-p", "--resume", "{session}", "--fork-session", "{prompt}"],
        edits: NONE,
        add_dir: NONE,
        system: &["--append-system-prompt", "{text}"],
        models: NONE,
        transcripts: Transcripts::None,
    },
    // Muse Code 1.4.18 (dev.meta.ai muse-code docs): in-workspace writes pass in every approval mode; a first message at
    // start and a headless resume of a past session are not documented.
    Row {
        id: "muse",
        name: "Muse Code",
        bin: "muse",
        aliases: &["muse-code", "muse-cli"],
        start: &["{bin}"],
        run: &["{bin}", "exec", "{opts}", "{prompt}"],
        model: &["--model", "{model}"],
        resume: &["{bin}", "resume", "{session}"],
        fork: NONE,
        run_fork: NONE,
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: &["muse-"],
        transcripts: Transcripts::None,
    },
];

/// The table with `agents.commands.<id>` from the config laid over it field by field (a new id adds an agent).
pub fn all() -> Vec<Agent> {
    in_config(&files::config())
}

/// The agent table with the given config's command overrides.
pub fn in_config(config: &Value) -> Vec<Agent> {
    match config["agents"]["commands"].as_object() {
        Some(overrides) => laid_over(builtin(), overrides),
        None => builtin(),
    }
}

/// `list` with each override laid over its agent's row, or added as a new one.
fn laid_over(mut list: Vec<Agent>, overrides: &serde_json::Map<String, Value>) -> Vec<Agent> {
    for (id, o) in overrides {
        let at = list.iter().position(|a| &a.id == id);
        let mut agent = at.map(|i| list[i].clone()).unwrap_or_else(|| Agent {
            id: id.clone(),
            name: id.clone(),
            bin: id.clone(),
            aliases: vec![],
            start: v(&["{bin}", "{prompt}"]),
            run: vec![],
            model: vec![],
            resume: vec![],
            fork: vec![],
            transcripts: Transcripts::None,
            plan: None,
            run_fork: vec![],
            edits: vec![],
            add_dir: vec![],
            system: vec![],
            models: vec![],
            signed_out: vec![],
            mode: None,
            interrupt: None,
            clear: None,
            permission: vec![],
            stream: None,
            new_session: vec![],
            offered: vec![],
            efforts: vec![],
            light: None,
            cycle: None,
            focus: None,
            effort: None,
            fast: None,
            live: vec![],
            typing: Typing::default(),
            tasks: None,
            account_env: None,
        });
        let strs = |x: &Value| x.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect::<Vec<_>>());
        if let Some(s) = o["name"].as_str() {
            agent.name = s.into();
        }
        if let Some(s) = o["bin"].as_str() {
            agent.bin = s.into();
        }
        let fields = [
            ("start", &mut agent.start),
            ("run", &mut agent.run),
            ("model", &mut agent.model),
            ("resume", &mut agent.resume),
            ("fork", &mut agent.fork),
            ("aliases", &mut agent.aliases),
            ("runFork", &mut agent.run_fork),
            ("edits", &mut agent.edits),
            ("addDir", &mut agent.add_dir),
            ("system", &mut agent.system),
            ("models", &mut agent.models),
            ("signedOut", &mut agent.signed_out),
            ("offered", &mut agent.offered),
            ("efforts", &mut agent.efforts),
        ];
        for (key, field) in fields {
            if let Some(list) = strs(&o[key]) {
                *field = list;
            }
        }
        // Patterns that do not compile are left out; with none left the measured ones stay, so detection never turns off.
        if let Some(list) = strs(&o["permission"]) {
            let read: Vec<String> = list.into_iter().filter(|p| regex::Regex::new(p).is_ok()).collect();
            if !read.is_empty() {
                agent.permission = read;
            }
        }
        if let Some(list) = strs(&o["live"]) {
            agent.live = list.iter().filter_map(|s| Setting::of(s)).collect();
        }
        if let Some(light) = o["light"].as_str() {
            agent.light = (!light.trim().is_empty()).then(|| light.trim().to_string());
        }
        if let Some(focus) = o["focus"].as_str() {
            agent.focus = (!focus.trim().is_empty()).then(|| focus.trim().to_string());
        }
        // An empty list says it has no such control.
        for (key, field) in [
            ("mode", &mut agent.mode),
            ("interrupt", &mut agent.interrupt),
            ("effort", &mut agent.effort),
            ("fast", &mut agent.fast),
            ("cycle", &mut agent.cycle),
        ] {
            if let Some(list) = strs(&o[key]) {
                *field = (!list.is_empty()).then_some(list);
            }
        }
        match at {
            Some(i) => list[i] = agent,
            None => list.push(agent),
        }
    }
    list
}

/// The permission patterns of `agents.commands.<id>.permission` in the config that do not compile, by agent id.
pub fn unreadable_permission() -> Vec<(String, String)> {
    unreadable_in(&files::config()["agents"]["commands"])
}

fn unreadable_in(overrides: &Value) -> Vec<(String, String)> {
    let Some(overrides) = overrides.as_object() else { return Vec::new() };
    overrides
        .iter()
        .flat_map(|(id, o)| o["permission"].as_array().into_iter().flatten().filter_map(|p| p.as_str()).map(move |p| (id.clone(), p.to_string())))
        .filter(|(_, p)| regex::Regex::new(p).is_err())
        .collect()
}

pub fn get(id: &str) -> Option<Agent> {
    all().into_iter().find(|a| a.id == id || a.aliases.iter().any(|x| x == id))
}

// ---------------------------------------------------------------------------------------------------------------------
// What an agent draws around its words

/// The lines of an agent's screen that are its own furniture or news of a limit, as patterns (regex) of one line with
/// its frame and indent taken off. None of them is ever a question or a choice.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Screen {
    /// Its footer: the mode line and key hints under its input box, which a status line sits right above.
    pub footer: Vec<String>,
    /// What it shows while it works and when a turn ended: a spinner, how long the turn took.
    pub busy: Vec<String>,
    /// It reached a usage or rate limit.
    pub limit: Vec<String>,
    /// The limit is over.
    pub lifted: Vec<String>,
    /// One row of the menu that completes a slash command as it is typed: the command in group 1, its description
    /// in group 2 when the row has one.
    pub completion: Vec<String>,
    /// The hint under a dialog it opened (`/model`, `/config`): the keys that confirm, move or close it.
    pub dialog_hint: Vec<String>,
    /// The title line a dialog of its opens with.
    pub dialog_title: Vec<String>,
}

/// How each agent's screen reads around its words, as seen: (id, footer, busy, limit, lifted, completion, dialog hint,
/// dialog title).
type ScreenRow = (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
);

/// A slash completion row as most agents draw it, the cursor or not in front: `❯ /model   Set the AI model`.
pub const COMPLETION_ROW: &str = r"^\s*(?:[❯›>]\s*)?(/[\w:.-]+)(?:\s{2,}(\S.*))?$";
/// The hint most agents end a dialog with.
pub const DIALOG_HINT: &str = r"(?i)\b(esc|escape) to (cancel|close|exit|go back|clear)\b|\benter to (confirm|select|submit|set|continue|save)\b";

const SCREENS: &[ScreenRow] = &[
    // Claude Code 2.1: "⏵⏵ auto mode on (shift+tab to cycle)", "⏸ plan mode on", "? for shortcuts"; "✻ Crunched for
    // 21m 49s"; "Usage limit reached · continuing automatically at 8am", "You've hit your limit · resets 8am".
    (
        "claude",
        &[
            r"(?i)^\W{0,3}(auto mode|accept edits|auto-accept edits|plan mode|bypass permissions) on( \(shift\+tab to cycle\))?$",
            r"(?i)^\W{0,3}[\w -]+ \(shift\+tab to cycle\)$",
            r"^\? for shortcuts(\s{2,}.*)?$",
        ],
        &[r"^[✻✽✶✳✢·*∗]\s+\w+(…|\.\.\.)( \(.*\))?$", r"^[✻✽✶✳✢·*∗]\s+\w+ for (\d+(\.\d+)?[hms] ?)+( · .*)?$"],
        &[
            r"(?i)\b(usage|weekly|session|5-hour|opus|sonnet) limit reached\b",
            r"(?i)\byou.ve (hit|reached) your\b.{0,40}\blimit\b",
            r"(?i)\bAPI Error: 429\b|\brate_limit_error\b",
        ],
        &[r"(?i)\blimit has reset\b"],
        // 2.1.311: "❯ /model    Set the AI model for Claude Code (currently Fable 5.1)", a wrapped description indented
        // under it; a dialog ends "Enter to set as default · s to use this session only · Esc to cancel", "←/→ to
        // adjust · Enter to confirm", "Type to filter · Enter/↓ to select · ↑ to tabs · Esc to clear", and opens with
        // "Select model", "Effort", "Resume session", "Settings".
        &[COMPLETION_ROW],
        &[DIALOG_HINT, r"(?i)\benter(/↓)? to (set as default|select|confirm)\b", r"(?i)\btype to (search|filter)\b"],
        &[r"^(Select model|Effort|Resume session|Settings|Fast mode|Permissions|Select effort level)\b"],
    ),
    // Codex: "? for shortcuts", "41% context left"; "• Working (12s • esc to interrupt)"; "You've hit your usage limit".
    // Its slash popup draws "/model    choose what model and reasoning effort to use"; its dialogs are unmeasured and
    // read by the shapes every agent shares.
    (
        "codex",
        &[r"^\? for shortcuts(\s{2,}.*)?$", r"(?i)(^|\s{2,})\d{1,3}% context left$"],
        &[r"(?i)^[•◦]\s+Working\b.*\besc to interrupt\)?$"],
        &[r"(?i)\byou.ve hit your usage limit\b"],
        &[],
        &[COMPLETION_ROW],
        &[DIALOG_HINT],
        &[],
    ),
];

/// The screen shapes of agent `id`; for a program the table does not know, every agent's furniture and no limit words.
pub fn screen(id: &str) -> Screen {
    let of = |row: &ScreenRow| Screen {
        footer: v(row.1),
        busy: v(row.2),
        limit: v(row.3),
        lifted: v(row.4),
        completion: v(row.5),
        dialog_hint: v(row.6),
        dialog_title: v(row.7),
    };
    let own = get(id).map(|a| a.id).unwrap_or_else(|| id.to_string());
    if let Some(row) = SCREENS.iter().find(|row| row.0 == own) {
        return of(row);
    }
    // Every agent's furniture, and the completion row and dialog hint most share; a limit only in its own words, which
    // an agent without a row has none of.
    let mut all = SCREENS.iter().map(of).fold(Screen::default(), |mut all, s| {
        all.footer.extend(s.footer);
        all.busy.extend(s.busy);
        all
    });
    all.completion = vec![COMPLETION_ROW.to_string()];
    all.dialog_hint = vec![DIALOG_HINT.to_string()];
    all
}

/// The agent a Herdr terminal runs, from its `program` (or the agent's own report, or its name as the app shows it).
pub fn of_program(program: &str) -> Option<Agent> {
    let p = program.trim().to_ascii_lowercase();
    all().into_iter().find(|a| a.id == p || a.bin == p || a.name.to_ascii_lowercase() == p || a.aliases.iter().any(|x| x.to_ascii_lowercase() == p))
}

// ---------------------------------------------------------------------------------------------------------------------
// Logos

/// Herdr's icon for each agent (`<herdr-app home>/icons/<name>.svg`) and the ids, aliases and Herdr kinds it stands for.
const ICONS: &[(&str, &[&str])] = &[
    ("claude", &["claude", "claude-code"]),
    ("codex", &["codex"]),
    ("pi", &["pi"]),
    ("copilot", &["copilot", "github-copilot", "github_copilot", "ghcs"]),
    ("kimi", &["kimi", "kimi-code", "kimicodecli"]),
    ("opencode", &["opencode", "open-code", "open_code", "opencode2"]),
    ("hermes", &["hermes", "hermes-agent"]),
    ("qoder", &["qoder", "qodercli", "qoderclicn", "qodercn"]),
    ("cursor", &["cursor", "cursor-agent"]),
    ("mastracode", &["mastracode", "mastra-code"]),
    ("antigravity", &["antigravity", "antigravity-cli", "antigravity_cli", "agy"]),
    ("grok", &["grok", "grok-build"]),
    ("gemini", &["gemini", "gemini-cli"]),
    ("kiro", &["kiro", "kiro-cli"]),
    ("amp", &["amp"]),
    ("muse", &["muse"]),
    ("crush", &["crush"]),
    ("command-code", &["command-code"]),
    ("prime-agent", &["prime-agent"]),
    ("ollama", &["ollama"]),
    ("pig", &["pig"]),
];

/// What Herdr calls the agents it integrates that the table has no row for.
const HERDR_NAMES: &[(&str, &str)] = &[("crush", "Crush"), ("command-code", "Command Code"), ("prime-agent", "Prime"), ("ollama", "Ollama"), ("pig", "PiG")];

/// The row of a table already read that an agent goes by: its id, executable or an alias, in any case.
pub fn in_table<'a>(table: &'a [Agent], id: &str) -> Option<&'a Agent> {
    let p = id.trim().to_ascii_lowercase();
    table.iter().find(|a| a.id == p || a.bin == p || a.aliases.iter().any(|x| x.to_ascii_lowercase() == p))
}

fn icon_named(word: &str) -> Option<&'static str> {
    let w = word.trim().to_ascii_lowercase();
    ICONS.iter().find(|(_, ids)| ids.contains(&w.as_str())).map(|(icon, _)| *icon)
}

/// The name of Herdr's icon for an agent, from its id, an alias or the kind Herdr reports, in a table already read;
/// `agent`, Herdr's neutral one, when it has none.
pub fn icon_in(table: &[Agent], id: &str) -> &'static str {
    if let Some(icon) = icon_named(id) {
        return icon;
    }
    in_table(table, id).and_then(|a| std::iter::once(&a.id).chain(&a.aliases).chain(std::iter::once(&a.bin)).find_map(|w| icon_named(w))).unwrap_or("agent")
}

/// What people read for an agent, from a table already read: the table's name, else Herdr's, else the id as given.
pub fn display_name_in(table: &[Agent], id: &str) -> String {
    if let Some(a) = in_table(table, id) {
        return a.name.clone();
    }
    let p = id.trim().to_ascii_lowercase();
    HERDR_NAMES.iter().find(|(k, _)| *k == p).map(|(_, n)| n.to_string()).unwrap_or_else(|| id.trim().to_string())
}

/// The TUI's text mark for an agent, from a table already read, standing in for its logo: one lowercase word, its
/// icon's name (`claude`, `codex`, `qoder`), else its id.
pub fn mark_in(table: &[Agent], id: &str) -> String {
    mark_for(icon_in(table, id), id)
}

fn mark_for(icon: &str, id: &str) -> String {
    if icon == "agent" {
        id.chars().filter(|c| c.is_ascii_alphanumeric() || "-_".contains(*c)).flat_map(char::to_lowercase).collect()
    } else {
        icon.to_string()
    }
}

/// The agent cockpit starts when nothing says which: `agents.default` in the config, else the first one installed in
/// table order, else Claude Code.
pub fn default() -> Agent {
    let config = files::config();
    if let Some(a) = config["agents"]["default"].as_str().and_then(get) {
        return a;
    }
    let list = all();
    list.iter().find(|a| a.installed()).cloned().unwrap_or_else(|| list[0].clone())
}

/// The agent a one-off headless job runs when nothing says which: the default when it runs headless, else the first
/// installed one that does, else the default (whose missing headless mode the job then reports).
pub fn for_headless() -> Agent {
    let first = default();
    if first.can_run() {
        return first;
    }
    all().into_iter().find(|a| a.can_run() && a.installed()).unwrap_or(first)
}

/// How to ask the default agent for what a cockpit command writes: Claude Code has the slash commands, every other
/// agent is asked in words. `cockpit setup` picks which agent this is.
pub fn ask_for(command: &str, in_words: &str) -> String {
    let agent = default();
    if agent.id == "claude" { format!("run /{command} in a Claude Code session") } else { format!("ask {} to {in_words}", agent.name) }
}

/// `id` when given and known, else the default.
pub fn pick(id: Option<&str>) -> Agent {
    id.filter(|i| !i.is_empty()).and_then(get).unwrap_or_else(default)
}

/// The agent named at `key` in the config's `agents` (such as `agents.recap`), else the default.
pub fn for_use(key: &str) -> Agent {
    pick(files::config()["agents"][key].as_str())
}

/// The model a config value gives this agent: a name it takes, or its entry in a map by agent id
/// (`{"claude": "haiku", "codex": "gpt-5-mini"}`).
pub fn model_in(value: &Value, agent: &Agent) -> Option<String> {
    let model = value.as_str().filter(|m| agent.takes_model(m)).or_else(|| value[&agent.id].as_str())?;
    (!model.is_empty()).then(|| model.to_string())
}

fn runs(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.is_file() && p.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

impl Agent {
    /// Whether cockpit's own plugin serves it, rather than its MCP config.
    pub fn has_plugin(&self) -> bool {
        PLUGIN.contains(&self.id.as_str())
    }

    /// Its executable: `COCKPIT_<ID>`, else its own install places (before PATH, which lacks the shell's aliases in a
    /// Herdr app terminal), else the first on PATH, else an app's bundled copy, else the bare name.
    pub fn path(&self) -> String {
        let var = format!("COCKPIT_{}", self.id.to_ascii_uppercase().replace('-', "_"));
        if let Some(set) = std::env::var(var).ok().filter(|c| !c.is_empty()) {
            return set;
        }
        if self.bin.contains('/') {
            return self.bin.clone();
        }
        let home = files::home();
        let mut first = vec![home.join(".local/bin").join(&self.bin)];
        let mut last: Vec<std::path::PathBuf> = vec![];
        match self.id.as_str() {
            "claude" => first.insert(0, home.join(".claude/local/claude")),
            "codex" => last.push("/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex".into()),
            _ => {}
        }
        let on_path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join(&self.bin)).collect::<Vec<_>>()).unwrap_or_default();
        first.into_iter().chain(on_path).chain(last).find(|p| runs(p)).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| self.bin.clone())
    }

    /// The switches for a verified text-only run, empty for unsupported or customized command templates.
    pub fn aside(&self) -> Vec<String> {
        let original = ROWS.iter().find(|row| row.id == self.id).map(Row::agent);
        if !original.is_some_and(|a| a.run == self.run && a.run_fork == self.run_fork && a.model == self.model && a.system == self.system) {
            return Vec::new();
        }
        ASIDE.iter().find(|(id, _)| *id == self.id).map(|(_, switches)| v(switches)).unwrap_or_default()
    }

    /// Its start switches for permission mode `mode`, from the `mode` column, the mode named as its help lists it
    /// (`manual` for `default` where the help lists no `default`); None where that was not measured.
    pub fn mode_argv(&self, mode: &str) -> Option<Vec<String>> {
        self.mode_argv_listed(mode, &crate::switches::of(self, || {}).modes)
    }

    /// `mode_argv` with the modes its help lists given (`known`), rather than read from what was kept of its help.
    pub fn mode_argv_listed(&self, mode: &str, known: &[String]) -> Option<Vec<String>> {
        let listed = crate::modes::listed(crate::modes::words_of(self), mode, known);
        Some(self.mode.as_ref()?.iter().map(|a| a.replace("{mode}", &listed)).collect())
    }

    pub fn installed(&self) -> bool {
        runs(Path::new(&self.path()))
    }

    /// What to type for it in a shell: its bare name when that is what PATH finds, else its full path.
    pub fn command(&self) -> String {
        let path = self.path();
        let on_path = std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(&self.bin)).find(|p| runs(p)));
        if on_path.is_some_and(|p| p.to_string_lossy() == path) { self.bin.clone() } else { path }
    }

    /// The argv as typed in a shell: its executable as `command` gives it.
    pub fn typed(&self, argv: &[String]) -> Vec<String> {
        let mut out = argv.to_vec();
        if out.first().is_some_and(|first| *first == self.path()) {
            out[0] = self.command();
        }
        out
    }

    /// Whether a model name is one to pass it: its own, or one no agent of the table claims.
    pub fn takes_model(&self, model: &str) -> bool {
        let claims = |a: &Agent| a.models.iter().any(|m| model == m || (m.ends_with(['-', '/']) && model.starts_with(m.as_str())));
        claims(self) || !all().iter().any(claims)
    }

    /// The template filled: an argument whose placeholder has no value is dropped, and `opts` go where `{opts}` stands,
    /// else after the rest. None when `session` is not a plain id, which could otherwise be read as a switch.
    fn fill(&self, template: &[String], prompt: Option<&str>, session: Option<&str>, opts: &[String]) -> Option<Vec<String>> {
        if session.is_some_and(|s| !plain_id(s)) {
            return None;
        }
        let bin = self.path();
        let mut out: Vec<String> = Vec::new();
        let mut placed = false;
        for arg in template {
            if arg == "{opts}" {
                out.extend(opts.iter().cloned());
                placed = true;
                continue;
            }
            if (arg.contains("{prompt}") && prompt.is_none()) || (arg.contains("{session}") && session.is_none()) {
                continue;
            }
            let fill = |a: &str| a.replace("{bin}", &bin).replace("{prompt}", prompt.unwrap_or("")).replace("{session}", session.unwrap_or(""));
            // "-i {prompt}": a switch and its value, given or dropped together.
            match arg.split_once(" {") {
                Some((flag, value)) if arg.starts_with('-') && !flag.contains(' ') => out.extend([flag.to_string(), fill(&format!("{{{value}"))]),
                _ => out.push(fill(arg)),
            }
        }
        if !placed {
            out.extend(opts.iter().cloned());
        }
        Some(out)
    }

    fn with(template: &[String], key: &str, value: &str) -> Vec<String> {
        template.iter().map(|a| a.replace(key, value)).collect()
    }

    /// Its command line, less the executable, for a session cockpit drives under `session`: the stream's switches first,
    /// then the permission `mode`, what was `picked` (model, effort, fast mode), then the session (new, or `resume`d).
    /// None when it is not driven, or the mode or the resume is not known for it.
    pub fn stream_argv(&self, session: &str, mode: &str, resume: bool, picked: &Picked) -> Option<Vec<String>> {
        let mut out = self.stream.clone()?;
        out.extend(self.mode_argv(mode)?);
        out.extend(self.picked_args(picked));
        match resume {
            true => out.extend(self.resume_argv(session)?.into_iter().skip(1)),
            false => out.extend(Self::with(&self.new_session, "{session}", plain_id(session).then_some(session)?)),
        }
        Some(out)
    }

    /// Its switches for `model`, from the `model` column.
    pub fn model_argv(&self, model: &str) -> Vec<String> {
        self.model_args(Some(model))
    }

    fn model_args(&self, model: Option<&str>) -> Vec<String> {
        model.map(|m| Self::with(&self.model, "{model}", m)).unwrap_or_default()
    }

    /// Whether it takes a first message as it starts (`{prompt}` in its start template): one that does not gets its first
    /// prompt once it is up.
    pub fn start_takes_prompt(&self) -> bool {
        self.start.iter().any(|a| a.contains("{prompt}"))
    }

    /// Interactive, with a first message when given.
    pub fn start_argv(&self, prompt: Option<&str>, model: Option<&str>) -> Vec<String> {
        self.fill(&self.start, prompt, None, &self.model_args(model)).unwrap_or_default()
    }

    /// Interactive, with a first message when given, on what was picked: what it cannot take is left out.
    pub fn start_picked(&self, prompt: Option<&str>, picked: &Picked) -> Vec<String> {
        self.fill(&self.start, prompt, None, &self.picked_args(picked)).unwrap_or_default()
    }

    /// Its switches for what was picked: a model it takes, an effort level it lists, fast mode where it has one.
    pub fn picked_args(&self, picked: &Picked) -> Vec<String> {
        let mut out = picked.model.filter(|m| !m.is_empty() && self.takes_model(m)).map(|m| self.model_args(Some(m))).unwrap_or_default();
        out.extend(picked.effort.and_then(|e| self.effort_argv(e)).unwrap_or_default());
        out.extend(picked.fast.and_then(|on| self.fast_argv(on)).unwrap_or_default());
        out
    }

    /// Its start switches for effort level `effort`: None where it takes no such level or none at start.
    pub fn effort_argv(&self, effort: &str) -> Option<Vec<String>> {
        self.takes_effort(effort).then_some(())?;
        Some(Self::with(self.effort.as_ref()?, "{effort}", effort))
    }

    /// Whether `effort` is one of its levels.
    pub fn takes_effort(&self, effort: &str) -> bool {
        !effort.is_empty() && self.efforts.iter().any(|e| e == effort)
    }

    /// Its start switches for fast mode on or off; None where it has no fast mode.
    pub fn fast_argv(&self, on: bool) -> Option<Vec<String>> {
        let switches = self.fast.as_ref()?;
        let valued = switches.iter().any(|a| a.contains("{fast}") || a.contains("{speed}"));
        Some(match (valued, on) {
            (true, _) => Self::with(&Self::with(switches, "{fast}", if on { "true" } else { "false" }), "{speed}", if on { "fast" } else { "default" }),
            (false, true) => switches.clone(),
            (false, false) => vec![],
        })
    }

    /// What to type in its terminal to set `setting` to `value` (a fast mode value is "on" or "off"): Ok with the line,
    /// Err with the command the person runs there by hand, None where its terminal cannot change it.
    pub fn typed_for(&self, setting: Setting, value: &str) -> Option<Result<String, String>> {
        Some(match self.typing.of(setting)? {
            Typed::Command(c) => Ok(c.replace(&format!("{{{}}}", setting.id()), value)),
            Typed::ByHand(c) => Err(c.clone()),
        })
    }

    /// The models its table names, each as people read it ("opus" is "Opus"): what it offers until its own listing answers.
    pub fn offers(&self) -> Vec<Offer> {
        self.offered.iter().cloned().map(Offer::named).collect()
    }

    /// What a person may pick for it and how each reaches it, as the views send it: `{agent, models: [{id, label, efforts?,
    /// fast?, resolved?, description?}], efforts, fast, modes: [{id, label, …}], start: {model, effort, fast}, live:
    /// [setting], typed: {setting: {command}|{byHand}|null}}`. `models` are the ones given, else the table's; `efforts`
    /// the table's, else what its `help` lists; `modes` its table's (`modes::json`), offered as its help lists them.
    pub fn picks_json(&self, models: Option<&[Offer]>, help: &crate::switches::Help) -> Value {
        let models: Vec<Value> = match models.filter(|c| !c.is_empty()) {
            Some(listed) => listed.iter().map(Offer::json).collect(),
            None => self.offers().iter().map(Offer::json).collect(),
        };
        let efforts = if self.efforts.is_empty() { &help.efforts } else { &self.efforts };
        let typed = |s: Setting| match self.typing.of(s) {
            Some(Typed::Command(c)) => serde_json::json!({"command": c}),
            Some(Typed::ByHand(c)) => serde_json::json!({"byHand": c}),
            None => Value::Null,
        };
        let applies: serde_json::Map<String, Value> = LIVE
            .iter()
            .find(|(id, _)| *id == self.id)
            .into_iter()
            .flat_map(|(_, list)| list.iter())
            .filter(|(setting, _)| self.live.contains(setting))
            .map(|(setting, when)| (setting.id().to_string(), Value::from(*when)))
            .collect();
        serde_json::json!({
            "agent": self.name, "models": models, "efforts": efforts, "fast": self.fast.is_some(),
            "modes": crate::modes::json(crate::modes::words_of(self), &help.modes),
            "start": {"model": !self.model.is_empty(), "effort": self.effort.is_some() && !efforts.is_empty(), "fast": self.fast.is_some()},
            "live": self.live.iter().map(|s| s.id()).collect::<Vec<_>>(),
            "applies": applies,
            "typed": {"model": typed(Setting::Model), "effort": typed(Setting::Effort), "fast": typed(Setting::Fast)},
        })
    }

    /// Its picks on the models it offers everywhere (`models::of`) and what its help lists (`switches::of`); `loading`
    /// is true while its own listing is read, and `ready` is called once that or its help was read.
    pub fn catalog_picks(&self, ready: fn()) -> Value {
        let listed = crate::models::of(self, ready);
        let help = crate::switches::of(self, ready);
        let mut picks = self.picks_json(Some(&listed.offers), &help);
        picks["loading"] = Value::Bool(listed.loading);
        picks
    }
}

/// The models a catalog answer offers, in its order, each a well-formed id (`model_id`), the ones it shows but lets
/// nobody pick left out, MAX_MODELS at most. `response` is the answer to the request `CATALOG` names for the agent `id`;
/// empty for an agent whose table names none.
pub fn catalog(id: &str, response: &Value) -> Vec<Offer> {
    match CATALOG.iter().find(|(agent, _, _)| *agent == id).map(|(_, request, _)| *request) {
        Some("list_models") => list_models(response),
        _ => Vec::new(),
    }
}

/// A `list_models` answer: `{models: [{value, resolvedModel, displayName, description, supportsEffort,
/// supportedEffortLevels, supportsFastMode, disabled}]}`; `efforts` is empty for a model without levels.
fn list_models(response: &Value) -> Vec<Offer> {
    response["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["disabled"] != true)
        .filter_map(|m| {
            let id = m["value"].as_str().filter(|v| model_id(v))?.to_string();
            let label = shown_label(m["displayName"].as_str(), &id);
            let efforts = if m["supportsEffort"] == true {
                m["supportedEffortLevels"].as_array().map(|l| l.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
            } else {
                None
            };
            let resolved = m["resolvedModel"].as_str().filter(|r| model_id(r)).map(str::to_string);
            let description = m["description"].as_str().map(|d| crate::text::clean(&crate::text::without_sequences(d), 120)).filter(|d| !d.trim().is_empty());
            Some(Offer { id, label, efforts: Some(efforts.unwrap_or_default()), fast: Some(m["supportsFastMode"] == true), resolved, description })
        })
        .take(MAX_MODELS)
        .collect()
}

/// The models `codex debug models` prints that its pickers list, each with its effort levels and whether it has a fast
/// tier, each a well-formed id (`model_id`), MAX_MODELS at most; None when the text is not its catalog.
pub fn codex_catalog(text: &str) -> Option<Vec<Offer>> {
    let doc: Value = serde_json::from_str(text).ok()?;
    let models = doc["models"].as_array()?;
    let offers: Vec<Offer> = models
        .iter()
        .filter(|m| m["visibility"].as_str().is_none_or(|v| v == "list"))
        .filter_map(|m| {
            let id = m["slug"].as_str().filter(|s| model_id(s))?.to_string();
            let label = shown_label(m["display_name"].as_str(), &id);
            let efforts = m["supported_reasoning_levels"].as_array().map(|l| l.iter().filter_map(|e| e["effort"].as_str().map(str::to_string)).collect());
            let tiers = |k: &str| m[k].as_array().is_some_and(|t| !t.is_empty());
            Some(Offer { id, label, efforts, fast: Some(tiers("additional_speed_tiers") || tiers("service_tiers")), ..Offer::default() })
        })
        .take(MAX_MODELS)
        .collect();
    Some(offers)
}

/// A label an agent gave a model as one plain line of 64 characters at most (`text::clean`), else the id as people read it.
fn shown_label(label: Option<&str>, id: &str) -> String {
    let clean = crate::text::clean(&crate::text::without_sequences(label.unwrap_or_default()), 64);
    if clean.trim().is_empty() { label_of(id) } else { clean.trim().to_string() }
}

/// A model's id as people read it: an alias capitalised ("opus" is "Opus"), a full name as it is.
fn label_of(id: &str) -> String {
    match !id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase()) {
        true => id[..1].to_ascii_uppercase() + &id[1..],
        false => id.to_string(),
    }
}

impl Agent {
    pub fn resume_argv(&self, session: &str) -> Option<Vec<String>> {
        self.resume_with(session, None)
    }

    pub fn fork_argv(&self, session: &str) -> Option<Vec<String>> {
        self.fork_with(session, None)
    }

    /// Interactive on a past session, with a message when given.
    pub fn resume_with(&self, session: &str, prompt: Option<&str>) -> Option<Vec<String>> {
        (!self.resume.is_empty()).then(|| self.fill(&self.resume, prompt.filter(|p| !p.is_empty()), Some(session), &[])).flatten()
    }

    /// Interactive on a new branch of a past session, with a first message when given.
    pub fn fork_with(&self, session: &str, prompt: Option<&str>) -> Option<Vec<String>> {
        (!self.fork.is_empty()).then(|| self.fill(&self.fork, prompt.filter(|p| !p.is_empty()), Some(session), &[])).flatten()
    }

    pub fn can_run(&self) -> bool {
        !self.run.is_empty()
    }

    /// The models only a routine of it takes, beside the ones it offers everywhere.
    pub fn routine_only(&self) -> Vec<String> {
        ROUTINE_ONLY.iter().find(|(id, _)| *id == self.id).map(|(_, models)| v(models)).unwrap_or_default()
    }

    /// The command that lists the models it can use, and how its answer reads; None when it has none. A `Probe` is the
    /// stream opened for its catalog request (`probe_argv`).
    pub fn model_listing(&self) -> Option<(Vec<String>, Listing)> {
        let (_, args, how) = MODEL_LIST.iter().find(|(id, _, _)| *id == self.id)?;
        if *how == Listing::Probe {
            return Some((self.probe_argv()?, Listing::Probe));
        }
        Some((std::iter::once(self.path()).chain(args.iter().map(|a| a.to_string())).collect(), *how))
    }

    /// Its command line for one catalog request over its stream: the executable and the switches `CATALOG` names, none
    /// of a session's (no permission mode, no replay, no session id). None where its table names no request.
    pub fn probe_argv(&self) -> Option<Vec<String>> {
        let (_, _, switches) = CATALOG.iter().find(|(id, _, _)| *id == self.id)?;
        Some(std::iter::once(self.path()).chain(switches.iter().map(|a| a.to_string())).collect())
    }

    /// The control request that answers with its catalog over its stream (`list_models`); None where none is known.
    pub fn catalog_request(&self) -> Option<&'static str> {
        CATALOG.iter().find(|(id, _, _)| *id == self.id).map(|(_, request, _)| *request)
    }

    /// Its command line printing its version, from the `VERSIONED` table; None where not known.
    pub fn version_argv(&self) -> Option<Vec<String>> {
        let (_, switches) = VERSIONED.iter().find(|(id, _)| *id == self.id)?;
        Some(std::iter::once(self.path()).chain(switches.iter().map(|a| a.to_string())).collect())
    }

    /// An unattended run, None when the agent has no headless mode or cannot fork when asked to. Switches it lacks are
    /// left out, and instructions it cannot take as a system prompt go before the prompt.
    pub fn headless(&self, prompt: &str, how: &Headless) -> Option<Vec<String>> {
        let template = if how.fork.is_some() { &self.run_fork } else { &self.run };
        if template.is_empty() {
            return None;
        }
        let mut opts = self.model_args(how.model.filter(|m| self.takes_model(m)));
        opts.extend(how.effort.and_then(|e| self.effort_argv(e)).unwrap_or_default());
        if how.aside {
            let isolated = self.aside();
            if isolated.is_empty() || how.edits || !how.dirs.is_empty() {
                return None;
            }
            opts.extend(isolated);
        }
        if how.edits {
            opts.extend(self.edits.iter().cloned());
        }
        if !self.add_dir.is_empty() {
            for dir in &how.dirs {
                opts.extend(Self::with(&self.add_dir, "{dir}", dir));
            }
        }
        let mut prompt = prompt.to_string();
        if let Some(text) = how.system.filter(|t| !t.is_empty()) {
            if self.system.is_empty() {
                prompt = format!("{text}\n\n{prompt}");
            } else {
                opts.extend(Self::with(&self.system, "{text}", text));
            }
        }
        self.fill(template, Some(&prompt), how.fork, &opts)
    }
}

/// Whether a model name is well formed, so it can be typed after a command as one word: letters and digits first, then
/// those or `.-_:/@+[]`, at most 128 characters.
pub fn model_id(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.len() <= 128
        && name.chars().all(|c| c.is_ascii_alphanumeric() || ".-_:/@+[]".contains(c))
}

/// Whether a session id is a plain one: letters, digits, `-` and `_`, not starting with `-`, so no agent reads it as a
/// switch.
pub fn plain_id(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('-') && id.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
}

/// The session id a headless run printed on a `session id: <id>` line, as Codex does on stderr.
pub fn session_in(output: &str) -> Option<String> {
    output.lines().find_map(|l| {
        let (key, id) = l.split_once(':')?;
        let id = id.trim();
        (key.trim().eq_ignore_ascii_case("session id") && plain_id(id)).then(|| id.to_string())
    })
}

/// An argv as one shell command line, each argument single-quoted, for terminals that take a line of text.
pub fn shell_line(argv: &[String]) -> String {
    argv.iter()
        .map(|a| typable(a))
        .map(|a| if a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@".contains(c)) { a } else { format!("'{}'", a.replace('\'', r"'\''")) })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Text that can be typed into an interactive shell as a quoted word: control characters but the newline dropped (a
/// tab a space), since a line editor acts on them even inside quotes.
pub fn typable(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c == '\t' {
                Some(' ')
            } else if c.is_control() && c != '\n' {
                None
            } else {
                Some(c)
            }
        })
        .collect()
}

/// How an agent reports its plan limits: the names the usage surfaces show it by, each source `usage` reads with the
/// words for where a reading comes from, and how its account is told. An agent missing from `REPORTS` reports none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reports {
    pub short: &'static str,
    pub letter: &'static str,
    /// (source id, where a reading comes from, as a person says it).
    pub sources: &'static [(&'static str, &'static str)],
    /// The account line: `{account}` and `{plan}` filled in from what its own files say.
    pub account: &'static str,
    /// The account line when those files say it is signed out.
    pub signed_out: &'static str,
    /// The account line of one of several accounts on this machine, `{folder}` its config folder.
    pub account_in: &'static str,
    /// That line when the folder's files say it is signed out.
    pub signed_out_in: &'static str,
}

const REPORTS: &[(&str, Reports)] = &[
    (
        "claude",
        Reports {
            short: "Claude",
            letter: "C",
            sources: &[("claude-statusline", "Claude Code's status line"), ("claude-stream", "a session cockpit drives")],
            account: "This machine's Claude Code: {account}",
            signed_out: "This machine's Claude Code is not signed in to a Claude account",
            account_in: "Claude Code in {folder}: {account}",
            signed_out_in: "Claude Code in {folder} is not signed in to a Claude account",
        },
    ),
    (
        "codex",
        Reports {
            short: "Codex",
            letter: "X",
            sources: &[("codex-rollout", "Codex's session log")],
            account: "ChatGPT {plan} plan",
            signed_out: "",
            account_in: "",
            signed_out_in: "",
        },
    ),
    // GitHub's own quota of the Copilot plan, read through `gh` with the sign-in it already has.
    (
        "copilot",
        Reports {
            short: "Copilot",
            letter: "G",
            sources: &[("copilot-gh", "GitHub, through gh")],
            account: "GitHub Copilot {plan} plan",
            signed_out: "",
            account_in: "",
            signed_out_in: "",
        },
    ),
];

/// What the agent with this id reports of its limits, if anything.
pub fn reports(id: &str) -> Option<Reports> {
    REPORTS.iter().find(|(a, _)| *a == id).map(|(_, r)| *r)
}

/// The agent a usage source belongs to.
pub fn reporting(source: &str) -> Option<&'static str> {
    REPORTS.iter().find(|(_, r)| r.sources.iter().any(|(id, _)| *id == source)).map(|(a, _)| *a)
}

/// Where a reading of this source comes from, as a person says it.
pub fn source_words(source: &str) -> Option<&'static str> {
    REPORTS.iter().flat_map(|(_, r)| r.sources.iter()).find(|(id, _)| *id == source).map(|(_, words)| *words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::switches::Help;

    fn row(id: &str) -> Agent {
        let mut a = builtin().into_iter().find(|a| a.id == id).unwrap();
        a.bin = format!("/bin/{}", a.bin);
        a
    }

    /// A plain headless run on a model, as every unattended run is built.
    fn run(a: &Agent, prompt: &str, model: Option<&str>) -> Option<Vec<String>> {
        a.headless(prompt, &Headless { model, ..Default::default() })
    }

    #[test]
    fn templates_drop_what_is_not_given() {
        let c = row("codex");
        assert_eq!(c.start_argv(None, None), ["/bin/codex"]);
        assert_eq!(c.start_argv(Some("fix it"), None), ["/bin/codex", "fix it"]);
        assert_eq!(run(&c, "hi", Some("o3")).unwrap(), ["/bin/codex", "exec", "--skip-git-repo-check", "--model", "o3", "hi"]);
        assert_eq!(c.fork_argv("abc").unwrap(), ["/bin/codex", "fork", "abc"]);
        assert_eq!(c.fork_with("abc", Some("look")).unwrap(), ["/bin/codex", "fork", "abc", "look"]);
        assert_eq!(c.resume_with("abc", Some("")).unwrap(), ["/bin/codex", "resume", "abc"]);
    }

    #[test]
    fn screen_shapes_compile_and_read_each_agents_own_lines() {
        let matches = |list: &[String], line: &str| list.iter().any(|p| regex::Regex::new(p).expect(p).is_match(line));
        let claude = screen("claude");
        for line in [
            "⏵⏵ auto mode on (shift+tab to cycle)",
            "⏵⏵ accept edits on (shift+tab to cycle)",
            "⏸ plan mode on (shift+tab to cycle)",
            "⏵⏵ bypass permissions on (shift+tab to cycle)",
            "plan mode on",
            "bypass permissions on",
            "? for shortcuts",
        ] {
            assert!(matches(&claude.footer, line), "{line}");
        }
        assert!(!matches(&claude.footer, "Should plan mode stay on for the migration?"));
        for line in ["✻ Thinking… (12s · esc to interrupt)", "✻ Crunched for 21m 49s", "✽ Worked for 8s"] {
            assert!(matches(&claude.busy, line), "{line}");
        }
        for line in [
            "⏺ Usage limit reached · continuing automatically at 8am · esc to cancel",
            "⎿  You've hit your limit · resets 8am (Europe/Paris)",
            "5-hour limit reached ∙ resets 3pm",
            "API Error: 429 rate_limit_error",
        ] {
            assert!(matches(&claude.limit, line), "{line}");
        }
        // Words about a limit, a footer or a timer in what the agent says are none of those.
        assert!(!matches(&claude.limit, "The rate limit is exceeded at 101 requests"));
        assert!(!matches(&claude.footer, "Plan mode on staging?") && !matches(&claude.busy, "* Wait for 2 days?"));
        assert!(matches(&claude.lifted, "⏺ Usage limit has reset · Press enter to continue"));
        assert_eq!(screen("claude-code"), claude, "by an alias");
        let codex = screen("codex");
        assert!(matches(&codex.footer, "? for shortcuts") && matches(&codex.footer, "41% context left"));
        assert!(matches(&codex.limit, "You've hit your usage limit. Try again at 3:14 PM."));
        // An agent with no row of its own, and a program the table does not know, read with every agent's furniture.
        for other in ["gemini", "some-other-agent", ""] {
            let all = screen(other);
            assert!(matches(&all.footer, "⏵⏵ auto mode on (shift+tab to cycle)") && all.limit.is_empty() && all.lifted.is_empty(), "{other}");
        }
    }

    #[test]
    fn permission_patterns_that_do_not_compile_never_turn_detection_off() {
        let overrides = serde_json::json!({"claude": {"permission": ["(unclosed", "[z-a]"]}, "codex": {"permission": ["^Run it\\?", "(bad"]}});
        let list = laid_over(builtin(), overrides.as_object().unwrap());
        let of = |id: &str| list.iter().find(|a| a.id == id).unwrap().permission.clone();
        assert_eq!(of("claude"), row("claude").permission, "none compiles: the measured ones stay");
        assert_eq!(of("codex"), ["^Run it\\?"], "the ones that compile replace them");
        assert_eq!(unreadable_in(&overrides).len(), 3, "doctor names each one left out");
    }

    #[test]
    fn a_driven_session_s_command_line_comes_from_the_table() {
        let claude = row("claude");
        let id = "00000000-acme-4000-8000-000000000421";
        let none = Picked::default();
        let haiku = Picked { model: Some("claude-haiku-5-5"), ..Picked::default() };
        let new = claude.stream_argv(id, "plan", false, &none).unwrap();
        assert!(new.starts_with(&["-p".to_string(), "--input-format".into(), "stream-json".into()]), "{new:?}");
        assert!(new.ends_with(&["--permission-mode".to_string(), "plan".into(), "--session-id".into(), id.into()]), "{new:?}");
        let back = claude.stream_argv(id, "default", true, &haiku).unwrap();
        assert!(back.ends_with(&["--model".to_string(), "claude-haiku-5-5".into(), "--resume".into(), id.into()]), "{back:?}");
        assert_eq!(row("codex").stream_argv(id, "default", false, &none), None, "not driven");
        assert_eq!(claude.stream_argv(id, "default", true, &none).unwrap().iter().filter(|a| *a == "--model").count(), 0);
        let picked = claude.stream_argv(id, "default", false, &haiku).unwrap();
        assert!(picked.ends_with(&["--model".to_string(), "claude-haiku-5-5".into(), "--session-id".into(), id.into()]), "a new session keeps it: {picked:?}");
        let all = Picked { model: Some("haiku"), effort: Some("high"), fast: Some(true) };
        let fast = claude.stream_argv(id, "default", true, &all).unwrap();
        assert!(
            fast.ends_with(&v(&["--model", "haiku", "--effort", "high", "--settings", "{\"fastMode\":true}", "--resume", id])),
            "resumed with what was picked: {fast:?}"
        );
    }

    #[test]
    fn each_agent_offers_the_models_effort_levels_and_fast_mode_it_takes() {
        let claude = row("claude");
        assert_eq!(claude.offers().iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), ["Opus", "Sonnet", "Haiku", "Fable"]);
        assert_eq!(claude.efforts, ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(claude.effort_argv("xhigh").unwrap(), ["--effort", "xhigh"]);
        assert_eq!(claude.effort_argv("ultra"), None, "not one of its levels");
        assert_eq!(
            (claude.fast_argv(true).unwrap(), claude.fast_argv(false).unwrap()),
            (v(&["--settings", "{\"fastMode\":true}"]), v(&["--settings", "{\"fastMode\":false}"]))
        );
        let codex = row("codex");
        assert!(codex.offers().is_empty() && codex.model_listing().is_some_and(|(_, how)| how == Listing::Codex), "its listing names its models");
        assert_eq!(codex.effort_argv("ultra").unwrap(), ["-c", "model_reasoning_effort=\"ultra\""]);
        assert_eq!(
            (codex.fast_argv(true).unwrap(), codex.fast_argv(false).unwrap()),
            (v(&["-c", "service_tier=\"fast\""]), v(&["-c", "service_tier=\"default\""])),
            "standard overrides a configured fast tier"
        );
        let picked = Picked { model: Some("gpt-acme-1"), effort: Some("high"), fast: Some(true) };
        assert_eq!(
            codex.start_picked(Some("fix it"), &picked),
            ["/bin/codex", "fix it", "--model", "gpt-acme-1", "-c", "model_reasoning_effort=\"high\"", "-c", "service_tier=\"fast\""]
        );
        assert_eq!(
            claude.start_picked(None, &Picked { model: Some("gpt-acme-1"), effort: Some("ultra"), fast: None }),
            ["/bin/claude"],
            "what it cannot take is left out"
        );
        for a in builtin().into_iter().filter(|a| a.id != "claude" && a.id != "codex") {
            assert!(a.efforts.is_empty() && a.effort.is_none() && a.fast.is_none() && a.live.is_empty(), "{} was not measured", a.id);
            assert_eq!(a.typing, Typing::default(), "{}", a.id);
            assert!(a.picks_json(None, &Help::default())["efforts"].as_array().unwrap().is_empty(), "{} shows no effort picker", a.id);
        }
    }

    #[test]
    fn standard_speed_overrides_an_inherited_fast_tier() {
        let codex = row("codex");
        let inherited = "service_tier=\"fast\"";
        let effective = |picked: Option<bool>| {
            let args = codex.picked_args(&Picked { fast: picked, ..Default::default() });
            args.windows(2).rfind(|w| w[0] == "-c" && w[1].starts_with("service_tier=")).map(|w| w[1].clone()).unwrap_or_else(|| inherited.into())
        };
        assert_eq!(effective(None), inherited, "its default inherits the person's config");
        assert_eq!(effective(Some(true)), inherited, "Fast explicitly requests the fast tier");
        assert_eq!(effective(Some(false)), "service_tier=\"default\"", "Standard explicitly overrides the configured fast tier");
    }

    #[test]
    fn a_setting_changes_live_in_a_driven_session_or_by_its_own_command_in_a_terminal() {
        let claude = row("claude");
        assert_eq!(claude.live, [Setting::Model, Setting::Effort, Setting::Fast]);
        assert_eq!(claude.picks_json(None, &Help::default())["applies"], serde_json::json!({"model": "call", "effort": "turn", "fast": "turn"}));
        assert_eq!(claude.typed_for(Setting::Model, "sonnet"), Some(Ok("/model sonnet".into())));
        assert_eq!(claude.typed_for(Setting::Effort, "max"), Some(Ok("/effort max".into())));
        assert_eq!(claude.typed_for(Setting::Fast, "off"), Some(Ok("/fast off".into())));
        let codex = row("codex");
        assert!(codex.live.is_empty(), "not driven");
        assert_eq!(codex.typed_for(Setting::Effort, "high"), Some(Ok("/model --reasoning=high".into())));
        assert_eq!(codex.typed_for(Setting::Fast, "on"), Some(Err("/fast".into())), "a toggle: changed by hand");
        assert_eq!(row("opencode").typed_for(Setting::Model, "x"), None);
        let json = codex.picks_json(None, &Help::default());
        assert_eq!((json["typed"]["fast"]["byHand"].as_str(), json["typed"]["model"]["command"].as_str()), (Some("/fast"), Some("/model {model}")));
        assert_eq!(json["start"], serde_json::json!({"model": true, "effort": true, "fast": true}));
    }

    #[test]
    fn codex_s_catalog_lists_the_models_its_picker_shows() {
        let text = serde_json::json!({"models": [
            {"slug": "gpt-acme-1", "display_name": "GPT Acme 1", "visibility": "list", "supported_reasoning_levels": [{"effort": "low"}, {"effort": "high"}],
             "additional_speed_tiers": ["fast"], "service_tiers": [{"id": "priority", "name": "Fast"}]},
            {"slug": "gpt-acme-mini", "display_name": "", "visibility": "list", "supported_reasoning_levels": [{"effort": "medium"}], "additional_speed_tiers": []},
            {"slug": "acme-review", "display_name": "Review", "visibility": "hide"},
        ]})
        .to_string();
        let offers = codex_catalog(&text).unwrap();
        assert_eq!(
            offers.iter().map(|o| (o.id.as_str(), o.label.as_str())).collect::<Vec<_>>(),
            [("gpt-acme-1", "GPT Acme 1"), ("gpt-acme-mini", "gpt-acme-mini")]
        );
        assert_eq!((offers[0].efforts.clone().unwrap(), offers[0].fast, offers[1].fast), (v(&["low", "high"]), Some(true), Some(false)));
        assert_eq!(codex_catalog("not json"), None);
        let json = row("codex").picks_json(Some(&offers), &Help::default());
        assert_eq!(json["models"][0], serde_json::json!({"id": "gpt-acme-1", "label": "GPT Acme 1", "efforts": ["low", "high"], "fast": true}));
    }

    const LIST_MODELS: &str = include_str!("../tests/fixtures/claude-list-models.json");

    #[test]
    fn claude_s_catalog_is_its_list_models_answer_each_model_with_what_it_supports() {
        let event: Value = serde_json::from_str(LIST_MODELS).unwrap();
        let offers = catalog("claude", &event["response"]["response"]);
        assert_eq!(offers.len(), 14);
        let by = |id: &str| offers.iter().find(|o| o.id == id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!((by("default").label.as_str(), by("default").resolved.as_deref()), ("Default (recommended)", Some("claude-fable-5-1")));
        assert_eq!((by("opus").fast, by("sonnet").fast), (Some(true), Some(false)), "only some have a fast mode");
        assert_eq!(by("opus").efforts.as_deref(), Some(&v(&["low", "medium", "high", "xhigh", "max"])[..]));
        assert_eq!(by("claude-haiku-4-5-20251001").efforts.as_deref(), Some(&[][..]), "no effort levels");
        assert_eq!(by("claude-opus-4-6").efforts.as_deref().map(|e| e.len()), Some(4));
        assert_eq!(by("haiku").description.as_deref(), Some("Fastest for quick answers"));
        // As the views read it: the shape a driven session's `models` had, plus the description.
        let json = by("opus").json();
        assert_eq!(json["id"], "opus");
        assert_eq!((json["fast"].as_bool(), json["resolved"].as_str()), (Some(true), Some("claude-opus-5-5")));
        assert_eq!(json["efforts"].as_array().map(|e| e.len()), Some(5));
        assert_eq!(Offer::of_json(&json), Some(by("opus").clone()), "kept and read back whole");
        assert!(catalog("codex", &event["response"]["response"]).is_empty(), "no catalog request in its table");
        let disabled = serde_json::json!({"models": [{"value": "opus", "disabled": true}, {"value": "-p"}, {"value": "haiku", "displayName": "Haiku"}]});
        assert_eq!(catalog("claude", &disabled).iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["haiku"], "disabled and switch-like values left out");
    }

    #[test]
    fn claude_s_catalog_is_probed_over_its_stream_and_kept_by_its_version() {
        let claude = row("claude");
        let (argv, how) = claude.model_listing().unwrap();
        assert_eq!(how, Listing::Probe);
        assert_eq!(argv, ["/bin/claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--no-session-persistence"]);
        assert!(!argv.iter().any(|a| a.contains("permission") || a.contains("session-id") || a.contains("replay")), "none of a session's switches: {argv:?}");
        assert_eq!(claude.catalog_request(), Some("list_models"));
        assert_eq!(claude.version_argv().unwrap(), ["/bin/claude", "--version"]);
        assert_eq!((row("codex").catalog_request(), row("codex").probe_argv()), (None, None));
        assert_eq!(row("pi").version_argv(), None, "not known: kept a day instead");
        assert_eq!(listed_models("{}", Listing::Probe), None, "a probe is not text");
        assert!(
            CATALOG
                .iter()
                .chain(VERSIONED.iter().map(|(id, s)| (*id, "", *s)).collect::<Vec<_>>().iter())
                .all(|(id, _, _)| builtin().iter().any(|a| a.id == *id))
        );
    }

    #[test]
    fn a_mode_is_passed_as_the_help_lists_it_and_the_picks_carry_every_mode() {
        let claude = row("claude");
        let help = Help {
            modes: v(&["acceptEdits", "auto", "bypassPermissions", "manual", "dontAsk", "plan"]),
            efforts: v(&["low", "medium", "high", "xhigh", "max"]),
        };
        assert_eq!(claude.mode_argv_listed("default", &help.modes).unwrap(), ["--permission-mode", "manual"], "this version lists manual, not default");
        assert_eq!(claude.mode_argv_listed("plan", &help.modes).unwrap(), ["--permission-mode", "plan"]);
        assert_eq!(claude.mode_argv_listed("default", &[]).unwrap(), ["--permission-mode", "default"], "no help read yet");
        assert_eq!(row("codex").mode_argv_listed("read-only", &[]), None, "no one mode switch");
        let picks = claude.picks_json(None, &help);
        let modes = picks["modes"].as_array().unwrap();
        assert_eq!(
            modes.iter().map(|m| m["id"].as_str().unwrap()).collect::<Vec<_>>(),
            ["manual", "acceptEdits", "plan", "bypassPermissions", "auto", "dontAsk"]
        );
        assert_eq!((modes[0]["label"].as_str(), modes[0]["offered"].as_bool(), modes[4]["offered"].as_bool()), (Some("Asks first"), Some(true), Some(false)));
        assert_eq!(picks["efforts"], serde_json::json!(["low", "medium", "high", "xhigh", "max"]), "the table's");
        let mut bare = claude.clone();
        bare.efforts.clear();
        let fallen = bare.picks_json(None, &Help { modes: vec![], efforts: v(&["low", "high"]) });
        assert_eq!(
            (fallen["efforts"].clone(), fallen["start"]["effort"].clone()),
            (serde_json::json!(["low", "high"]), serde_json::json!(true)),
            "else the help's"
        );
        assert_eq!(row("pi").picks_json(None, &Help::default())["modes"], serde_json::json!([]), "no modes known");
    }

    #[test]
    fn the_config_lays_picks_over_the_table() {
        let overrides = serde_json::json!({"claude": {"offered": ["opus"], "efforts": ["low", "high"], "fast": [], "live": ["model", "speed"]}, "acme": {"efforts": ["quick"], "effort": ["--think", "{effort}"]}});
        let list = laid_over(builtin(), overrides.as_object().unwrap());
        let claude = list.iter().find(|a| a.id == "claude").unwrap();
        assert_eq!(
            (claude.offered.clone(), claude.efforts.clone(), claude.fast.clone()),
            (v(&["opus"]), v(&["low", "high"]), None),
            "an empty list says it has none"
        );
        assert_eq!(claude.live, [Setting::Model], "a name it does not know is left out");
        let acme = list.iter().find(|a| a.id == "acme").unwrap();
        assert_eq!(acme.effort_argv("quick").unwrap(), ["--think", "quick"]);
        assert_eq!(claude.light.as_deref(), Some("haiku"), "measured");
        let light = laid_over(builtin(), serde_json::json!({"codex": {"light": "gpt-6-luna"}}).as_object().unwrap());
        assert_eq!(light.iter().find(|a| a.id == "codex").unwrap().light.as_deref(), Some("gpt-6-luna"));
    }

    #[test]
    fn mode_and_interrupt_are_filled_only_where_measured() {
        let claude = row("claude");
        assert_eq!(claude.mode_argv("plan").unwrap(), ["--permission-mode", "plan"]);
        assert_eq!(claude.interrupt.as_deref(), Some(&["esc".to_string()][..]));
        let codex = row("codex");
        assert_eq!((codex.mode_argv("plan"), codex.interrupt.as_deref()), (None, Some(&["esc".to_string()][..])), "esc measured, no one mode");
        for a in builtin().into_iter().filter(|a| a.id != "claude" && a.id != "codex") {
            assert_eq!((a.mode.as_ref(), a.interrupt.as_ref()), (None, None), "{} was not measured", a.id);
        }
        assert!(CONTROLS.iter().all(|(id, _, _, _)| builtin().iter().any(|a| a.id == *id)), "every measured agent has a row");
    }

    #[test]
    fn every_agent_herdr_integrates_has_a_row() {
        let ids: Vec<String> = builtin().into_iter().map(|a| a.id).collect();
        let herdr = [
            "pi",
            "omp",
            "claude",
            "codex",
            "copilot",
            "devin",
            "droid",
            "kimi",
            "opencode",
            "kilo",
            "hermes",
            "qodercli",
            "qwen",
            "letta",
            "cursor",
            "mastracode",
            "antigravity-cli",
            "grok",
        ];
        for id in herdr {
            assert!(ids.iter().any(|i| i == id), "{id}");
        }
        assert!(builtin().iter().all(|a| a.start.first().is_some_and(|b| b == "{bin}")), "every row starts interactive");
    }

    /// Herdr 0.9.5's agent kinds, as its panes report them (`agent`): each one is a row of the table.
    #[test]
    fn every_kind_herdr_detects_is_an_agent_of_the_table() {
        let kinds = [
            "pi",
            "claude",
            "codex",
            "gemini",
            "cursor",
            "devin",
            "agy",
            "cline",
            "omp",
            "mastracode",
            "opencode",
            "copilot",
            "kimi",
            "kiro",
            "droid",
            "amp",
            "grok",
            "hermes",
            "kilo",
            "qodercli",
            "qwen",
            "letta",
            "maki",
            "muse",
        ];
        let table = builtin();
        for kind in kinds {
            let p = kind.to_ascii_lowercase();
            let found = in_table(&table, &p);
            assert!(found.is_some(), "{kind} has no row");
        }
        let unique: std::collections::HashSet<&str> = table.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(unique.len(), table.len(), "one row per agent");
        assert!(table.iter().all(|a| a.can_run() && !a.resume.is_empty()), "every agent runs headless and resumes");
    }

    #[test]
    fn the_new_rows_start_run_and_resume_as_their_own_help_says() {
        assert_eq!(row("gemini").start_argv(Some("go"), Some("flash")), ["/bin/gemini", "-i", "go", "-m", "flash"]);
        assert_eq!(row("gemini").resume_with("a1b2", Some("go")).unwrap(), ["/bin/gemini", "--resume", "a1b2"], "a resume takes no first message");
        assert_eq!(run(&row("cline"), "go", None).unwrap(), ["/bin/cline", "go"]);
        assert_eq!(row("cline").start_argv(None, None), ["/bin/cline"]);
        assert_eq!(run(&row("kiro"), "go", Some("m")).unwrap(), ["/bin/kiro-cli", "chat", "--no-interactive", "--model", "m", "go"]);
        assert_eq!(row("kiro").resume_argv("3f2a").unwrap(), ["/bin/kiro-cli", "chat", "--resume-id", "3f2a"]);
        assert_eq!(row("amp").start_argv(Some("go"), None), ["/bin/amp"], "Amp takes a first message only on its input");
        assert_eq!(run(&row("amp"), "go", None).unwrap(), ["/bin/amp", "-x", "go"]);
        assert_eq!(row("amp").resume_argv("T-1").unwrap(), ["/bin/amp", "threads", "continue", "T-1"]);
        assert_eq!(
            row("maki").headless("go", &Headless { fork: Some("s1"), ..Default::default() }).unwrap(),
            ["/bin/maki", "-p", "--resume", "s1", "--fork-session", "go"]
        );
        assert_eq!(run(&row("muse"), "go", Some("muse-spark-1.2")).unwrap(), ["/bin/muse", "exec", "--model", "muse-spark-1.2", "go"]);
        assert!(row("cline").fork_argv("s1").is_none() && row("muse").fork_argv("s1").is_none() && row("amp").fork_argv("s1").is_none());
        for kind in ["kiro-cli", "muse-code", "gemini"] {
            assert!(of_program(kind).is_some(), "{kind}");
        }
        assert!(row("gemini").takes_model("gemini-3-pro") && !row("claude").takes_model("flash"));
    }

    #[test]
    fn an_unattended_run_gets_each_agent_s_own_edit_folder_and_instruction_switches() {
        let how = Headless { edits: true, dirs: vec!["/rb".into()], system: Some("Be brief."), ..Default::default() };
        let tail = |id: &str| row(id).headless("Go.", &how).unwrap()[1..].to_vec();
        assert_eq!(tail("qwen"), ["-p", "Go.", "--approval-mode", "auto-edit", "--include-directories", "/rb", "--append-system-prompt", "Be brief."]);
        assert_eq!(tail("gemini"), ["-p", "Be brief.\n\nGo.", "--approval-mode", "auto_edit", "--include-directories", "/rb"]);
        assert_eq!(tail("copilot"), ["-p", "Be brief.\n\nGo.", "--allow-tool=write", "--add-dir=/rb"]);
        assert_eq!(tail("droid"), ["exec", "Go.", "--auto", "low", "--append-system-prompt", "Be brief."]);
        assert_eq!(tail("kimi"), ["--print", "-p", "Be brief.\n\nGo.", "--add-dir", "/rb"]);
        assert_eq!(tail("pi"), ["-p", "Go.", "--append-system-prompt", "Be brief."], "pi asks before nothing: no edits switch");
        assert_eq!(tail("kiro"), ["chat", "--no-interactive", "--trust-tools=write", "Be brief.\n\nGo."]);
        assert_eq!(tail("grok"), ["-p", "Go.", "--permission-mode", "acceptEdits", "--rules", "Be brief."]);
    }

    #[test]
    fn only_agents_with_a_listing_command_list_their_models() {
        assert_eq!(row("opencode").model_listing().unwrap(), (vec!["/bin/opencode".to_string(), "models".into()], Listing::Lines));
        assert_eq!(row("omp").model_listing().unwrap().0[1..], ["models", "--json"]);
        assert!(row("claude").model_listing().is_some_and(|(_, how)| how == Listing::Probe) && row("copilot").model_listing().is_none());
        assert!(MODEL_LIST.iter().all(|(id, _, _)| builtin().iter().any(|a| a.id == *id)), "every listing has a row");
        assert!(ASIDE.iter().all(|(id, _)| builtin().iter().any(|a| a.id == *id)));
        assert!(row("codex").aside().is_empty() && row("pi").aside().is_empty(), "no verified text-only restrictions");
        assert!(!row("claude").aside().is_empty());
    }

    #[test]
    fn each_agent_starts_resumes_and_forks_its_own_way() {
        assert_eq!(row("claude").fork_with("s1", Some("go")).unwrap(), ["/bin/claude", "--resume", "s1", "--fork-session", "go"]);
        assert_eq!(row("opencode").start_argv(Some("go"), Some("anthropic/x")), ["/bin/opencode", "--prompt", "go", "-m", "anthropic/x"]);
        assert_eq!(row("opencode").fork_with("ses_1", Some("go")).unwrap(), ["/bin/opencode", "--session", "ses_1", "--fork", "--prompt", "go"]);
        assert_eq!(row("copilot").resume_argv("s1").unwrap(), ["/bin/copilot", "--resume=s1"]);
        assert_eq!(row("copilot").start_argv(Some("go"), None), ["/bin/copilot", "-i", "go"]);
        assert!(row("copilot").fork_argv("s1").is_none(), "Copilot forks only from inside a session");
        assert_eq!(row("cursor").start_argv(Some("go"), None), ["/bin/cursor-agent", "go"]);
        assert_eq!(row("antigravity-cli").resume_with("c1", Some("go")).unwrap(), ["/bin/agy", "--conversation", "c1", "-i", "go"]);
        assert_eq!(run(&row("droid"), "go", Some("m")).unwrap(), ["/bin/droid", "exec", "go", "-m", "m"]);
        assert_eq!(row("kimi").start_argv(Some("go"), None), ["/bin/kimi"], "Kimi takes no first message");
        for id in ["kimi", "letta", "grok", "amp", "muse"] {
            assert!(!row(id).start_takes_prompt(), "{id}");
        }
        assert!(row("claude").start_takes_prompt() && row("codex").start_takes_prompt());
        assert_eq!(of_program("agy").unwrap().id, "antigravity-cli");
        assert_eq!(of_program("cursor-agent").unwrap().id, "cursor");
        assert!(of_program("agent").is_none());
    }

    #[test]
    fn a_headless_run_gets_what_the_agent_takes() {
        let how =
            Headless { model: Some("sonnet"), effort: None, fork: Some("s1"), edits: true, dirs: vec!["/rb".into()], system: Some("Be brief."), aside: false };
        let claude = row("claude").headless("Go.", &how).unwrap();
        assert_eq!(
            claude,
            [
                "/bin/claude",
                "-p",
                "--resume",
                "s1",
                "--fork-session",
                "Go.",
                "--model",
                "sonnet",
                "--permission-mode",
                "acceptEdits",
                "--add-dir",
                "/rb",
                "--append-system-prompt",
                "Be brief."
            ]
        );
        let codex = row("codex").headless("Go.", &how).unwrap();
        assert_eq!(
            codex,
            ["/bin/codex", "exec", "--skip-git-repo-check", "--sandbox", "workspace-write", "--add-dir", "/rb", "fork", "s1", "Be brief.\n\nGo."],
            "never --model sonnet"
        );
        let plain = row("codex").headless("Go.", &Headless { model: Some("gpt-5-mini"), ..Default::default() }).unwrap();
        let aside = Headless { fork: Some("s1"), aside: true, ..Default::default() };
        assert_eq!(
            row("claude").headless("Go.", &aside).unwrap(),
            [
                "/bin/claude",
                "-p",
                "--resume",
                "s1",
                "--fork-session",
                "Go.",
                "--no-session-persistence",
                "--setting-sources",
                "",
                "--safe-mode",
                "--tools",
                "",
                "--strict-mcp-config",
                "--disable-slash-commands",
                "--no-chrome",
                "--permission-mode",
                "plan",
                "--max-turns",
                "1"
            ],
            "a fork kept aside: no tools, customizations, MCP or session writes"
        );
        assert!(row("codex").headless("Go.", &aside).is_none(), "read-only alone does not remove tools or MCP");
        assert!(row("opencode").headless("Go.", &aside).is_none(), "unverified isolation never falls back to an ordinary run");
        assert_eq!(plain, ["/bin/codex", "exec", "--skip-git-repo-check", "--model", "gpt-5-mini", "Go."]);
        assert!(row("copilot").headless("Go.", &how).is_none(), "Copilot cannot branch headless");
        assert_eq!(row("copilot").headless("Go.", &Headless::default()).unwrap(), ["/bin/copilot", "-p", "Go."]);
    }

    #[test]
    fn isolated_runs_fail_closed_for_every_unverified_or_customized_row() {
        let isolated = Headless { aside: true, ..Default::default() };
        for agent in builtin() {
            assert_eq!(agent.headless("Do nothing", &isolated).is_some(), agent.id == "claude", "{}", agent.id);
        }
        let mut customized = row("claude");
        customized.run = v(&["{bin}", "-p", "{prompt}"]);
        customized.run_fork.push("--tools=default".into());
        assert!(customized.headless("Do nothing", &isolated).is_none(), "a modified template is not verified");
        assert!(row("claude").headless("Do nothing", &Headless { edits: true, ..isolated.clone() }).is_none());
        assert!(row("claude").headless("Do nothing", &Headless { dirs: vec!["/r/api".into()], ..isolated }).is_none());
    }

    #[test]
    fn a_model_goes_only_to_the_agent_it_belongs_to() {
        assert!(row("claude").takes_model("haiku") && !row("codex").takes_model("haiku"));
        // The slash commands are Claude Code's; another agent is asked in words.
        let agent = default();
        let expected = if agent.id == "claude" { "run /runbook in a Claude Code session".to_string() } else { format!("ask {} to write one", agent.name) };
        assert_eq!(ask_for("runbook", "write one"), expected);
        assert!(row("codex").takes_model("gpt-5-mini") && !row("claude").takes_model("gpt-5-mini"));
        assert!(row("opencode").takes_model("anthropic/claude-sonnet-4"), "a name nobody claims passes");
        let both = serde_json::json!({"claude": "haiku", "codex": "gpt-5-mini"});
        assert_eq!((model_in(&both, &row("codex")), model_in(&serde_json::json!("haiku"), &row("codex"))), (Some("gpt-5-mini".into()), None));
    }

    #[test]
    fn a_session_id_is_read_from_a_runs_output() {
        assert_eq!(session_in("model: x\nsession id: 01a1-77\n--------"), Some("01a1-77".into()));
        assert_eq!(session_in("session id: a b"), None);
        assert_eq!(session_in("session id: --dangerously-skip-permissions"), None);
    }

    #[test]
    fn a_session_id_that_reads_as_a_switch_or_more_fills_no_template() {
        let aside = |fork| Headless { fork: Some(fork), aside: true, ..Default::default() };
        for bad in ["--dangerously-skip-permissions", "-p", "", "s1 --fork-session", "../s1", "s1;rm", "s1\n"] {
            for agent in builtin().iter().filter(|a| !a.resume.is_empty() || !a.fork.is_empty() || !a.run_fork.is_empty()) {
                assert_eq!(agent.resume_with(bad, Some("go")), None, "{} resumes {bad:?}", agent.id);
                assert_eq!(agent.fork_with(bad, Some("go")), None, "{} forks {bad:?}", agent.id);
                assert_eq!(agent.headless("go", &aside(bad)), None, "{} runs a fork of {bad:?}", agent.id);
                assert_eq!(agent.stream_argv(bad, "default", true, &Picked::default()), None);
                assert_eq!(agent.stream_argv(bad, "default", false, &Picked::default()), None);
            }
        }
        assert_eq!(row("claude").resume_argv("0b7c-41_a").unwrap(), ["/bin/claude", "--resume", "0b7c-41_a"]);
        assert!(plain_id("ses_01") && !plain_id("-x") && !plain_id("a/b") && !plain_id("a.b"));
        assert!(model_id("claude-sonnet-4-5") && model_id("sonnet[1m]") && model_id("acme/large:8b") && model_id("gpt-5.1"));
        assert!(!model_id("") && !model_id("-p") && !model_id("haiku\n/exit") && !model_id("haiku; rm") && !model_id(&"m".repeat(129)));
    }

    #[test]
    fn every_agent_gets_herdr_s_icon_or_the_neutral_one() {
        let table = builtin();
        let of = |id: &str| icon_in(&table, id);
        assert_eq!((of("claude"), of("claude-code"), of("codex"), of("pig")), ("claude", "claude", "codex", "pig"));
        assert_eq!((of("antigravity-cli"), of("agy")), ("antigravity", "antigravity"), "the table's id and Herdr's kind alike");
        assert_eq!((of("qodercli"), of("Qoder")), ("qoder", "qoder"));
        assert_eq!((of("cursor"), of("cursor-agent"), of("github-copilot")), ("cursor", "cursor", "copilot"));
        assert_eq!((of("gemini"), of("kiro"), of("amp"), of("muse")), ("gemini", "kiro", "amp", "muse"), "Herdr's kinds the table has no row for");
        for id in ["omp", "devin", "droid", "kilo", "qwen", "letta", "cline", "maki", "", "my-agent"] {
            assert_eq!(of(id), "agent", "{id} has no icon of its own in Herdr");
        }
        let shipped = [
            "agent",
            "amp",
            "antigravity",
            "claude",
            "codex",
            "command-code",
            "copilot",
            "crush",
            "cursor",
            "gemini",
            "grok",
            "hermes",
            "kimi",
            "kiro",
            "mastracode",
            "muse",
            "ollama",
            "opencode",
            "pi",
            "pig",
            "prime-agent",
            "qoder",
        ];
        assert!(ICONS.iter().all(|(icon, _)| shipped.contains(icon)), "every name is an icon Herdr ships");
        for a in &table {
            assert!(shipped.contains(&of(&a.id)), "{}", a.id);
        }
    }

    #[test]
    fn an_agent_s_name_and_mark_come_from_the_table_then_herdr() {
        let table = builtin();
        assert_eq!(display_name_in(&table, "claude"), "Claude Code");
        assert_eq!(display_name_in(&table, "agy"), "Antigravity", "an alias");
        assert_eq!(
            (display_name_in(&table, "gemini"), display_name_in(&table, "cline")),
            ("Gemini CLI".to_string(), "Cline".to_string()),
            "a table row first, then Herdr"
        );
        assert_eq!(display_name_in(&table, "my-agent"), "my-agent");
        let mark = |id: &str| mark_for(icon_in(&table, id), id);
        assert_eq!((mark("claude"), mark("qodercli"), mark("antigravity-cli")), ("claude".to_string(), "qoder".to_string(), "antigravity".to_string()));
        assert_eq!(mark("Devin"), "devin", "no icon: its own id");
        assert_eq!(mark_for("agent", "acme\nhelper\u{1b}[31m"), "acmehelper31m", "one printable word");
        assert_eq!((display_name_in(&table, "pig"), display_name_in(&table, "command-code")), ("PiG".to_string(), "Command Code".to_string()));
        for (id, _) in HERDR_NAMES {
            assert!(in_table(&table, id).is_none(), "{id} has a row: its name is the table's");
        }
    }

    #[test]
    fn a_shell_line_quotes_what_needs_it() {
        assert_eq!(shell_line(&v(&["claude", "--resume", "a-1", "it's done"])), r"claude --resume a-1 'it'\''s done'");
        assert_eq!(shell_line(&v(&["claude", "--resume", "a\u{15}touch x\r"])), "claude --resume 'atouch x'", "nothing a line editor acts on");
    }
}
