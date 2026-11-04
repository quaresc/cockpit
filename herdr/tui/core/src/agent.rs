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
}

/// What a headless run gets beyond its prompt.
#[derive(Clone, Debug, Default)]
pub struct Headless<'a> {
    pub model: Option<&'a str>,
    /// A past session to branch from.
    pub fork: Option<&'a str>,
    /// May edit files in its folder, and in `dirs`, without asking.
    pub edits: bool,
    pub dirs: Vec<String>,
    /// Instructions beside the prompt.
    pub system: Option<&'a str>,
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
            run_fork: v(self.run_fork),
            edits: v(self.edits),
            add_dir: v(self.add_dir),
            system: v(self.system),
            models: v(self.models),
        }
    }
}

const NONE: &[&str] = &[];

/// The built-in table, before the config's overrides. Agents Herdr integrates; see `herdr integration --help`.
pub fn builtin() -> Vec<Agent> {
    ROWS.iter().map(Row::agent).collect()
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
        system: NONE,
        models: NONE,
        transcripts: Transcripts::Pi,
    },
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
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::Omp,
    },
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
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::Copilot,
    },
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
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
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
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
    Row {
        id: "kimi",
        name: "Kimi Code",
        bin: "kimi",
        aliases: &["kimi-code"],
        start: &["{bin}"],
        run: &["{bin}", "-p", "{prompt}"],
        model: &["-m", "{model}"],
        resume: &["{bin}", "--session", "{session}"],
        fork: NONE,
        run_fork: NONE,
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
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
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
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
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: &["qwen"],
        transcripts: Transcripts::Qwen,
    },
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
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: NONE,
        transcripts: Transcripts::None,
    },
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
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: &["gemini-"],
        transcripts: Transcripts::None,
    },
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
        edits: NONE,
        add_dir: NONE,
        system: NONE,
        models: &["grok-"],
        transcripts: Transcripts::None,
    },
];

/// The table with `agents.commands.<id>` from the config laid over it field by field (a new id adds an agent).
pub fn all() -> Vec<Agent> {
    let mut list = builtin();
    let config = files::config();
    let Some(overrides) = config["agents"]["commands"].as_object() else { return list };
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
            run_fork: vec![],
            edits: vec![],
            add_dir: vec![],
            system: vec![],
            models: vec![],
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
        ];
        for (key, field) in fields {
            if let Some(list) = strs(&o[key]) {
                *field = list;
            }
        }
        match at {
            Some(i) => list[i] = agent,
            None => list.push(agent),
        }
    }
    list
}

pub fn get(id: &str) -> Option<Agent> {
    all().into_iter().find(|a| a.id == id || a.aliases.iter().any(|x| x == id))
}

/// The agent a Herdr terminal runs, from its `program` (or the agent's own report, or its name as the app shows it).
pub fn of_program(program: &str) -> Option<Agent> {
    let p = program.trim().to_ascii_lowercase();
    all().into_iter().find(|a| a.id == p || a.bin == p || a.name.to_ascii_lowercase() == p || a.aliases.iter().any(|x| x.to_ascii_lowercase() == p))
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
    /// else after the rest.
    fn fill(&self, template: &[String], prompt: Option<&str>, session: Option<&str>, opts: &[String]) -> Vec<String> {
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
        out
    }

    fn with(template: &[String], key: &str, value: &str) -> Vec<String> {
        template.iter().map(|a| a.replace(key, value)).collect()
    }

    fn model_args(&self, model: Option<&str>) -> Vec<String> {
        model.map(|m| Self::with(&self.model, "{model}", m)).unwrap_or_default()
    }

    /// Interactive, with a first message when given.
    pub fn start_argv(&self, prompt: Option<&str>, model: Option<&str>) -> Vec<String> {
        self.fill(&self.start, prompt, None, &self.model_args(model))
    }

    /// Headless: None when the agent has no such mode.
    pub fn run_argv(&self, prompt: &str, model: Option<&str>) -> Option<Vec<String>> {
        (!self.run.is_empty()).then(|| self.fill(&self.run, Some(prompt), None, &self.model_args(model)))
    }

    pub fn resume_argv(&self, session: &str) -> Option<Vec<String>> {
        self.resume_with(session, None)
    }

    pub fn fork_argv(&self, session: &str) -> Option<Vec<String>> {
        self.fork_with(session, None)
    }

    /// Interactive on a past session, with a message when given.
    pub fn resume_with(&self, session: &str, prompt: Option<&str>) -> Option<Vec<String>> {
        (!self.resume.is_empty()).then(|| self.fill(&self.resume, prompt.filter(|p| !p.is_empty()), Some(session), &[]))
    }

    /// Interactive on a new branch of a past session, with a first message when given.
    pub fn fork_with(&self, session: &str, prompt: Option<&str>) -> Option<Vec<String>> {
        (!self.fork.is_empty()).then(|| self.fill(&self.fork, prompt.filter(|p| !p.is_empty()), Some(session), &[]))
    }

    pub fn can_run(&self) -> bool {
        !self.run.is_empty()
    }

    /// An unattended run, None when the agent has no headless mode or cannot fork when asked to. Switches it lacks are
    /// left out, and instructions it cannot take as a system prompt go before the prompt.
    pub fn headless(&self, prompt: &str, how: &Headless) -> Option<Vec<String>> {
        let template = if how.fork.is_some() { &self.run_fork } else { &self.run };
        if template.is_empty() {
            return None;
        }
        let mut opts = self.model_args(how.model.filter(|m| self.takes_model(m)));
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
        Some(self.fill(template, Some(&prompt), how.fork, &opts))
    }
}

/// The session id a headless run printed on a `session id: <id>` line, as Codex does on stderr.
pub fn session_in(output: &str) -> Option<String> {
    output.lines().find_map(|l| {
        let (key, id) = l.split_once(':')?;
        let id = id.trim();
        (key.trim().eq_ignore_ascii_case("session id") && !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)))
            .then(|| id.to_string())
    })
}

/// An argv as one shell command line, each argument single-quoted, for terminals that take a line of text.
pub fn shell_line(argv: &[String]) -> String {
    argv.iter()
        .map(|a| if a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@".contains(c)) { a.clone() } else { format!("'{}'", a.replace('\'', r"'\''")) })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> Agent {
        let mut a = builtin().into_iter().find(|a| a.id == id).unwrap();
        a.bin = format!("/bin/{}", a.bin);
        a
    }

    #[test]
    fn templates_drop_what_is_not_given() {
        let c = row("codex");
        assert_eq!(c.start_argv(None, None), ["/bin/codex"]);
        assert_eq!(c.start_argv(Some("fix it"), None), ["/bin/codex", "fix it"]);
        assert_eq!(c.run_argv("hi", Some("o3")).unwrap(), ["/bin/codex", "exec", "--skip-git-repo-check", "--model", "o3", "hi"]);
        assert_eq!(c.fork_argv("abc").unwrap(), ["/bin/codex", "fork", "abc"]);
        assert_eq!(c.fork_with("abc", Some("look")).unwrap(), ["/bin/codex", "fork", "abc", "look"]);
        assert_eq!(c.resume_with("abc", Some("")).unwrap(), ["/bin/codex", "resume", "abc"]);
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
        assert_eq!(row("droid").run_argv("go", Some("m")).unwrap(), ["/bin/droid", "exec", "go", "-m", "m"]);
        assert_eq!(row("kimi").start_argv(Some("go"), None), ["/bin/kimi"], "Kimi takes no first message");
        assert_eq!(of_program("agy").unwrap().id, "antigravity-cli");
        assert_eq!(of_program("cursor-agent").unwrap().id, "cursor");
        assert!(of_program("agent").is_none());
    }

    #[test]
    fn a_headless_run_gets_what_the_agent_takes() {
        let how = Headless { model: Some("sonnet"), fork: Some("s1"), edits: true, dirs: vec!["/rb".into()], system: Some("Be brief.") };
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
        assert_eq!(plain, ["/bin/codex", "exec", "--skip-git-repo-check", "--model", "gpt-5-mini", "Go."]);
        assert!(row("copilot").headless("Go.", &how).is_none(), "Copilot cannot branch headless");
        assert_eq!(row("copilot").headless("Go.", &Headless::default()).unwrap(), ["/bin/copilot", "-p", "Go."]);
    }

    #[test]
    fn a_model_goes_only_to_the_agent_it_belongs_to() {
        assert!(row("claude").takes_model("haiku") && !row("codex").takes_model("haiku"));
        assert!(row("codex").takes_model("gpt-5-mini") && !row("claude").takes_model("gpt-5-mini"));
        assert!(row("opencode").takes_model("anthropic/claude-sonnet-4"), "a name nobody claims passes");
        let both = serde_json::json!({"claude": "haiku", "codex": "gpt-5-mini"});
        assert_eq!((model_in(&both, &row("codex")), model_in(&serde_json::json!("haiku"), &row("codex"))), (Some("gpt-5-mini".into()), None));
    }

    #[test]
    fn a_session_id_is_read_from_a_runs_output() {
        assert_eq!(session_in("model: x\nsession id: 01a1-77\n--------"), Some("01a1-77".into()));
        assert_eq!(session_in("session id: a b"), None);
    }

    #[test]
    fn a_shell_line_quotes_what_needs_it() {
        assert_eq!(shell_line(&v(&["claude", "--resume", "a-1", "it's done"])), r"claude --resume a-1 'it'\''s done'");
    }
}
