//! Agents' past sessions from the files each agent keeps: where they are, which one a terminal holds, and a reader per
//! format (`agent::Transcripts`) giving its folder, title, times, model, branch, messages, tokens and context use.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

pub use crate::agent::Transcripts;
use crate::agent::{Plan, Tasks};
use crate::files;

/// The most of one message kept: a pasted log or a long answer says what it is in far less.
const MESSAGE_CAP: usize = 16 * 1024;
const TITLE_CAP: usize = 100;
/// The most of a tool call's line kept.
const SUMMARY_CAP: usize = 120;
/// Longer lines hold tool output or pictures: skipped without being read into memory.
const LINE_CAP: usize = 4 << 20;
/// How much of a transcript's end `usage` reads.
const TAIL: u64 = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub role: Role,
    pub text: String,
    /// RFC 3339, as the agent wrote it; empty when it did not.
    pub at: String,
    /// The tool it called in place of words, when the read asked for them.
    pub tool: Option<Tool>,
}

/// A tool call, as a chat shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    /// The agent's own id for the call (`toolu_…`), empty where its format gives none.
    pub id: String,
    /// What the agent calls it: `Bash`, `Edit`, `Read`.
    pub name: String,
    /// The command, path or pattern it was given, on one line.
    pub summary: String,
    /// Whether it worked, when the transcript says.
    pub ok: Option<bool>,
    /// What the agent said the call is for (a command's description), when it said.
    pub about: Option<String>,
}

/// What a task started beside the turn is: a subagent with a conversation of its own, a background job, a workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    Subagent,
    Background,
    Workflow,
}

impl TaskKind {
    pub fn word(self) -> &'static str {
        match self {
            TaskKind::Subagent => "subagent",
            TaskKind::Background => "background",
            TaskKind::Workflow => "workflow",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Running,
    Done,
    Failed,
}

impl TaskState {
    pub fn word(self) -> &'static str {
        match self {
            TaskState::Running => "running",
            TaskState::Done => "done",
            TaskState::Failed => "failed",
        }
    }
}

/// Work an agent started beside its turn, as its transcript tells it: the call that started it, what it is for, and
/// how it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    /// The agent's id for it (a subagent's `agentId`, a background job's task id), its call's id until the result says.
    pub id: String,
    pub kind: TaskKind,
    pub name: String,
    pub state: TaskState,
    /// RFC 3339, when its call was made and when its result or notification came.
    pub started: String,
    pub ended: Option<String>,
    pub tool_use_id: String,
    /// What it reported when it ended, when the transcript holds a notification.
    pub summary: Option<String>,
    /// A subagent's own transcript, once found beside its parent's.
    pub transcript: Option<PathBuf>,
}

/// How much of a transcript a read keeps. Tool calls cost a second pass over records the other readers skip, so only
/// what draws a conversation asks for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Detail {
    #[default]
    Words,
    Tools,
    /// What names the session alone: no messages are kept, so a transcript followed for its name stays small.
    Names,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    /// Tokens over the whole session; input counts what was not read from the cache.
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Tokens the last turn held in the model's context.
    pub context: Option<u64>,
    /// The model's context window, when the transcript says.
    pub window: Option<u64>,
    /// USD, when the transcript says.
    pub cost: Option<f64>,
}

impl Usage {
    /// How full the context is, in percent, when both sides are known.
    pub fn context_percent(&self) -> Option<f64> {
        let (used, window) = (self.context?, self.window.filter(|w| *w > 0)?);
        Some((used as f64 * 100.0 / window as f64).clamp(0.0, 100.0))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcript {
    pub kind: Transcripts,
    pub id: String,
    pub path: PathBuf,
    pub cwd: String,
    pub title: String,
    /// RFC 3339 times of its first and last records.
    pub first: String,
    pub last: String,
    pub model: Option<String>,
    /// The effort level its last turn ran with, where the transcript says.
    pub effort: Option<String>,
    /// The speed its last reply was served at ("fast", "standard"), where the transcript says.
    pub speed: Option<String>,
    /// The permission mode its last turn ran in, in Claude Code's words (`default`, `acceptEdits`, `plan`, `auto`,
    /// `bypassPermissions`, `dontAsk`), where the transcript says.
    pub mode: Option<String>,
    /// Whether the agent showed the person only its last words (Claude Code's focus mode), where the transcript says.
    pub focus: Option<bool>,
    pub branch: Option<String>,
    pub messages: Vec<Message>,
    pub usage: Usage,
    /// The name the person gave the session in the agent itself (Claude Code's `/rename`), when any.
    pub named: Option<String>,
    /// The title the agent gave the session, when any.
    pub titled: Option<String>,
    /// The first thing the person typed, a slash command as "name args".
    pub opening: Option<String>,
    /// The subagents, background jobs and workflows its turns started, read with the tool calls.
    pub tasks: Vec<Task>,
}

impl Transcript {
    /// Its messages' words, one message per line, up to `cap` bytes (cut on a character).
    pub fn text(&self, cap: usize) -> String {
        let mut out = String::new();
        for m in self.messages.iter().filter(|m| m.tool.is_none()) {
            keep(&mut out, &m.text, cap);
        }
        out
    }
}

/// A file's size and modification time in nanoseconds: it changed when either did.
pub type Stamp = (u64, u128);

/// Every format with a reader.
pub const KINDS: [Transcripts; 7] =
    [Transcripts::Claude, Transcripts::Codex, Transcripts::Pi, Transcripts::Omp, Transcripts::Qwen, Transcripts::Copilot, Transcripts::Gemini];

impl Transcripts {
    /// The agent that writes this format (its id in `agent::all()`), for resuming a session found in it.
    pub fn agent(self) -> &'static str {
        match self {
            Transcripts::None => "",
            Transcripts::Claude => "claude",
            Transcripts::Codex => "codex",
            Transcripts::Pi => "pi",
            Transcripts::Omp => "omp",
            Transcripts::Qwen => "qwen",
            Transcripts::Copilot => "copilot",
            Transcripts::Gemini => "gemini",
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Where they are

/// Claude Code's own folder: `$CLAUDE_CONFIG_DIR` when set, else `~/.claude`.
pub fn claude_home() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| files::home().join(".claude"))
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|d| !d.is_empty()).map(PathBuf::from)
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| files::home().join(".codex"))
}

/// The folder holding a format's transcripts (it may not exist): what to watch for new ones.
pub fn root(kind: Transcripts) -> Option<PathBuf> {
    match kind {
        Transcripts::None => None,
        Transcripts::Claude => Some(claude_home().join("projects")),
        Transcripts::Codex => Some(codex_home().join("sessions")),
        Transcripts::Pi => Some(env_dir("PI_CODING_AGENT_DIR").unwrap_or_else(|| files::home().join(".pi/agent")).join("sessions")),
        Transcripts::Omp => {
            let xdg = env_dir("XDG_DATA_HOME").map(|d| d.join("omp")).filter(|d| d.is_dir());
            Some(match (env_dir("PI_CODING_AGENT_DIR"), xdg) {
                (Some(d), _) => d.join("sessions"),
                (None, Some(d)) => d.join("sessions"),
                (None, None) => files::home().join(".omp/agent/sessions"),
            })
        }
        Transcripts::Qwen => Some(env_dir("QWEN_RUNTIME_DIR").or_else(|| env_dir("QWEN_HOME")).unwrap_or_else(|| files::home().join(".qwen")).join("projects")),
        Transcripts::Copilot => Some(env_dir("COPILOT_HOME").unwrap_or_else(|| files::home().join(".copilot")).join("session-state")),
        // `GEMINI_CLI_HOME` stands for the home folder (gemini-cli `utils/paths.ts`).
        Transcripts::Gemini => Some(env_dir("GEMINI_CLI_HOME").unwrap_or_else(files::home).join(".gemini/tmp")),
    }
}

/// The session files of the formats that keep one folder per project or session, under their root.
fn layered(kind: Transcripts, root: &Path) -> Vec<PathBuf> {
    let jsonl = |p: &PathBuf| p.extension().is_some_and(|e| e == "jsonl");
    match kind {
        // <folder>/<time>_<id>.jsonl; oh-my-pi's subagents sit in a folder beside their parent's file.
        Transcripts::Pi | Transcripts::Omp => children(root).iter().flat_map(|d| children(d)).filter(jsonl).collect(),
        Transcripts::Qwen => children(root).iter().flat_map(|d| children(&d.join("chats"))).filter(jsonl).collect(),
        Transcripts::Copilot => children(root).into_iter().map(|d| d.join("events.jsonl")).filter(|p| p.is_file()).collect(),
        // A subagent's file sits in a folder of its parent's under chats/: left out.
        Transcripts::Gemini => children(root).iter().flat_map(|d| children(&d.join("chats"))).filter(jsonl).collect(),
        _ => Vec::new(),
    }
}

/// The folder a session of a layered format says it ran in, from its first records; Gemini CLI's project folder from the
/// `.project_root` file it keeps beside `chats`.
fn header_cwd(kind: Transcripts, path: &Path) -> Option<String> {
    if kind == Transcripts::Gemini {
        return gemini_root(path);
    }
    let mut lines = lines_of(path)?;
    for _ in 0..5 {
        let v: Value = serde_json::from_slice(lines.next()?).unwrap_or(Value::Null);
        let cwd = match kind {
            Transcripts::Pi | Transcripts::Omp => v["cwd"].as_str().filter(|_| v["type"] == "session"),
            Transcripts::Qwen => v["cwd"].as_str(),
            Transcripts::Copilot => v["data"]["context"]["cwd"].as_str(),
            _ => None,
        };
        if let Some(c) = cwd {
            return Some(c.to_string());
        }
    }
    None
}

/// The format a transcript file is in, from the folder it sits under.
pub fn kind_of(path: &Path) -> Option<Transcripts> {
    KINDS.into_iter().find(|k| root(*k).is_some_and(|r| path.starts_with(&r)))
}

fn stamp(meta: &std::fs::Metadata) -> Stamp {
    let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or_default();
    (meta.len(), modified)
}

fn modified_ms(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| (stamp(&m).1 / 1_000_000) as u64).unwrap_or(0)
}

/// Every session transcript of a format under its root, with its stamp; subagents' are left out.
pub fn all(kind: Transcripts) -> BTreeMap<String, Stamp> {
    root(kind).map(|r| all_in(kind, &r)).unwrap_or_default()
}

/// Every session transcript of a format under this folder (its root, or a copy of one).
pub fn all_in(kind: Transcripts, root: &Path) -> BTreeMap<String, Stamp> {
    let mut found = BTreeMap::new();
    let mut add = |path: PathBuf| {
        if let Ok(meta) = std::fs::metadata(&path)
            && meta.is_file()
        {
            found.insert(path.to_string_lossy().into_owned(), stamp(&meta));
        }
    };
    match kind {
        Transcripts::None => {}
        Transcripts::Claude => {
            for file in children(root).iter().flat_map(|p| children(p)) {
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if name.ends_with(".jsonl") && !name.starts_with("agent-") {
                    add(file);
                }
            }
        }
        Transcripts::Codex => {
            for file in codex_days(root, 0).iter().flat_map(|d| children(d)) {
                if is_rollout(&file) {
                    add(file);
                }
            }
        }
        _ => layered(kind, root).into_iter().for_each(add),
    }
    found
}

fn children(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    out.sort();
    out
}

/// A session's transcript by its id: under its folder first (where the format files by folder), else anywhere.
pub fn find(kind: Transcripts, session: &str, cwd: &str) -> Option<PathBuf> {
    if session.is_empty() || session.contains(['/', '\\']) {
        return None;
    }
    let root = root(kind)?;
    match kind {
        Transcripts::None => None,
        Transcripts::Claude => {
            let name = format!("{session}.jsonl");
            let direct = root.join(claude_folder(cwd)).join(&name);
            if direct.is_file() {
                return Some(direct);
            }
            children(&root).into_iter().map(|d| d.join(&name)).find(|p| p.is_file())
        }
        Transcripts::Codex => {
            let tail = format!("-{session}.jsonl");
            codex_days(&root, 0).into_iter().rev().flat_map(|d| children(&d)).find(|f| f.to_string_lossy().ends_with(&tail))
        }
        _ => layered(kind, &root).into_iter().find(|p| id_of(kind, p).as_deref() == Some(session)),
    }
}

/// A session's transcript in whichever format holds it, the agent's own first when known.
pub fn locate(session: &str, cwd: &str, first: Option<Transcripts>) -> Option<(Transcripts, PathBuf)> {
    first.into_iter().chain(KINDS).find_map(|k| find(k, session, cwd).map(|p| (k, p)))
}

/// The session an agent started in this folder since `since_ms` (a terminal's start), the latest written: for a
/// terminal whose agent does not say which session it holds.
pub fn latest(kind: Transcripts, cwd: &str, since_ms: u64) -> Option<PathBuf> {
    if cwd.is_empty() {
        return None;
    }
    let root = root(kind)?;
    let fresh = |p: &PathBuf| modified_ms(p) + 2_000 >= since_ms;
    let newest = |files: Vec<PathBuf>| files.into_iter().max_by_key(|p| modified_ms(p));
    match kind {
        Transcripts::None => None,
        Transcripts::Claude => {
            let files = children(&root.join(claude_folder(cwd))).into_iter().filter(|p| p.extension().is_some_and(|e| e == "jsonl") && fresh(p)).collect();
            newest(files)
        }
        Transcripts::Codex => {
            let here = canonical(cwd);
            let files = codex_days(&root, since_ms.saturating_sub(86_400_000))
                .iter()
                .flat_map(|d| children(d))
                .filter(|p| is_rollout(p) && fresh(p))
                .filter(|p| {
                    codex_meta(p).is_some_and(|m| canonical(m["cwd"].as_str().unwrap_or_default()) == here && m["source"] != "exec" && !codex_subagent(&m))
                })
                .collect();
            newest(files)
        }
        _ => {
            let here = canonical(cwd);
            let files = layered(kind, &root).into_iter().filter(|p| fresh(p) && header_cwd(kind, p).is_some_and(|c| canonical(&c) == here)).collect();
            newest(files)
        }
    }
}

/// The session id a transcript file holds, from its name.
pub fn id_of(kind: Transcripts, path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    match kind {
        Transcripts::None => None,
        Transcripts::Claude => Some(stem.to_string()),
        // rollout-2026-11-03T17-01-47-<uuid>
        Transcripts::Codex => (stem.len() > 36).then(|| stem[stem.len() - 36..].to_string()).filter(|id| id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')),
        // <time>_<id>
        Transcripts::Pi | Transcripts::Omp => Some(stem.rsplit_once('_').map_or(stem, |(_, id)| id).to_string()),
        Transcripts::Qwen => Some(stem.to_string()),
        // session-state/<id>/events.jsonl
        Transcripts::Copilot => path.parent()?.file_name()?.to_str().map(str::to_string),
        // session-<time>-<first 8 of the id>.jsonl: the whole id is in its first line.
        Transcripts::Gemini => {
            let first: Value = serde_json::from_slice(lines_of(path)?.next()?).ok()?;
            first["sessionId"].as_str().filter(|id| !id.is_empty()).map(str::to_string)
        }
    }
}

/// Claude Code's folder for a cwd: every character not a letter or digit as "-".
fn claude_folder(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn canonical(path: &str) -> String {
    std::fs::canonicalize(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| path.trim_end_matches('/').to_string())
}

/// Codex's day folders (`YYYY/MM/DD`), oldest first, from the day holding `since_ms` on (local time, as Codex names them).
fn codex_days(root: &Path, since_ms: u64) -> Vec<PathBuf> {
    let from = (since_ms > 0)
        .then(|| chrono::TimeZone::timestamp_millis_opt(&chrono::Local, since_ms as i64).single().map(|t| t.format("%Y/%m/%d").to_string()))
        .flatten()
        .unwrap_or_default();
    let mut out = Vec::new();
    for year in children(root) {
        for month in children(&year) {
            for day in children(&month) {
                let rel = day.strip_prefix(root).map(|r| r.to_string_lossy().into_owned()).unwrap_or_default();
                if day.is_dir() && rel >= from {
                    out.push(day);
                }
            }
        }
    }
    out
}

fn is_rollout(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    name.starts_with("rollout-") && name.ends_with(".jsonl")
}

/// A Codex rollout's first record: the session's id, cwd, source and git.
fn codex_meta(path: &Path) -> Option<Value> {
    let file = std::fs::File::open(path).ok()?;
    let mut lines = Lines { reader: BufReader::new(file), line: Vec::new() };
    let v: Value = serde_json::from_slice(lines.next()?).ok()?;
    match v["type"].as_str() {
        Some("session_meta") => Some(v["payload"].clone()),
        // The first rollouts began with the meta itself.
        _ => v["id"].is_string().then_some(v),
    }
}

/// A session Codex started itself (a sub-agent it spawned, its guardian reviewing an approval): not one to find again.
fn codex_subagent(meta: &Value) -> bool {
    meta["source"].get("subagent").is_some()
}

/// Thread names Codex keeps beside its sessions, by session id; the latest wins.
fn codex_titles() -> HashMap<String, String> {
    let text = std::fs::read_to_string(codex_home().join("session_index.jsonl")).unwrap_or_default();
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| Some((v["id"].as_str()?.to_string(), v["thread_name"].as_str().filter(|n| !n.trim().is_empty())?.to_string())))
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading

/// The session in a transcript, its words alone; None when it holds nothing the person typed, or cannot be read.
pub fn read(kind: Transcripts, path: &Path) -> Option<Transcript> {
    read_detail(kind, path, Detail::Words)
}

/// The same, keeping the tool calls between the messages when `detail` asks for them.
pub fn read_detail(kind: Transcripts, path: &Path, detail: Detail) -> Option<Transcript> {
    read_in(kind, path, detail, false)
}

/// A subagent's own transcript, which the plain readers leave out: Claude's `agent-*.jsonl` beside its parent, a
/// Codex rollout its parent spawned.
pub fn read_subagent(kind: Transcripts, path: &Path, detail: Detail) -> Option<Transcript> {
    read_in(kind, path, detail, true)
}

fn read_in(kind: Transcripts, path: &Path, detail: Detail, child: bool) -> Option<Transcript> {
    let mut t = match kind {
        Transcripts::None => return None,
        Transcripts::Claude => read_claude_in(path, detail, child)?,
        Transcripts::Codex => read_codex_in(path, detail, child)?,
        Transcripts::Pi | Transcripts::Omp => read_pi(path, detail)?,
        Transcripts::Qwen => read_qwen(path, detail)?,
        Transcripts::Copilot => read_copilot(path, detail)?,
        Transcripts::Gemini => read_gemini(path, detail)?,
    };
    t.kind = kind;
    t.path = path.to_path_buf();
    if t.id.is_empty() {
        t.id = id_of(kind, path)?;
    }
    t.title = tidy(&t.title);
    if detail == Detail::Names {
        t.messages.clear();
    }
    Some(t)
}

/// The token use at a transcript's end, read from its last part only (cheap enough to ask on every poll): context and
/// window always, totals where the format keeps running ones (Codex).
pub fn usage(kind: Transcripts, path: &Path) -> Option<Usage> {
    let tail = tail(path, TAIL)?;
    let lines = tail.split(|b| *b == b'\n').rev();
    match kind {
        Transcripts::None => None,
        Transcripts::Claude => lines.filter(|l| contains(l, b"\"usage\"")).find_map(|l| {
            let line: ClaudeLine = serde_json::from_slice(l).ok()?;
            let m = line.message.filter(|_| line.kind == "assistant" && line.is_sidechain != Some(true))?;
            let u = m.usage?;
            Some(Usage { context: Some(u.input_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens), ..Usage::default() })
        }),
        Transcripts::Codex => lines.filter(|l| contains(l, b"token_count")).find_map(|l| {
            let v: Value = serde_json::from_slice(l).ok()?;
            codex_usage(&v["payload"]["info"])
        }),
        Transcripts::Pi | Transcripts::Omp => lines.filter(|l| contains(l, b"\"usage\"")).find_map(|l| {
            let v: Value = serde_json::from_slice(l).ok()?;
            let u = &v["message"]["usage"];
            (v["type"] == "message" && v["message"]["role"] == "assistant" && u.is_object()).then(|| Usage { context: Some(pi_context(u)), ..Usage::default() })
        }),
        Transcripts::Qwen => lines.filter(|l| contains(l, b"usageMetadata")).find_map(|l| {
            let v: Value = serde_json::from_slice(l).ok()?;
            let u = &v["usageMetadata"];
            u.is_object().then(|| Usage { context: Some(qwen_context(u)), window: v["contextWindowSize"].as_u64(), ..Usage::default() })
        }),
        Transcripts::Copilot => lines.filter(|l| contains(l, b"session.usage_info")).find_map(|l| {
            let v: Value = serde_json::from_slice(l).ok()?;
            Some(Usage { context: v["data"]["currentTokens"].as_u64(), window: v["data"]["tokenLimit"].as_u64(), ..Usage::default() })
        }),
        Transcripts::Gemini => lines.filter(|l| contains(l, b"\"tokens\"")).find_map(|l| {
            let v: Value = serde_json::from_slice(l).ok()?;
            (v["type"] == "gemini").then(|| Usage { context: v["tokens"]["input"].as_u64(), ..Usage::default() })
        }),
    }
}

/// Where an agent stands in its session, read off its transcript's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    Working,
    /// Waiting on the person: an approval or a question, when the format records it.
    Blocked,
    Done,
    /// The person stopped the turn: Claude Code's "[Request interrupted…]", Codex's `turn_aborted`.
    Interrupted,
}

/// Whether the agent is in a turn, from the last turn event its transcript holds: for agents that report no status to
/// Herdr. None for formats with no such events.
pub fn turn(kind: Transcripts, path: &Path) -> Option<Turn> {
    match kind {
        // Claude Code closes a turn with a `turn_duration` line; the bookkeeping it writes after says nothing.
        Transcripts::Claude => {
            let tail = tail(path, TAIL)?;
            // From the end: a closed turn ended interrupted when its last message is the interruption.
            let mut closed = false;
            for l in tail.split(|b| *b == b'\n').rev() {
                let Ok(v) = serde_json::from_slice::<Value>(l) else { continue };
                if v["isSidechain"] == true {
                    continue;
                }
                match (v["type"].as_str().unwrap_or_default(), v["subtype"].as_str().unwrap_or_default()) {
                    ("system", "turn_duration") => closed = true,
                    ("user", _) if interruption(&v["message"]["content"]) => return Some(Turn::Interrupted),
                    ("assistant" | "user", _) => return Some(if closed { Turn::Done } else { Turn::Working }),
                    _ => {}
                }
            }
            closed.then_some(Turn::Done)
        }
        Transcripts::Codex => {
            let tail = tail(path, TAIL)?;
            tail.split(|b| *b == b'\n').rev().filter(|l| contains(l, b"\"event_msg\"")).find_map(|l| {
                let v: Value = serde_json::from_slice(l).ok()?;
                match v["payload"]["type"].as_str()? {
                    "task_started" => Some(Turn::Working),
                    "task_complete" => Some(Turn::Done),
                    "turn_aborted" => Some(Turn::Interrupted),
                    t if t.ends_with("_approval_request") || t == "request_user_input" => Some(Turn::Blocked),
                    _ => None,
                }
            })
        }
        _ => None,
    }
}

/// The input of a `tool` call the agent made and nothing answered yet, read off the transcript's end: what its prompt
/// asks about in full. Only when that call is the one thing left unanswered in its last message, so the prompt on its
/// screen can be about nothing else. Formats that write the call before asking: Claude Code's, Qwen Code's, Gemini's.
pub fn pending_input(kind: Transcripts, path: &Path, tool: &str) -> Option<Value> {
    let calls = match kind {
        Transcripts::Claude => pending_claude(path)?,
        Transcripts::Qwen => pending_qwen(path)?,
        Transcripts::Gemini => pending_gemini(path)?,
        _ => return None,
    };
    match calls.as_slice() {
        [(name, input)] if name == tool => Some(input.clone()),
        _ => None,
    }
}

/// Claude Code: the last assistant line's `tool_use` blocks without a `tool_result` after them, as (name, input).
fn pending_claude(path: &Path) -> Option<Vec<(String, Value)>> {
    let tail = tail(path, TAIL)?;
    let mut answered: Vec<String> = Vec::new();
    for l in tail.split(|b| *b == b'\n').rev() {
        let Ok(v) = serde_json::from_slice::<Value>(l) else { continue };
        if v["isSidechain"] == true {
            continue;
        }
        let blocks = v["message"]["content"].as_array().into_iter().flatten();
        match v["type"].as_str().unwrap_or_default() {
            "user" => answered.extend(blocks.filter(|b| b["type"] == "tool_result").filter_map(|b| b["tool_use_id"].as_str().map(str::to_string))),
            "assistant" => {
                let open = blocks.filter(|b| b["type"] == "tool_use" && !answered.iter().any(|id| b["id"] == id.as_str()));
                return Some(open.map(|b| (b["name"].as_str().unwrap_or_default().to_string(), b["input"].clone())).collect());
            }
            _ => {}
        }
    }
    None
}

/// Qwen Code (Gemini's chat format): the last assistant line's `functionCall` parts without a `functionResponse` after
/// them, by id when the parts carry one, else by name.
fn pending_qwen(path: &Path) -> Option<Vec<(String, Value)>> {
    let tail = tail(path, TAIL)?;
    let mut answered: Vec<Value> = Vec::new();
    for l in tail.split(|b| *b == b'\n').rev() {
        let Ok(v) = serde_json::from_slice::<Value>(l) else { continue };
        if v["isSidechain"] == true {
            continue;
        }
        let parts = v["message"]["parts"].as_array().into_iter().flatten();
        match v["type"].as_str().unwrap_or_default() {
            "user" => answered.extend(parts.map(|p| p["functionResponse"].clone()).filter(Value::is_object)),
            "assistant" => {
                let calls = parts.map(|p| &p["functionCall"]).filter(|c| c.is_object());
                let open = calls.filter(|c| {
                    !answered.iter().any(|r| match (c["id"].as_str(), r["id"].as_str()) {
                        (Some(a), Some(b)) => a == b,
                        _ => r["name"] == c["name"],
                    })
                });
                return Some(open.map(|c| (c["name"].as_str().unwrap_or_default().to_string(), c["args"].clone())).collect());
            }
            _ => {}
        }
    }
    None
}

/// Gemini CLI: the last message's tool calls still without an outcome (no `result`, and a `status` that is neither
/// success, error nor cancelled).
fn pending_gemini(path: &Path) -> Option<Vec<(String, Value)>> {
    let (messages, _, _) = gemini_replay(path)?;
    let last = messages.iter().rev().find(|m| matches!(m["type"].as_str(), Some("gemini" | "user")))?;
    if last["type"] != "gemini" {
        return Some(Vec::new());
    }
    let calls = last["toolCalls"].as_array().into_iter().flatten();
    let open = calls.filter(|c| c["result"].is_null() && !matches!(c["status"].as_str(), Some("success" | "error" | "cancelled")));
    Some(open.map(|c| (c["name"].as_str().unwrap_or_default().to_string(), c["args"].clone())).collect())
}

/// The last message of an agent writing in its plan mode, read off the transcript's end: Codex's, when the turn's
/// collaboration mode is plan. None in any other mode: a message of its own is no plan.
pub fn plan_message(kind: Transcripts, path: &Path) -> Option<String> {
    if kind != Transcripts::Codex {
        return None;
    }
    let tail = tail(path, TAIL)?;
    let mut message: Option<String> = None;
    for l in tail.split(|b| *b == b'\n').rev() {
        let Ok(v) = serde_json::from_slice::<Value>(l) else { continue };
        let p = &v["payload"];
        match (v["type"].as_str().unwrap_or_default(), p["type"].as_str().unwrap_or_default()) {
            ("turn_context", _) => return message.filter(|_| p["collaboration_mode"]["mode"] == "plan"),
            ("response_item", "message") if message.is_none() && p["role"] == "assistant" => {
                let text = p["content"].as_array().into_iter().flatten().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n");
                if !text.trim().is_empty() {
                    message = Some(text);
                }
            }
            _ => {}
        }
    }
    None
}

/// The plan an agent asks to carry out, whole, as `plan` says it is kept: a call's argument, the file that argument
/// names (read when it is a readable file), or its last message in plan mode. Empty text is no plan.
pub fn plan(plan: Plan, kind: Transcripts, path: &Path) -> Option<String> {
    let text = match plan {
        Plan::Argument { tool, arg } => pending_input(kind, path, tool)?[arg].as_str().map(str::to_string)?,
        Plan::File { tool, arg } => {
            let named = pending_input(kind, path, tool)?[arg].as_str()?.to_string();
            let file = named.strip_prefix("~/").map(|rest| files::home().join(rest)).unwrap_or_else(|| PathBuf::from(&named));
            let bytes = std::fs::read(file).ok().filter(|b| b.len() <= PLAN_FILE_MAX)?;
            String::from_utf8_lossy(&bytes).into_owned()
        }
        Plan::LastMessage => plan_message(kind, path)?,
    };
    Some(text).filter(|t| !t.trim().is_empty())
}

/// How large a plan file is read: a plan is pages, not a dump.
const PLAN_FILE_MAX: usize = 1024 * 1024;

/// Claude Code's note that the person stopped the turn, as a user message's text.
fn interruption(content: &Value) -> bool {
    let said = |t: &str| t.trim_start().starts_with("[Request interrupted");
    match content {
        Value::String(t) => said(t),
        Value::Array(blocks) => blocks.iter().any(|b| b["type"] == "text" && b["text"].as_str().is_some_and(said)),
        _ => false,
    }
}

/// The file's last `bytes`, from its first whole line.
pub fn tail(path: &Path, bytes: u64) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    if start > 0 {
        // The first line is cut.
        let first = buf.iter().position(|b| *b == b'\n').map(|i| i + 1).unwrap_or(buf.len());
        buf.drain(..first);
    }
    Some(buf)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// A title as one line, short.
fn tidy(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(TITLE_CAP).collect()
}

/// Text the person typed, not what the agent put in their turn: tagged wrappers (`<command-name>`,
/// `<environment_context>`), interruptions, injected instructions.
fn is_typed(text: &str) -> bool {
    let text = text.trim_start();
    let tagged = text.strip_prefix('<').is_some_and(|rest| {
        let tag: String = rest.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        !tag.is_empty()
            && tag.chars().any(|c| c == '-' || c == '_' || c.is_ascii_lowercase())
            && rest.trim_start_matches('/')[tag.len()..].starts_with(['>', ' ', '\n'])
    });
    let pasted = text.starts_with("<pasted_content");
    !text.is_empty() && (pasted || !tagged) && !text.starts_with("[Request interrupted") && !text.starts_with("# AGENTS.md instructions")
}

/// What the person typed with the wrappers Claude Code puts around a paste taken off, the pasted words kept.
fn unwrap_pasted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("<pasted_content") {
        out.push_str(&rest[..at]);
        rest = rest[at..].find('>').map_or("", |end| &rest[at + end + 1..]);
    }
    out.push_str(rest);
    out.replace("</pasted_content>", "").trim().to_string()
}

/// A slash command the person ran, as its transcript wraps it (`<command-name>/review</command-name>
/// <command-args>412</command-args>`): "review 412".
fn command_of(text: &str) -> Option<String> {
    let inside = |tag: &str| -> Option<String> {
        let open = format!("<{tag}>");
        let from = text.find(&open)? + open.len();
        let to = text[from..].find(&format!("</{tag}>"))? + from;
        Some(text[from..to].trim().to_string())
    };
    let name = inside("command-name")?.trim_start_matches('/').to_string();
    if name.is_empty() {
        return None;
    }
    let args = inside("command-args").unwrap_or_default();
    Some(if args.is_empty() { name } else { format!("{name} {args}") })
}

/// A tool call where a message would be: `summary` is already one line.
fn tool_message(at: &str, id: &str, name: &str, summary: String, ok: Option<bool>, about: Option<String>) -> Message {
    Message {
        role: Role::Assistant,
        text: String::new(),
        at: at.to_string(),
        tool: Some(Tool { id: id.to_string(), name: name.to_string(), summary, ok, about }),
    }
}

fn message(role: Role, text: &str, at: &str) -> Message {
    let mut end = text.len().min(MESSAGE_CAP);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Message { role, text: text[..end].to_string(), at: at.to_string(), tool: None }
}

/// Adds the text on a line of its own, up to the cap, cut on a character boundary.
fn keep(out: &mut String, text: &str, cap: usize) {
    let room = cap.saturating_sub(out.len() + 1);
    if room == 0 {
        return;
    }
    let mut end = text.len().min(room);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&text[..end]);
}

/// A transcript's lines through one buffer, those over LINE_CAP skipped.
struct Lines<R> {
    reader: R,
    line: Vec<u8>,
}

impl<R: BufRead> Lines<R> {
    fn next(&mut self) -> Option<&[u8]> {
        self.line.clear();
        let mut too_long = false;
        loop {
            let buf = match self.reader.fill_buf() {
                Ok(buf) if !buf.is_empty() => buf,
                _ => return (!too_long && !self.line.is_empty()).then_some(self.line.as_slice()),
            };
            let end = buf.iter().position(|&b| b == b'\n');
            let chunk = &buf[..end.unwrap_or(buf.len())];
            too_long |= self.line.len() + chunk.len() > LINE_CAP;
            if !too_long {
                self.line.extend_from_slice(chunk);
            }
            let used = chunk.len() + usize::from(end.is_some());
            self.reader.consume(used);
            if end.is_some() {
                if too_long {
                    self.line.clear();
                    too_long = false;
                    continue;
                }
                return Some(self.line.as_slice());
            }
        }
    }
}

fn lines_of(path: &Path) -> Option<Lines<BufReader<std::fs::File>>> {
    Some(Lines { reader: BufReader::new(std::fs::File::open(path).ok()?), line: Vec::new() })
}

// ---------------------------------------------------------------------------------------------------------------------
// Claude Code: ~/.claude/projects/<cwd, non-alphanumerics as "-">/<session>.jsonl, a record per line

/// One line of the transcript, its other fields skipped unread.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct ClaudeLine {
    #[serde(rename = "type")]
    kind: String,
    timestamp: Option<String>,
    cwd: Option<String>,
    session_id: Option<String>,
    git_branch: Option<String>,
    summary: Option<String>,
    custom_title: Option<String>,
    ai_title: Option<String>,
    is_meta: Option<bool>,
    is_sidechain: Option<bool>,
    is_compact_summary: Option<bool>,
    effort: Option<String>,
    permission_mode: Option<String>,
    message: Option<ClaudeMessage>,
    /// What a tool result says beside its text: the subagent or background job it started.
    tool_use_result: Option<ToolUseResult>,
}

/// The telling fields of a tool's result record; its output is skipped unread.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct ToolUseResult {
    agent_id: Option<String>,
    background_task_id: Option<String>,
    is_async: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
    #[serde(deserialize_with = "chunks")]
    content: Vec<Chunk>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation_input_tokens: u64,
    speed: Option<String>,
}

/// A content block, kept only where it says something: words, a tool call, or how one ended.
enum Chunk {
    Text(String),
    Call {
        id: String,
        name: String,
        summary: String,
        about: Option<String>,
        task: CallTask,
    },
    /// How a call ended, and the head of what it answered.
    Result {
        id: String,
        ok: bool,
        head: String,
    },
}

/// What of a call's input says which task it starts: its description, the subagent type, whether it runs in the
/// background.
#[derive(Clone, Debug, Default, PartialEq)]
struct CallTask {
    description: Option<String>,
    subagent_type: Option<String>,
    background: bool,
}

/// The most of a result's text kept: enough to read the id a background job answers with.
const HEAD_CAP: usize = 200;

/// The first `HEAD_CAP` characters of a string, or of the text blocks of a list, the rest left unread.
struct Head(String);

impl<'de> Deserialize<'de> for Head {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Head;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a string or a list of content blocks")
            }
            fn visit_str<E>(self, s: &str) -> Result<Head, E> {
                Ok(Head(s.chars().take(HEAD_CAP).collect()))
            }
            fn visit_unit<E>(self) -> Result<Head, E> {
                Ok(Head(String::new()))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Head, A::Error> {
                #[derive(Deserialize, Default)]
                #[serde(default)]
                struct TextBlock {
                    text: Option<Head>,
                }
                let mut out = String::new();
                while let Some(block) = seq.next_element::<TextBlock>()? {
                    if out.chars().count() >= HEAD_CAP {
                        continue;
                    }
                    if let Some(Head(t)) = block.text {
                        out.push_str(&t);
                    }
                }
                Ok(Head(out.chars().take(HEAD_CAP).collect()))
            }
        }
        d.deserialize_any(V)
    }
}

/// The telling fields of a tool call's input; the rest is skipped unread.
#[derive(Deserialize, Default)]
#[serde(default)]
struct ToolInput {
    command: Option<String>,
    file_path: Option<String>,
    path: Option<String>,
    pattern: Option<String>,
    url: Option<String>,
    description: Option<String>,
    prompt: Option<String>,
    subagent_type: Option<String>,
    run_in_background: Option<bool>,
}

impl ToolInput {
    /// The first of its fields that says what the call did, its description when that is not it already, and what
    /// says which task it starts.
    fn summary(self) -> (Option<String>, Option<String>, CallTask) {
        let about = self.description.clone().filter(|d| !d.trim().is_empty());
        let task = CallTask {
            description: about.as_deref().map(tool_line),
            subagent_type: self.subagent_type.filter(|t| !t.trim().is_empty()),
            background: self.run_in_background == Some(true),
        };
        let summary = [self.command, self.file_path, self.path, self.pattern, self.url, self.description, self.prompt]
            .into_iter()
            .flatten()
            .find(|s| !s.trim().is_empty());
        let about = about.filter(|d| summary.as_ref() != Some(d)).map(|d| tool_line(&d));
        (summary, about, task)
    }
}

/// A tool call on one line: the command it ran, the file it touched, else nothing.
fn tool_line(text: &str) -> String {
    let mut out: String = text.trim().lines().next().unwrap_or_default().chars().take(SUMMARY_CAP).collect();
    if out.len() < text.trim().len() {
        out.push('…');
    }
    out
}

/// A message's content: the words when it is a string, else its blocks. Thinking is dropped; tool calls and their
/// results are kept, and `read_*` decides whether to show them.
fn chunks<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Chunk>, D::Error> {
    struct Chunks;
    struct Block(Option<Chunk>);
    impl<'de> Deserialize<'de> for Block {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Fields;
            impl<'de> Visitor<'de> for Fields {
                type Value = Block;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a content block")
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Block, A::Error> {
                    let (mut kind, mut text, mut name) = (None::<String>, None::<String>, None::<String>);
                    let (mut id, mut used, mut error, mut summary, mut about) = (None::<String>, None::<String>, false, None::<String>, None::<String>);
                    let (mut task, mut head) = (CallTask::default(), None::<Head>);
                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "type" => kind = Some(map.next_value()?),
                            "text" => text = map.next_value()?,
                            "name" => name = map.next_value()?,
                            "id" => id = map.next_value()?,
                            "tool_use_id" => used = map.next_value()?,
                            "is_error" => error = map.next_value::<Option<bool>>()?.unwrap_or(false),
                            "input" => (summary, about, task) = map.next_value::<ToolInput>()?.summary(),
                            // A result's content is read only for its head: the rest is the tool's output.
                            "content" if kind.as_deref() == Some("tool_result") || kind.is_none() => head = map.next_value::<Option<Head>>()?,
                            _ => {
                                map.next_value::<IgnoredAny>()?;
                            }
                        }
                    }
                    Ok(Block(match kind.as_deref() {
                        Some("text") => text.map(Chunk::Text),
                        Some("tool_use") => Some(Chunk::Call {
                            id: id.unwrap_or_default(),
                            name: name.unwrap_or_default(),
                            summary: summary.map(|s| tool_line(&s)).unwrap_or_default(),
                            about,
                            task,
                        }),
                        Some("tool_result") => used.map(|id| Chunk::Result { id, ok: !error, head: head.map(|h| h.0).unwrap_or_default() }),
                        _ => None,
                    }))
                }
            }
            d.deserialize_any(Fields)
        }
    }
    impl<'de> Visitor<'de> for Chunks {
        type Value = Vec<Chunk>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or a list of content blocks")
        }
        fn visit_str<E>(self, s: &str) -> Result<Vec<Chunk>, E> {
            Ok(vec![Chunk::Text(s.to_string())])
        }
        fn visit_unit<E>(self) -> Result<Vec<Chunk>, E> {
            Ok(Vec::new())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<Chunk>, A::Error> {
            let mut out = Vec::new();
            while let Some(Block(chunk)) = seq.next_element::<Block>()? {
                out.extend(chunk);
            }
            Ok(out)
        }
    }
    d.deserialize_any(Chunks)
}

/// A Claude transcript read whole; a subagent's file is read with `child`, its sidechain lines kept.
fn read_claude_in(path: &Path, detail: Detail, child: bool) -> Option<Transcript> {
    let mut fold = ClaudeFold::new_in(detail, child);
    let mut lines = lines_of(path)?;
    while let Some(line) = lines.next() {
        fold.feed(line);
    }
    let title = fold.title()?;
    fold.t.title = title;
    fold.name();
    Some(fold.t)
}

/// What a task notification says: the task, how it ended, its words, and the file holding its output.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notice {
    pub id: String,
    pub status: String,
    pub summary: Option<String>,
    pub output_file: Option<String>,
}

/// The notification Claude Code gives its agent when a task ends, as a user message starting `<task-notification>`;
/// None for any other text.
pub fn task_notification(text: &str) -> Option<Notice> {
    let text = text.trim_start();
    if !text.starts_with("<task-notification>") {
        return None;
    }
    let inside = |tag: &str| -> Option<String> {
        let open = format!("<{tag}>");
        let from = text.find(&open)? + open.len();
        let to = text[from..].find(&format!("</{tag}>"))? + from;
        Some(text[from..to].trim().to_string())
    };
    let id = inside("task-id").filter(|id| !id.is_empty())?;
    Some(Notice { id, status: inside("status").unwrap_or_default(), summary: inside("summary"), output_file: inside("output-file") })
}

/// The task a call starts, by the agent table: its kind and its name (what it is for, else the subagent's type, else
/// the call's own line). None when the tool starts none, or a background tool was not asked to.
pub fn call_task(
    tasks: &Tasks,
    tool: &str,
    description: Option<&str>,
    subagent_type: Option<&str>,
    background: bool,
    summary: &str,
) -> Option<(TaskKind, String)> {
    let named = |fallback: Option<&str>| description.filter(|d| !d.trim().is_empty()).or(fallback).unwrap_or(summary).trim().to_string();
    let is = |list: &[String]| list.iter().any(|t| t == tool);
    if is(&tasks.spawn) {
        return Some((TaskKind::Subagent, named(subagent_type)));
    }
    if is(&tasks.workflow) {
        return Some((TaskKind::Workflow, named(None)));
    }
    if background && is(&tasks.background) {
        return Some((TaskKind::Background, named(None)));
    }
    None
}

/// The id a background job answers with ("Command running in background with ID: b7k2m9xq1."), when its text says.
pub fn background_id(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let at = lower.find("in background with id:")? + "in background with id:".len();
    let id: String = text[at..].trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (!id.is_empty()).then_some(id)
}

/// A Claude transcript folded one line at a time, so a reader following it feeds only the lines added since.
struct ClaudeFold {
    t: Transcript,
    detail: Detail,
    prompt: Option<String>,
    summary: Option<String>,
    named: Option<String>,
    ai_title: Option<String>,
    opening: Option<String>,
    counted: Option<String>,
    /// Where each tool call landed in the messages, so its result can say whether it worked.
    calls: HashMap<String, usize>,
    /// The tools that start a task, from the agent table.
    tasks: Option<Tasks>,
    /// A subagent's own file: its lines are sidechain ones, kept rather than skipped.
    child: bool,
}

impl ClaudeFold {
    fn new_in(detail: Detail, child: bool) -> Self {
        ClaudeFold {
            t: Transcript::default(),
            detail,
            prompt: None,
            summary: None,
            named: None,
            ai_title: None,
            opening: None,
            counted: None,
            calls: HashMap::new(),
            tasks: (detail == Detail::Tools).then(|| crate::agent::tasks_of(Transcripts::Claude)).flatten(),
            child,
        }
    }

    /// Starts the task a call begins, when its tool is one of the table's.
    fn call_started(&mut self, id: &str, name: &str, summary: &str, task: &CallTask, at: &str) {
        let Some(table) = self.tasks.as_ref() else { return };
        let Some((kind, title)) = call_task(table, name, task.description.as_deref(), task.subagent_type.as_deref(), task.background, summary) else { return };
        self.t.tasks.push(Task {
            id: id.to_string(),
            kind,
            name: title,
            state: TaskState::Running,
            started: at.to_string(),
            ended: None,
            tool_use_id: id.to_string(),
            summary: None,
            transcript: None,
        });
    }

    /// How a call's result moves its task: a subagent or job launched gets its id and stays running, anything else
    /// is over.
    fn call_ended(&mut self, id: &str, ok: bool, head: &str, result: Option<&ToolUseResult>, at: &str) {
        let Some(task) = self.t.tasks.iter_mut().find(|t| t.tool_use_id == id) else { return };
        let launched = match task.kind {
            TaskKind::Background => result.and_then(|r| r.background_task_id.clone()).or_else(|| background_id(head)),
            TaskKind::Subagent | TaskKind::Workflow => result.and_then(|r| r.agent_id.clone()).filter(|_| result.is_some_and(|r| r.is_async == Some(true))),
        };
        match launched {
            Some(launched) => task.id = launched,
            None => {
                task.state = if ok { TaskState::Done } else { TaskState::Failed };
                task.ended = Some(at.to_string());
            }
        }
    }

    /// A task notification the agent got: the task it names ends as it says.
    fn notified(&mut self, notice: &Notice, at: &str) {
        let Some(task) = self.t.tasks.iter_mut().find(|t| t.id == notice.id) else { return };
        task.state = if notice.status == "completed" { TaskState::Done } else { TaskState::Failed };
        task.ended = Some(at.to_string());
        task.summary = notice.summary.clone().filter(|s| !s.trim().is_empty());
    }

    /// Lays what names the session on the transcript: the person's rename, the agent's title, the opening words.
    fn name(&mut self) {
        let given = |t: &Option<String>| t.clone().filter(|t| !t.trim().is_empty());
        self.t.named = given(&self.named);
        self.t.titled = given(&self.ai_title).or_else(|| given(&self.summary));
        self.t.opening = self.opening.clone();
    }

    /// Its title: the name given to it, its summary, the agent's own, else the first thing typed. None before that.
    fn title(&self) -> Option<String> {
        let prompt = self.prompt.clone()?;
        Some(self.named.clone().or_else(|| self.summary.clone()).or_else(|| self.ai_title.clone()).unwrap_or(prompt))
    }

    fn feed(&mut self, line: &[u8]) {
        // The system prompt the agent ran with says whether focus mode is on; the snapshot is read for nothing else.
        if contains(line, b"\"type\":\"prompt_snapshot\"") {
            self.t.focus = Some(contains(line, b"# Focus mode"));
            return;
        }
        // `/focus` typed in its terminal says so at once, before the next request's snapshot.
        if contains(line, b"\"subtype\":\"local_command\"") {
            if contains(line, b"local-command-stdout>Focus view enabled") {
                self.t.focus = Some(true);
            } else if contains(line, b"local-command-stdout>Focus view disabled") {
                self.t.focus = Some(false);
            }
            return;
        }
        let Ok(line) = serde_json::from_slice::<ClaudeLine>(line) else { return };
        let t = &mut self.t;
        if let Some(mode) = line.permission_mode.clone().filter(|m| !m.is_empty()) {
            t.mode = Some(mode);
        }
        let at = line.timestamp.filter(|t| !t.is_empty()).unwrap_or_default();
        if !at.is_empty() {
            if t.first.is_empty() {
                t.first = at.clone();
            }
            t.last = at.clone();
        }
        if t.cwd.is_empty()
            && let Some(cwd) = line.cwd
        {
            t.cwd = cwd;
        }
        if t.id.is_empty()
            && let Some(id) = line.session_id
        {
            t.id = id;
        }
        if let Some(b) = line.git_branch.filter(|b| !b.is_empty() && b != "HEAD") {
            t.branch = Some(b);
        }
        match line.kind.as_str() {
            "summary" => self.summary = line.summary.or(self.summary.take()),
            "custom-title" => self.named = line.custom_title.or(self.named.take()),
            "ai-title" => self.ai_title = line.ai_title.or(self.ai_title.take()),
            "user" | "assistant" if ![line.is_compact_summary, line.is_meta].contains(&Some(true)) && (self.child || line.is_sidechain != Some(true)) => {
                let Some(m) = line.message else { return };
                let result = line.tool_use_result;
                let role = if line.kind == "user" { Role::User } else { Role::Assistant };
                if role == Role::Assistant {
                    if let Some(model) = m.model.filter(|m| !m.is_empty() && !m.starts_with('<')) {
                        self.t.model = Some(model);
                    }
                    if let Some(effort) = line.effort.filter(|e| !e.is_empty()) {
                        self.t.effort = Some(effort);
                    }
                    if let Some(speed) = m.usage.as_ref().and_then(|u| u.speed.clone()).filter(|s| !s.is_empty()) {
                        self.t.speed = Some(speed);
                    }
                    // A reply streamed over several lines repeats its usage on each.
                    if let Some(u) = m.usage
                        && (m.id.is_none() || m.id != self.counted)
                    {
                        self.counted = m.id.clone();
                        self.t.usage.input += u.input_tokens;
                        self.t.usage.output += u.output_tokens;
                        self.t.usage.cache_read += u.cache_read_input_tokens;
                        self.t.usage.cache_write += u.cache_creation_input_tokens;
                        self.t.usage.context = Some(u.input_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens);
                    }
                }
                for chunk in m.content {
                    match chunk {
                        Chunk::Text(text) => {
                            if role == Role::User
                                && self.detail == Detail::Tools
                                && let Some(notice) = task_notification(&text)
                            {
                                self.notified(&notice, &at);
                                continue;
                            }
                            if role == Role::User && !is_typed(&text) {
                                if self.opening.is_none() {
                                    self.opening = command_of(&text);
                                }
                                continue;
                            }
                            let text = if role == Role::User { unwrap_pasted(&text) } else { text };
                            if role == Role::User && self.prompt.is_none() {
                                self.prompt = Some(text.clone());
                                self.opening.get_or_insert_with(|| text.clone());
                            }
                            if self.detail != Detail::Names {
                                self.t.messages.push(message(role, &text, &at));
                            }
                        }
                        Chunk::Call { id, name, summary, about, task } if self.detail == Detail::Tools => {
                            self.calls.insert(id.clone(), self.t.messages.len());
                            self.t.messages.push(tool_message(&at, &id, &name, summary.clone(), None, about));
                            self.call_started(&id, &name, &summary, &task, &at);
                        }
                        Chunk::Result { id, ok, head } if self.detail == Detail::Tools => {
                            if let Some(tool) = self.calls.get(&id).and_then(|i| self.t.messages.get_mut(*i)).and_then(|m| m.tool.as_mut()) {
                                tool.ok = Some(ok);
                            }
                            self.call_ended(&id, ok, &head, result.as_ref(), &at);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

/// A transcript read as it grows: a Claude one folds only the lines written since the last read, any other format is
/// read again whole only when its file changed.
pub struct Follow {
    kind: Transcripts,
    path: PathBuf,
    detail: Detail,
    stamp: Option<Stamp>,
    /// Which file it was (device and inode): one put in its place is read again from the start, never as grown.
    ident: Option<(u64, u64)>,
    /// Where the next unread line of a Claude transcript starts.
    offset: u64,
    fold: Option<ClaudeFold>,
    /// Any other format's transcript, as last read whole.
    read: Option<Transcript>,
    /// A subagent's file, read as `read_subagent` reads it.
    child: bool,
}

impl Follow {
    pub fn new(kind: Transcripts, path: &Path, detail: Detail) -> Self {
        Follow { kind, path: path.to_path_buf(), detail, stamp: None, ident: None, offset: 0, fold: None, read: None, child: false }
    }

    /// Follows a subagent's own transcript, which `new` would read as empty.
    pub fn subagent(kind: Transcripts, path: &Path, detail: Detail) -> Self {
        Follow { child: true, ..Follow::new(kind, path, detail) }
    }

    /// About how much the transcript it holds weighs in memory: its words and its calls' summaries.
    pub fn bytes(&self) -> usize {
        let held = match self.kind {
            Transcripts::Claude => self.fold.as_ref().map(|f| &f.t),
            _ => self.read.as_ref(),
        };
        held.map_or(0, |t| t.messages.iter().map(|m| m.text.len() + m.tool.as_ref().map_or(0, |c| c.summary.len())).sum())
    }

    /// The transcript as the file now holds it, as `read_detail` gives it; None when it cannot be read or holds
    /// nothing typed yet.
    pub fn read(&mut self) -> Option<&Transcript> {
        let meta = std::fs::metadata(&self.path).ok()?;
        let now = stamp(&meta);
        let ident = identity(&meta);
        if self.ident != ident {
            (self.ident, self.stamp) = (ident, None);
        }
        if self.stamp != Some(now) {
            self.update(now)?;
        }
        match self.kind {
            Transcripts::Claude => self.fold.as_ref().filter(|f| f.prompt.is_some() && !f.t.id.is_empty()).map(|f| &f.t),
            _ => self.read.as_ref(),
        }
    }

    fn update(&mut self, now: Stamp) -> Option<()> {
        // A file that only grew is read on from where the last read stopped; anything else is read again.
        let grown = self.stamp.is_some_and(|(len, _)| now.0 >= len);
        self.stamp = None;
        if self.kind != Transcripts::Claude {
            self.read = read_in(self.kind, &self.path, self.detail, self.child);
            self.stamp = Some(now);
            return Some(());
        }
        if !grown || self.fold.is_none() {
            (self.offset, self.fold) = (0, Some(ClaudeFold::new_in(self.detail, self.child)));
        }
        let fold = self.fold.as_mut()?;
        self.offset = feed_from(&self.path, self.offset, fold)?;
        fold.name();
        if let Some(title) = fold.title() {
            fold.t.title = tidy(&title);
            fold.t.kind = Transcripts::Claude;
            fold.t.path = self.path.clone();
            if fold.t.id.is_empty() {
                fold.t.id = id_of(Transcripts::Claude, &self.path).unwrap_or_default();
            }
        }
        self.stamp = Some(now);
        Some(())
    }
}

/// A file's device and inode, where the system has them.
fn identity(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        None
    }
}

/// Feeds the fold every whole line from `offset` on, too long ones skipped unread as `Lines` does, and says where the
/// first unfinished line starts.
fn feed_from(path: &Path, offset: u64, fold: &mut ClaudeFold) -> Option<u64> {
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut reader = BufReader::new(file);
    let (mut at, mut start, mut line, mut too_long) = (offset, offset, Vec::new(), false);
    loop {
        let buf = match reader.fill_buf() {
            Ok(buf) if !buf.is_empty() => buf,
            // A line still being written waits for its end.
            _ => return Some(start),
        };
        let end = buf.iter().position(|&b| b == b'\n');
        let chunk = &buf[..end.unwrap_or(buf.len())];
        too_long |= line.len() + chunk.len() > LINE_CAP;
        if !too_long {
            line.extend_from_slice(chunk);
        }
        let used = chunk.len() + usize::from(end.is_some());
        reader.consume(used);
        at += used as u64;
        if end.is_some() {
            if !too_long {
                fold.feed(&line);
            }
            (start, too_long) = (at, false);
            line.clear();
        }
    }
}

/// A subagent's transcript found beside its parent's: the id in its name, and its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub path: PathBuf,
}

/// The subagent transcripts a session's agent wrote beside it: Claude Code's `<session>/subagents/agent-<id>.jsonl`
/// (listed again only when that folder changed), Codex's rollouts spawned in the parent's folder.
pub fn subagents_of(kind: Transcripts, path: &Path, parent: &Transcript) -> Vec<Found> {
    match kind {
        Transcripts::Claude => {
            let Some(stem) = path.file_stem().map(|s| s.to_os_string()) else { return Vec::new() };
            let dir = path.with_file_name(stem).join("subagents");
            type Listed = HashMap<PathBuf, (Stamp, Vec<Found>)>;
            static LISTED: Mutex<Option<Listed>> = Mutex::new(None);
            let Ok(meta) = std::fs::metadata(&dir) else { return Vec::new() };
            let now = stamp(&meta);
            let mut listed = LISTED.lock().unwrap_or_else(|e| e.into_inner());
            let listed = listed.get_or_insert_with(HashMap::new);
            if let Some((at, found)) = listed.get(&dir)
                && *at == now
            {
                return found.clone();
            }
            let found: Vec<Found> = children(&dir)
                .into_iter()
                .filter_map(|p| {
                    let name = p.file_name()?.to_str()?;
                    let id = name.strip_prefix("agent-")?.strip_suffix(".jsonl")?;
                    (!id.is_empty() && p.is_file()).then(|| Found { id: id.to_string(), path: p.clone() })
                })
                .collect();
            if listed.len() > 256 {
                listed.clear();
            }
            listed.insert(dir, (now, found.clone()));
            found
        }
        Transcripts::Codex => {
            let Some(root) = root(kind) else { return Vec::new() };
            let since = chrono::DateTime::parse_from_rfc3339(&parent.first).map(|t| t.timestamp_millis().max(0) as u64).unwrap_or(0);
            let here = canonical(&parent.cwd);
            let mut found: Vec<(u64, Found)> = codex_days(&root, since)
                .iter()
                .flat_map(|d| children(d))
                .filter(|p| is_rollout(p) && *p != path)
                .filter_map(|p| {
                    let meta = codex_meta(&p)?;
                    let spawned = codex_subagent(&meta) && canonical(meta["cwd"].as_str().unwrap_or_default()) == here;
                    let id = id_of(kind, &p)?;
                    spawned.then(|| (modified_ms(&p), Found { id, path: p.clone() }))
                })
                .collect();
            found.sort_by_key(|(at, _)| *at);
            found.into_iter().map(|(_, f)| f).collect()
        }
        _ => Vec::new(),
    }
}

/// Lays the files found on the subagent tasks: by the id a result named, else in order, the first task without a
/// file getting the first file no task claims.
pub fn match_subagents(tasks: &mut [Task], found: &[Found]) {
    let subagents = |t: &Task| t.kind == TaskKind::Subagent;
    for task in tasks.iter_mut().filter(|t| subagents(t) && t.transcript.is_none()) {
        if let Some(f) = found.iter().find(|f| f.id == task.id) {
            task.transcript = Some(f.path.clone());
        }
    }
    let mut free: Vec<&Found> = found.iter().filter(|f| !tasks.iter().any(|t| t.transcript.as_deref() == Some(f.path.as_path()))).collect();
    for task in tasks.iter_mut().filter(|t| subagents(t) && t.transcript.is_none()) {
        if free.is_empty() {
            break;
        }
        task.transcript = Some(free.remove(0).path.clone());
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Codex: $CODEX_HOME/sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl, a record per line ({timestamp, type, payload});
// thread names in $CODEX_HOME/session_index.jsonl

fn codex_usage(info: &Value) -> Option<Usage> {
    let total = &info["total_token_usage"];
    let last = &info["last_token_usage"];
    if !total.is_object() {
        return None;
    }
    let n = |v: &Value, k: &str| v[k].as_u64().unwrap_or(0);
    Some(Usage {
        input: n(total, "input_tokens").saturating_sub(n(total, "cached_input_tokens")),
        output: n(total, "output_tokens"),
        cache_read: n(total, "cached_input_tokens"),
        cache_write: n(total, "cache_write_input_tokens"),
        context: last["total_tokens"].as_u64(),
        window: info["model_context_window"].as_u64(),
        cost: None,
    })
}

/// A rollout read whole; one Codex spawned itself is read only with `child`.
fn read_codex_in(path: &Path, detail: Detail, child: bool) -> Option<Transcript> {
    let mut t = Transcript::default();
    let mut prompt: Option<String> = None;
    let mut window = None;
    let tasks = (detail == Detail::Tools).then(|| crate::agent::tasks_of(Transcripts::Codex)).flatten();
    let mut lines = lines_of(path)?;
    let mut first_line = true;
    while let Some(raw) = lines.next() {
        let first = std::mem::take(&mut first_line);
        // Most records are tool calls, their output and reasoning: left unparsed.
        let mut names: Vec<&[u8]> = vec![&b"\"type\":\"message\""[..], b"token_count", b"turn_context", b"task_started", b"session_meta"];
        if detail == Detail::Tools {
            names.push(b"function_call");
            names.push(b"function_call_output");
        }
        let wanted = first || names.iter().any(|n| contains(raw, n));
        if !wanted {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(raw) else { continue };
        // The first rollouts held bare items, with no timestamp or payload around them.
        let (kind, p) = if v["payload"].is_object() {
            (v["type"].as_str().unwrap_or_default(), &v["payload"])
        } else {
            (if first { "session_meta" } else { "response_item" }, &v)
        };
        let at = v["timestamp"].as_str().unwrap_or_default().to_string();
        if !at.is_empty() {
            if t.first.is_empty() {
                t.first = at.clone();
            }
            t.last = at.clone();
        }
        match (kind, p["type"].as_str().unwrap_or_default()) {
            ("session_meta", _) => {
                if codex_subagent(p) && !child {
                    return None;
                }
                t.id = p["id"].as_str().unwrap_or_default().to_string();
                t.cwd = p["cwd"].as_str().unwrap_or_default().to_string();
                t.branch = p["git"]["branch"].as_str().map(str::to_string);
                if t.first.is_empty() {
                    t.first = p["timestamp"].as_str().unwrap_or_default().to_string();
                }
            }
            ("turn_context", _) => {
                if let Some(m) = p["model"].as_str().filter(|m| !m.is_empty()) {
                    t.model = Some(m.to_string());
                }
                if let Some(e) = p["effort"].as_str().filter(|e| !e.is_empty()) {
                    t.effort = Some(e.to_string());
                }
                t.mode = codex_mode(p).or(t.mode.take());
            }
            ("event_msg", "task_started") => window = p["model_context_window"].as_u64().or(window),
            ("event_msg", "token_count") => {
                if let Some(u) = codex_usage(&p["info"]) {
                    t.usage = u;
                }
            }
            ("response_item", "function_call") if detail == Detail::Tools => {
                let call = json_call(p);
                t.messages.extend(call.as_ref().and_then(|c| chunk_message(c, &at)));
                if let (Some(table), Some(Chunk::Call { id, name, summary, task, .. })) = (tasks.as_ref(), call.as_ref())
                    && let Some((kind, title)) = call_task(table, name, task.description.as_deref(), task.subagent_type.as_deref(), task.background, summary)
                {
                    t.tasks.push(Task {
                        id: id.clone(),
                        kind,
                        name: title,
                        state: TaskState::Running,
                        started: at.clone(),
                        ended: None,
                        tool_use_id: id.clone(),
                        summary: None,
                        transcript: None,
                    });
                }
            }
            ("response_item", "function_call_output") if detail == Detail::Tools => {
                let id = p["call_id"].as_str().unwrap_or_default();
                if let Some(task) = t.tasks.iter_mut().find(|task| task.tool_use_id == id) {
                    task.state = TaskState::Done;
                    task.ended = Some(at.clone());
                }
            }
            ("response_item", "message") => {
                let role = match p["role"].as_str() {
                    Some("user") => Role::User,
                    Some("assistant") => Role::Assistant,
                    _ => continue,
                };
                for block in p["content"].as_array().into_iter().flatten() {
                    let Some(text) = block["text"].as_str().filter(|_| matches!(block["type"].as_str(), Some("input_text" | "output_text" | "text"))) else {
                        continue;
                    };
                    if role == Role::User && !is_typed(text) {
                        continue;
                    }
                    if role == Role::User && prompt.is_none() {
                        prompt = Some(text.to_string());
                    }
                    t.messages.push(message(role, text, &at));
                }
            }
            _ => {}
        }
    }
    t.usage.window = t.usage.window.or(window);
    let prompt = prompt?;
    if t.id.is_empty() {
        t.id = id_of(Transcripts::Codex, path)?;
    }
    t.titled = codex_titles().remove(&t.id);
    t.opening = Some(prompt.clone());
    t.title = t.titled.clone().unwrap_or(prompt);
    Some(t)
}

/// A Codex turn's approval and sandbox policies as the permission mode Claude Code would name them.
fn codex_mode(p: &Value) -> Option<String> {
    if p["collaboration_mode"]["mode"] == "plan" {
        return Some("plan".into());
    }
    let full = p["sandbox_policy"]["type"] == "danger-full-access";
    let mode = match p["approval_policy"].as_str()? {
        "never" if full => "bypassPermissions",
        "never" => "dontAsk",
        _ if p["approvals_reviewer"] == "auto_review" => "auto",
        _ => "default",
    };
    Some(mode.into())
}

// ---------------------------------------------------------------------------------------------------------------------
// pi and oh-my-pi: <agent dir>/sessions/<folder of the cwd>/<time>_<session>.jsonl, a header then one entry per line
// (badlogic/pi-mono `session-manager.ts`). Entries form a tree; every branch's messages are read.

fn pi_context(u: &Value) -> u64 {
    ["input", "cacheRead", "cacheWrite"].iter().map(|k| u[*k].as_u64().unwrap_or(0)).sum()
}

/// A message's content as chunks: its words when a string, else its text blocks and the tool calls between them.
fn json_chunks(content: &Value, detail: Detail) -> Vec<Chunk> {
    match content {
        Value::String(s) => vec![Chunk::Text(s.clone())],
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b["type"].as_str().unwrap_or_default() {
                "text" => b["text"].as_str().map(|s| Chunk::Text(s.to_string())),
                "toolCall" | "tool_use" | "function_call" if detail == Detail::Tools => json_call(b),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A tool call read from a JSON object, whichever field names the agent uses.
fn json_call(v: &Value) -> Option<Chunk> {
    let name = ["name", "toolName", "tool"].iter().find_map(|k| v[*k].as_str()).filter(|n| !n.is_empty())?;
    let args = ["input", "arguments", "args", "parameters"].iter().map(|k| &v[*k]).find(|a| !a.is_null());
    let summary = args.map(arg_line).unwrap_or_default();
    let parsed = args.and_then(|a| a.as_str()).and_then(|s| serde_json::from_str::<Value>(s).ok());
    let args = parsed.as_ref().or(args);
    let description = args.and_then(|a| a["description"].as_str()).map(tool_line).filter(|d| !d.is_empty());
    let about = description.clone().filter(|d| *d != summary);
    let id = ["call_id", "id", "toolCallId"].iter().find_map(|k| v[*k].as_str()).unwrap_or_default().to_string();
    let task = CallTask {
        description,
        subagent_type: args.and_then(|a| a["subagent_type"].as_str().or(a["agent_type"].as_str())).map(str::to_string),
        background: args.is_some_and(|a| a["run_in_background"] == true),
    };
    Some(Chunk::Call { id, name: name.to_string(), summary, about, task })
}

/// The telling field of a call's arguments, on one line; arguments written as a JSON string are read first.
pub fn arg_line(args: &Value) -> String {
    let parsed = args.as_str().and_then(|s| serde_json::from_str::<Value>(s).ok());
    let args = parsed.as_ref().unwrap_or(args);
    for field in ["command", "cmd", "file_path", "path", "pattern", "url", "description", "prompt", "query"] {
        match &args[field] {
            Value::String(s) if !s.trim().is_empty() => return tool_line(s),
            Value::Array(parts) => {
                let joined = parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ");
                if !joined.trim().is_empty() {
                    return tool_line(&joined);
                }
            }
            _ => {}
        }
    }
    args.as_str().map(tool_line).unwrap_or_default()
}

/// A chunk as a message, when it is a call.
fn chunk_message(chunk: &Chunk, at: &str) -> Option<Message> {
    match chunk {
        Chunk::Call { id, name, summary, about, .. } => Some(tool_message(at, id, name, summary.clone(), None, about.clone())),
        _ => None,
    }
}

fn read_pi(path: &Path, detail: Detail) -> Option<Transcript> {
    let mut t = Transcript::default();
    let (mut prompt, mut named) = (None::<String>, None::<String>);
    let mut cost = 0.0;
    let mut lines = lines_of(path)?;
    while let Some(raw) = lines.next() {
        let Ok(v) = serde_json::from_slice::<Value>(raw) else { continue };
        let at = v["timestamp"].as_str().unwrap_or_default().to_string();
        if !at.is_empty() {
            if t.first.is_empty() {
                t.first = at.clone();
            }
            t.last = at.clone();
        }
        match v["type"].as_str().unwrap_or_default() {
            "session" => {
                t.id = v["id"].as_str().unwrap_or_default().to_string();
                t.cwd = v["cwd"].as_str().unwrap_or_default().to_string();
                named = v["title"].as_str().map(str::to_string).or(named);
            }
            "session_info" => named = v["name"].as_str().map(str::to_string).or(named),
            "title" | "title_change" => named = v["title"].as_str().map(str::to_string).or(named),
            "model_change" => t.model = v["modelId"].as_str().map(str::to_string).or(t.model.take()),
            "message" => {
                let m = &v["message"];
                let role = match m["role"].as_str() {
                    Some("user") => Role::User,
                    Some("assistant") => Role::Assistant,
                    _ => continue,
                };
                if role == Role::Assistant {
                    t.model = m["model"].as_str().map(str::to_string).or(t.model.take());
                    let u = &m["usage"];
                    if u.is_object() {
                        t.usage.input += u["input"].as_u64().unwrap_or(0);
                        t.usage.output += u["output"].as_u64().unwrap_or(0);
                        t.usage.cache_read += u["cacheRead"].as_u64().unwrap_or(0);
                        t.usage.cache_write += u["cacheWrite"].as_u64().unwrap_or(0);
                        t.usage.context = Some(pi_context(u));
                        cost += u["cost"]["total"].as_f64().unwrap_or(0.0);
                    }
                }
                for chunk in json_chunks(&m["content"], detail) {
                    let Chunk::Text(text) = &chunk else {
                        t.messages.extend(chunk_message(&chunk, &at));
                        continue;
                    };
                    if role == Role::User && !is_typed(text) {
                        continue;
                    }
                    if role == Role::User && prompt.is_none() {
                        prompt = Some(text.clone());
                    }
                    t.messages.push(message(role, text, &at));
                }
            }
            _ => {}
        }
    }
    t.usage.cost = (cost > 0.0).then_some(cost);
    t.titled = named.filter(|n| !n.trim().is_empty());
    t.opening = prompt.clone();
    t.title = t.titled.clone().or(prompt.clone())?;
    prompt.map(|_| t)
}

// ---------------------------------------------------------------------------------------------------------------------
// Qwen Code: <runtime>/projects/<cwd, non-alphanumerics as "-">/chats/<session>.jsonl, one record per line
// (QwenLM/qwen-code `chatRecordingService.ts`)

fn qwen_context(u: &Value) -> u64 {
    u["promptTokenCount"].as_u64().unwrap_or(0) + u["candidatesTokenCount"].as_u64().unwrap_or(0)
}

fn read_qwen(path: &Path, detail: Detail) -> Option<Transcript> {
    let mut t = Transcript::default();
    let (mut prompt, mut named) = (None::<String>, None::<String>);
    let mut lines = lines_of(path)?;
    while let Some(raw) = lines.next() {
        let Ok(v) = serde_json::from_slice::<Value>(raw) else { continue };
        let at = v["timestamp"].as_str().unwrap_or_default().to_string();
        if !at.is_empty() {
            if t.first.is_empty() {
                t.first = at.clone();
            }
            t.last = at.clone();
        }
        if t.id.is_empty() {
            t.id = v["sessionId"].as_str().unwrap_or_default().to_string();
        }
        if t.cwd.is_empty() {
            t.cwd = v["cwd"].as_str().unwrap_or_default().to_string();
        }
        if let Some(b) = v["gitBranch"].as_str().filter(|b| !b.is_empty() && *b != "HEAD") {
            t.branch = Some(b.to_string());
        }
        if v["isSidechain"] == true {
            continue;
        }
        let role = match v["type"].as_str().unwrap_or_default() {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "system" => {
                if v["subtype"] == "custom_title" {
                    named = v["systemPayload"]["customTitle"].as_str().map(str::to_string).or(named);
                }
                continue;
            }
            _ => continue,
        };
        if role == Role::Assistant {
            t.model = v["model"].as_str().map(str::to_string).or(t.model.take());
            let u = &v["usageMetadata"];
            if u.is_object() {
                let cached = u["cachedContentTokenCount"].as_u64().unwrap_or(0);
                t.usage.input += u["promptTokenCount"].as_u64().unwrap_or(0).saturating_sub(cached);
                t.usage.cache_read += cached;
                t.usage.output += u["candidatesTokenCount"].as_u64().unwrap_or(0);
                t.usage.context = Some(qwen_context(u));
            }
            t.usage.window = v["contextWindowSize"].as_u64().or(t.usage.window);
        }
        let parts = v["message"]["parts"].as_array().cloned().unwrap_or_default();
        for part in parts.iter().filter(|p| p["thought"] != true) {
            let Some(text) = part["text"].as_str() else {
                if detail == Detail::Tools {
                    t.messages.extend(json_call(&part["functionCall"]).as_ref().and_then(|c| chunk_message(c, &at)));
                }
                continue;
            };
            if role == Role::User && !is_typed(text) {
                continue;
            }
            if role == Role::User && prompt.is_none() {
                prompt = Some(text.to_string());
            }
            t.messages.push(message(role, text, &at));
        }
    }
    t.named = named.filter(|n| !n.trim().is_empty());
    t.opening = prompt.clone();
    t.title = t.named.clone().or(prompt.clone())?;
    prompt.map(|_| t)
}

// ---------------------------------------------------------------------------------------------------------------------
// GitHub Copilot CLI: $COPILOT_HOME/session-state/<session>/events.jsonl, one event per line ({type, data, timestamp};
// github/copilot-sdk's generated `session-events`)

fn read_copilot(path: &Path, detail: Detail) -> Option<Transcript> {
    let mut t = Transcript::default();
    let (mut prompt, mut named) = (None::<String>, None::<String>);
    let mut lines = lines_of(path)?;
    while let Some(raw) = lines.next() {
        let Ok(v) = serde_json::from_slice::<Value>(raw) else { continue };
        let (d, at) = (&v["data"], v["timestamp"].as_str().unwrap_or_default().to_string());
        if !at.is_empty() {
            if t.first.is_empty() {
                t.first = at.clone();
            }
            t.last = at.clone();
        }
        match v["type"].as_str().unwrap_or_default() {
            "session.start" | "session.resume" => {
                if t.id.is_empty() {
                    t.id = d["sessionId"].as_str().unwrap_or_default().to_string();
                }
                if let Some(cwd) = d["context"]["cwd"].as_str() {
                    t.cwd = cwd.to_string();
                }
                t.branch = d["context"]["branch"].as_str().map(str::to_string).or(t.branch.take());
                t.model = d["selectedModel"].as_str().map(str::to_string).or(t.model.take());
            }
            "session.title_changed" => named = d["title"].as_str().map(str::to_string).or(named),
            "session.model_change" => t.model = d["newModel"].as_str().map(str::to_string).or(t.model.take()),
            "session.usage_info" => {
                t.usage.context = d["currentTokens"].as_u64().or(t.usage.context);
                t.usage.window = d["tokenLimit"].as_u64().or(t.usage.window);
            }
            "assistant.usage" => {
                let n = |k: &str| d[k].as_u64().unwrap_or(0);
                t.usage.input += n("inputTokens");
                t.usage.output += n("outputTokens");
                t.usage.cache_read += n("cacheReadTokens");
                t.usage.cache_write += n("cacheWriteTokens");
            }
            "user.message" => {
                let Some(text) = d["content"].as_str().filter(|x| is_typed(x)) else { continue };
                if prompt.is_none() {
                    prompt = Some(text.to_string());
                }
                t.messages.push(message(Role::User, text, &at));
            }
            "assistant.message" => {
                if let Some(text) = d["content"].as_str().filter(|x| !x.trim().is_empty()) {
                    t.messages.push(message(Role::Assistant, text, &at));
                }
                if detail == Detail::Tools {
                    for req in d["toolRequests"].as_array().into_iter().flatten() {
                        t.messages.extend(json_call(req).as_ref().and_then(|c| chunk_message(c, &at)));
                    }
                }
                t.model = d["model"].as_str().map(str::to_string).or(t.model.take());
            }
            _ => {}
        }
    }
    t.titled = named.filter(|n| !n.trim().is_empty());
    t.opening = prompt.clone();
    t.title = t.titled.clone().or(prompt.clone())?;
    prompt.map(|_| t)
}

// ---------------------------------------------------------------------------------------------------------------------
// Gemini replay is adapted from google-gemini/gemini-cli chatRecordingService.ts (Apache-2.0).
// Source: https://github.com/google-gemini/gemini-cli; see THIRD_PARTY_NOTICES.md.
// Gemini CLI: $GEMINI_CLI_HOME/.gemini/tmp/<project>/chats/session-<time>-<id start>.jsonl, a header, then each message
// as a line, written again whole under the same `id` as it grows, and `$set`, `$rewindTo` and `$patch` lines
// (google-gemini/gemini-cli `chatRecordingService.ts`, `chatRecordingTypes.ts`)

/// The project folder Gemini CLI keeps in `<project>/.project_root`, beside the session's `chats`.
/// A Gemini session as last written: each message once, at its first place, with `$set` checkpoints, `$patch` updates
/// and `$rewindTo` replayed; then the summary and the session id the file states.
fn gemini_replay(path: &Path) -> Option<(Vec<Value>, Option<String>, String)> {
    let mut summary: Option<String> = None;
    let mut id_of_session = String::new();
    let mut order: Vec<String> = Vec::new();
    let mut kept: HashMap<String, Value> = HashMap::new();
    let mut lines = lines_of(path)?;
    while let Some(raw) = lines.next() {
        let Ok(v) = serde_json::from_slice::<Value>(raw) else { continue };
        if let Some(set) = v.get("$set") {
            if let Some(value) = set.get("summary") {
                summary = value.as_str().map(str::to_string);
            }
            if let Some(messages) = set["messages"].as_array() {
                gemini_checkpoint(&mut order, &mut kept, messages);
            }
            continue;
        }
        if let Some(to) = v.get("$rewindTo").and_then(Value::as_str) {
            let at = order.iter().position(|id| id == to).unwrap_or(0);
            for id in order.drain(at..) {
                kept.remove(&id);
            }
            continue;
        }
        if let Some(patch) = v.get("$patch") {
            gemini_patch(&mut kept, patch);
            for update in patch["updates"].as_array().into_iter().flatten() {
                gemini_patch(&mut kept, update);
            }
            for id in patch["removeIds"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                order.retain(|o| o != id);
                kept.remove(id);
            }
            if let Some(ids) = patch["orderIds"].as_array() {
                let mut suffix: Vec<String> = Vec::new();
                for id in ids.iter().filter_map(Value::as_str).filter(|id| kept.contains_key(*id)) {
                    if !suffix.iter().any(|s| s == id) {
                        suffix.push(id.to_string());
                    }
                }
                order.retain(|id| !suffix.contains(id));
                order.extend(suffix);
            }
            continue;
        }
        let Some(id) = v["id"].as_str().map(str::to_string) else {
            if id_of_session.is_empty() {
                id_of_session = v["sessionId"].as_str().unwrap_or_default().to_string();
            }
            summary = v["summary"].as_str().map(str::to_string).or(summary);
            if let Some(messages) = v["messages"].as_array() {
                gemini_checkpoint(&mut order, &mut kept, messages);
            }
            continue;
        };
        if !kept.contains_key(&id) {
            order.push(id.clone());
        }
        kept.insert(id, v);
    }
    let messages = order.iter().filter_map(|id| kept.remove(id)).collect();
    Some((messages, summary, id_of_session))
}

fn gemini_root(path: &Path) -> Option<String> {
    let project = path.parent()?.parent()?;
    let text = std::fs::read_to_string(project.join(".project_root")).ok()?;
    let cwd = text.trim();
    (!cwd.is_empty()).then(|| cwd.to_string())
}

/// A message's words: its content when a string, else its text parts.
fn gemini_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// What the person typed: never a command (`/…`, `?`) or the context the CLI adds itself (gemini-cli `sessionUtils.ts`).
fn gemini_typed(text: &str) -> bool {
    let t = text.trim_start();
    is_typed(t) && !t.starts_with(['/', '?']) && !t.starts_with("<session_context>") && !t.starts_with("<hook_context>")
}

/// Applies the mutable content and tool results of one Gemini message patch.
fn gemini_patch(kept: &mut HashMap<String, Value>, patch: &Value) {
    let Some(message) = patch["id"].as_str().and_then(|id| kept.get_mut(id)) else { return };
    if let Some(content) = patch.get("content") {
        message["content"] = content.clone();
    }
    for update in patch["toolCalls"].as_array().into_iter().flatten() {
        let Some(call) = message["toolCalls"].as_array_mut().and_then(|calls| calls.iter_mut().find(|c| c["id"] == update["id"])) else { continue };
        for field in ["result", "status"] {
            if let Some(value) = update.get(field) {
                call[field] = value.clone();
            }
        }
    }
}

/// Replaces the message checkpoint while preserving the order and latest version of each id.
fn gemini_checkpoint(order: &mut Vec<String>, kept: &mut HashMap<String, Value>, messages: &[Value]) {
    order.clear();
    kept.clear();
    for message in messages {
        if let Some(id) = message["id"].as_str() {
            if !kept.contains_key(id) {
                order.push(id.to_string());
            }
            kept.insert(id.to_string(), message.clone());
        }
    }
}

fn read_gemini(path: &Path, detail: Detail) -> Option<Transcript> {
    let mut t = Transcript { cwd: gemini_root(path).unwrap_or_default(), ..Transcript::default() };
    let (messages, summary, id) = gemini_replay(path)?;
    t.id = id;
    let mut prompt: Option<String> = None;
    for v in &messages {
        let at = v["timestamp"].as_str().unwrap_or_default().to_string();
        if !at.is_empty() {
            if t.first.is_empty() {
                t.first = at.clone();
            }
            t.last = at.clone();
        }
        let text = gemini_text(&v["content"]);
        match v["type"].as_str().unwrap_or_default() {
            "user" => {
                if !gemini_typed(&text) {
                    continue;
                }
                prompt.get_or_insert_with(|| text.clone());
                t.messages.push(message(Role::User, &text, &at));
            }
            "gemini" => {
                t.model = v["model"].as_str().map(str::to_string).or(t.model.take());
                let u = &v["tokens"];
                if u.is_object() {
                    let n = |k: &str| u[k].as_u64().unwrap_or(0);
                    t.usage.input += n("input").saturating_sub(n("cached"));
                    t.usage.cache_read += n("cached");
                    t.usage.output += n("output") + n("thoughts");
                    t.usage.context = Some(n("input"));
                }
                if !text.trim().is_empty() {
                    t.messages.push(message(Role::Assistant, &text, &at));
                }
                if detail == Detail::Tools {
                    for call in v["toolCalls"].as_array().into_iter().flatten() {
                        let summary = arg_line(&call["args"]);
                        let ok = match call["status"].as_str() {
                            Some("success") => Some(true),
                            Some("error" | "cancelled") => Some(false),
                            _ => None,
                        };
                        let name = call["displayName"].as_str().or(call["name"].as_str()).unwrap_or("tool");
                        t.messages.push(tool_message(&at, call["id"].as_str().unwrap_or_default(), name, summary, ok, None));
                    }
                }
            }
            _ => {}
        }
    }
    t.title = summary.filter(|n| !n.trim().is_empty()).or(prompt.clone())?;
    prompt.map(|_| t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The agents' home variables set for one test at a time, and taken away again when it ends, even by a panic.
    struct Homes(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>, Vec<&'static str>);

    impl Homes {
        fn set(vars: &[(&'static str, &Path)]) -> Homes {
            static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
            for (name, value) in vars {
                // SAFETY: every test that sets or reads these variables holds ENV.
                unsafe { std::env::set_var(name, value) };
            }
            Homes(guard, vars.iter().map(|(n, _)| *n).collect())
        }
    }

    impl Drop for Homes {
        fn drop(&mut self) {
            for name in &self.1 {
                // SAFETY: ENV is still held.
                unsafe { std::env::remove_var(name) };
            }
        }
    }

    fn write(dir: &Path, rel: &str, lines: &[Value]) -> PathBuf {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        path
    }

    use serde_json::json;

    #[test]
    fn typed_text_is_told_from_what_the_agent_put_there() {
        assert!(is_typed("Fix the quota check in acme/api"));
        assert!(is_typed("<3 this works"));
        assert!(!is_typed("<command-name>/clear</command-name>"));
        assert!(!is_typed("<environment_context>\n  <cwd>/r</cwd>"));
        assert!(!is_typed("</image>"));
        assert!(!is_typed("# AGENTS.md instructions for /r/api"));
        assert!(!is_typed("[Request interrupted by user]"));
    }

    #[test]
    fn a_pending_call_is_read_off_the_transcript_end_until_its_result_lands() {
        let dir = tempfile::tempdir().unwrap();
        let plan = json!({"type": "assistant", "timestamp": "2026-11-03T10:00:06Z", "message": {"id": "m1", "content": [{"type": "text", "text": "The plan is written."}, {"type": "tool_use", "id": "t1", "name": "ExitPlanMode", "input": {"plan": "# Daily quota\n\n1. Add the column"}}]}});
        let path = write(
            dir.path(),
            "-r-api/c-pending.jsonl",
            &[json!({"type": "user", "cwd": "/r/api", "sessionId": "c-pending", "message": {"role": "user", "content": "Plan the quota"}}), plan.clone()],
        );
        assert_eq!(pending_input(Transcripts::Claude, &path, "ExitPlanMode"), Some(json!({"plan": "# Daily quota\n\n1. Add the column"})));
        assert_eq!(pending_input(Transcripts::Claude, &path, "Bash"), None, "another tool's call is not pending");
        assert_eq!(pending_input(Transcripts::Codex, &path, "ExitPlanMode"), None, "only a format that writes the call before asking");
        // Answered: its result follows, and nothing is pending any more.
        let answered = json!({"type": "user", "timestamp": "2026-11-03T10:00:07Z", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "approved"}]}});
        let done = write(dir.path(), "-r-api/c-done.jsonl", &[plan.clone(), answered]);
        assert_eq!(pending_input(Transcripts::Claude, &done, "ExitPlanMode"), None);
        // A sidechain's call is not the agent's own.
        let aside = json!({"type": "assistant", "isSidechain": true, "message": {"id": "m2", "content": [{"type": "tool_use", "id": "t2", "name": "ExitPlanMode", "input": {"plan": "not mine"}}]}});
        let main = write(dir.path(), "-r-api/c-aside.jsonl", &[plan.clone(), aside]);
        assert_eq!(pending_input(Transcripts::Claude, &main, "ExitPlanMode"), Some(json!({"plan": "# Daily quota\n\n1. Add the column"})));
        // Two calls open at once: the prompt on the screen could be about either, so neither is the one.
        let two = json!({"type": "assistant", "message": {"id": "m3", "content": [{"type": "tool_use", "id": "t3", "name": "Bash", "input": {"command": "ls"}}, {"type": "tool_use", "id": "t4", "name": "ExitPlanMode", "input": {"plan": "x"}}]}});
        let both = write(dir.path(), "-r-api/c-two.jsonl", &[plan.clone(), two]);
        assert_eq!(pending_input(Transcripts::Claude, &both, "ExitPlanMode"), None);
        assert_eq!(plan_of_claude(&main), Some("# Daily quota\n\n1. Add the column".into()));
        assert_eq!(plan_of_claude(&both), None);
    }

    fn plan_of_claude(path: &Path) -> Option<String> {
        plan(Plan::Argument { tool: "ExitPlanMode", arg: "plan" }, Transcripts::Claude, path)
    }

    #[test]
    fn qwens_pending_plan_call_is_read_until_its_response_lands() {
        let dir = tempfile::tempdir().unwrap();
        let call = json!({"type": "assistant", "sessionId": "q-2", "message": {"role": "model", "parts": [{"text": "Here is the plan."}, {"functionCall": {"id": "f1", "name": "exit_plan_mode", "args": {"plan": "## Steps\n\n1. Add the column"}}}]}});
        let open = write(
            dir.path(),
            "-r-api/chats/q-2.jsonl",
            &[json!({"type": "user", "sessionId": "q-2", "message": {"role": "user", "parts": [{"text": "Plan it"}]}}), call.clone()],
        );
        let plan_call = Plan::Argument { tool: "exit_plan_mode", arg: "plan" };
        assert_eq!(plan(plan_call, Transcripts::Qwen, &open), Some("## Steps\n\n1. Add the column".into()));
        assert_eq!(pending_input(Transcripts::Qwen, &open, "write_file"), None);
        let reply = json!({"type": "user", "sessionId": "q-2", "message": {"role": "user", "parts": [{"functionResponse": {"id": "f1", "name": "exit_plan_mode", "response": {"output": "approved"}}}]}});
        let done = write(dir.path(), "-r-api/chats/q-2-done.jsonl", &[call.clone(), reply]);
        assert_eq!(plan(plan_call, Transcripts::Qwen, &done), None);
        // Parts without ids answer by name.
        let unnamed =
            json!({"type": "assistant", "message": {"role": "model", "parts": [{"functionCall": {"name": "exit_plan_mode", "args": {"plan": "p"}}}]}});
        let by_name = json!({"type": "user", "message": {"role": "user", "parts": [{"functionResponse": {"name": "exit_plan_mode", "response": {}}}]}});
        let named = write(dir.path(), "-r-api/chats/q-3.jsonl", &[unnamed.clone(), by_name]);
        assert_eq!(plan(plan_call, Transcripts::Qwen, &named), None);
        let still = write(dir.path(), "-r-api/chats/q-4.jsonl", &[unnamed]);
        assert_eq!(plan(plan_call, Transcripts::Qwen, &still), Some("p".into()));
    }

    #[test]
    fn geminis_pending_plan_call_names_the_plan_file_which_is_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let plans = dir.path().join("plans");
        std::fs::create_dir_all(&plans).unwrap();
        let file = plans.join("daily-quota.md");
        std::fs::write(&file, "# Daily quota\n\n1. Add the column\n").unwrap();
        let path = dir.path().join("session-2026-11-03T10-00-9d2e0000.jsonl");
        let asking = json!({"id": "m2", "type": "gemini", "content": "The plan is ready.", "toolCalls": [{"id": "c1", "name": "exit_plan_mode", "displayName": "Exit Plan Mode", "args": {"plan_path": file.to_string_lossy()}, "status": "awaiting_approval"}]});
        std::fs::write(
            &path,
            lines(&[json!({"sessionId": "9d2e0000", "projectHash": "ab12"}), json!({"id": "m1", "type": "user", "content": "Plan the quota"}), asking.clone()]),
        )
        .unwrap();
        let plan_file = Plan::File { tool: "exit_plan_mode", arg: "plan_path" };
        assert_eq!(plan(plan_file, Transcripts::Gemini, &path), Some("# Daily quota\n\n1. Add the column\n".into()));
        // Approved: the call's status lands through a patch, and the plan is no longer asked about.
        std::fs::write(
            &path,
            lines(&[
                json!({"id": "m1", "type": "user", "content": "Plan the quota"}),
                asking.clone(),
                json!({"$patch": {"id": "m2", "toolCalls": [{"id": "c1", "status": "success", "result": "ok"}]}}),
            ]),
        )
        .unwrap();
        assert_eq!(plan(plan_file, Transcripts::Gemini, &path), None);
        // A file it names that is not there is no plan.
        let gone = json!({"id": "m2", "type": "gemini", "content": "", "toolCalls": [{"id": "c2", "name": "exit_plan_mode", "args": {"plan_path": plans.join("missing.md").to_string_lossy()}}]});
        std::fs::write(&path, lines(&[gone])).unwrap();
        assert_eq!(plan(plan_file, Transcripts::Gemini, &path), None);
    }

    #[test]
    fn codexs_plan_is_its_last_message_in_plan_mode_only() {
        let dir = tempfile::tempdir().unwrap();
        let turn = |mode: Value| json!({"timestamp": "2026-11-03T10:00:00.400Z", "type": "turn_context", "payload": {"cwd": "/r/api", "model": "gpt-acme-1", "collaboration_mode": mode}});
        let said = json!({"timestamp": "2026-11-03T10:01:00.000Z", "type": "response_item", "payload": {"type": "message", "role": "assistant", "phase": "final_answer", "content": [{"type": "output_text", "text": "## Plan\n\n1. Add the column"}]}});
        let planning = write(dir.path(), "sessions/2026/11/03/rollout-plan.jsonl", &[turn(json!({"mode": "plan"})), said.clone()]);
        assert_eq!(plan(Plan::LastMessage, Transcripts::Codex, &planning), Some("## Plan\n\n1. Add the column".into()));
        let coding = write(dir.path(), "sessions/2026/11/03/rollout-code.jsonl", &[turn(json!({"mode": "default"})), said.clone()]);
        assert_eq!(plan(Plan::LastMessage, Transcripts::Codex, &coding), None, "a message in another mode is no plan");
        let unsaid = write(dir.path(), "sessions/2026/11/03/rollout-quiet.jsonl", &[turn(json!({"mode": "plan"}))]);
        assert_eq!(plan(Plan::LastMessage, Transcripts::Codex, &unsaid), None);
        assert_eq!(plan(Plan::LastMessage, Transcripts::Claude, &planning), None, "only a format whose plan mode is written down");
    }

    #[test]
    fn a_claude_transcript_reads_its_messages_model_branch_and_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let used = |i: u64, r: u64| json!({"input_tokens": i, "output_tokens": 10, "cache_read_input_tokens": r, "cache_creation_input_tokens": 5});
        let path = write(
            dir.path(),
            "-r-api/c-1.jsonl",
            &[
                json!({"type": "user", "cwd": "/r/api", "sessionId": "c-1", "gitBranch": "me/eng-412-quotas", "timestamp": "2026-11-03T10:00:00Z", "message": {"role": "user", "content": "Enforce the daily quota for ENG-412"}}),
                json!({"type": "assistant", "timestamp": "2026-11-03T10:00:05Z", "message": {"id": "m1", "model": "claude-opus-4-5", "usage": used(100, 1000), "content": [{"type": "thinking", "thinking": "hmm"}, {"type": "text", "text": "Reading the limiter."}]}}),
                json!({"type": "assistant", "timestamp": "2026-11-03T10:00:06Z", "message": {"id": "m1", "model": "claude-opus-4-5", "usage": used(100, 1000), "content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "mix test\nand more"}}]}}),
                json!({"type": "user", "timestamp": "2026-11-03T10:00:07Z", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "is_error": true, "content": "it failed"}]}}),
                json!({"type": "assistant", "timestamp": "2026-11-03T10:01:00Z", "message": {"id": "m2", "model": "claude-opus-4-5", "usage": used(50, 3000), "content": [{"type": "text", "text": "Done: the limiter counts per day."}]}}),
                json!({"type": "ai-title", "aiTitle": "Daily quota limiter"}),
            ],
        );
        let t = read(Transcripts::Claude, &path).unwrap();
        assert_eq!((t.id.as_str(), t.cwd.as_str(), t.title.as_str()), ("c-1", "/r/api", "Daily quota limiter"));
        assert_eq!((t.branch.as_deref(), t.model.as_deref()), (Some("me/eng-412-quotas"), Some("claude-opus-4-5")));
        assert_eq!(
            t.messages.iter().map(|m| (m.role, m.text.as_str())).collect::<Vec<_>>(),
            [
                (Role::User, "Enforce the daily quota for ENG-412"),
                (Role::Assistant, "Reading the limiter."),
                (Role::Assistant, "Done: the limiter counts per day.")
            ]
        );
        assert_eq!((t.usage.input, t.usage.output, t.usage.cache_read, t.usage.context), (150, 20, 4000, Some(3055)), "a streamed reply counts once");
        assert_eq!((t.first.as_str(), t.last.as_str()), ("2026-11-03T10:00:00Z", "2026-11-03T10:01:00Z"));
        assert_eq!(usage(Transcripts::Claude, &path).unwrap().context, Some(3055));

        let told = read_detail(Transcripts::Claude, &path, Detail::Tools).unwrap();
        assert_eq!(
            told.messages.iter().map(|m| (m.text.as_str(), m.tool.as_ref().map(|t| (t.name.as_str(), t.summary.as_str(), t.ok)))).collect::<Vec<_>>(),
            [
                ("Enforce the daily quota for ENG-412", None),
                ("Reading the limiter.", None),
                ("", Some(("Bash", "mix test…", Some(false)))),
                ("Done: the limiter counts per day.", None)
            ],
            "the call sits where it was made, and its result says it failed"
        );
        assert_eq!(told.text(4096), t.text(4096), "a call is not words");
    }

    /// A parent's turn that spawns a subagent (launched async, its result naming the agent), runs a job in the
    /// background, and gets both notifications.
    fn claude_task_lines() -> Vec<Value> {
        let call = |at: &str, id: &str, name: &str, input: Value| json!({"type": "assistant", "timestamp": at, "message": {"id": id, "content": [{"type": "tool_use", "id": id, "name": name, "input": input}]}});
        vec![
            json!({"type": "user", "cwd": "/r/api", "sessionId": "c-9", "timestamp": "2026-11-03T10:00:00Z", "message": {"role": "user", "content": "Find the slow quota test and run the suite meanwhile"}}),
            call(
                "2026-11-03T10:00:05Z",
                "toolu_agent",
                "Agent",
                json!({"description": "Find the slow test", "subagent_type": "Explore", "prompt": "Find the slowest test under test/quotas"}),
            ),
            json!({"type": "user", "timestamp": "2026-11-03T10:00:06Z", "message": {"role": "user", "content": [{"tool_use_id": "toolu_agent", "type": "tool_result", "content": [{"type": "text", "text": "Async agent launched successfully.\nagentId: a1f2e3d4 (internal ID)"}]}]},
                   "toolUseResult": {"isAsync": true, "status": "async_launched", "agentId": "a1f2e3d4", "description": "Find the slow test", "outputFile": "/tmp/tasks/a1f2e3d4.output"}}),
            call(
                "2026-11-03T10:00:07Z",
                "toolu_bash",
                "Bash",
                json!({"command": "mix test test/quotas", "description": "Run the quota tests", "run_in_background": true}),
            ),
            json!({"type": "user", "timestamp": "2026-11-03T10:00:08Z", "message": {"role": "user", "content": [{"tool_use_id": "toolu_bash", "type": "tool_result", "content": "Command running in background with ID: b7k2m9xq1. Output is being written to: /tmp/tasks/b7k2m9xq1.output.", "is_error": false}]},
                   "toolUseResult": {"stdout": "", "stderr": "", "interrupted": false, "backgroundTaskId": "b7k2m9xq1"}}),
            call("2026-11-03T10:00:09Z", "toolu_ls", "Bash", json!({"command": "ls test/quotas"})),
            json!({"type": "user", "timestamp": "2026-11-03T10:00:10Z", "message": {"role": "user", "content": [{"tool_use_id": "toolu_ls", "type": "tool_result", "content": "daily_test.exs", "is_error": false}]}}),
            json!({"type": "assistant", "timestamp": "2026-11-03T10:00:11Z", "message": {"id": "m3", "content": [{"type": "text", "text": "Both run in the background; I will report when they finish."}]}}),
            json!({"type": "user", "timestamp": "2026-11-03T10:02:00Z", "message": {"role": "user", "content": "<task-notification>\n<task-id>a1f2e3d4</task-id>\n<status>completed</status>\n<summary>The slowest test is daily_test.exs:41, at 3.2 s.</summary>\n<output-file>/tmp/tasks/a1f2e3d4.output</output-file>\n</task-notification>"}}),
            json!({"type": "user", "timestamp": "2026-11-03T10:02:30Z", "message": {"role": "user", "content": "<task-notification>\n<task-id>b7k2m9xq1</task-id>\n<status>failed</status>\n<summary>Background command \"Run the quota tests\" failed (exit code 1)</summary>\n</task-notification>"}}),
        ]
    }

    #[test]
    fn a_claude_transcript_lists_the_subagents_and_background_jobs_its_turns_started() {
        let dir = tempfile::tempdir().unwrap();
        let mut lines = claude_task_lines();
        let path = write(dir.path(), "-r-api/c-9.jsonl", &lines[..8]);
        let t = read_detail(Transcripts::Claude, &path, Detail::Tools).unwrap();
        fn shown(t: &Transcript) -> Vec<(TaskKind, &str, &str, TaskState, &str)> {
            t.tasks.iter().map(|x| (x.kind, x.id.as_str(), x.name.as_str(), x.state, x.tool_use_id.as_str())).collect()
        }
        assert_eq!(
            shown(&t),
            [
                (TaskKind::Subagent, "a1f2e3d4", "Find the slow test", TaskState::Running, "toolu_agent"),
                (TaskKind::Background, "b7k2m9xq1", "Run the quota tests", TaskState::Running, "toolu_bash"),
            ],
            "the results name each task's id; a plain call starts none"
        );
        assert_eq!(t.tasks[0].started, "2026-11-03T10:00:05Z");
        assert_eq!(
            t.messages.iter().filter_map(|m| m.tool.as_ref()).map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["toolu_agent", "toolu_bash", "toolu_ls"],
            "each call carries its id"
        );

        let path = write(dir.path(), "-r-api/c-9.jsonl", &lines);
        let t = read_detail(Transcripts::Claude, &path, Detail::Tools).unwrap();
        assert_eq!(shown(&t).iter().map(|x| x.3).collect::<Vec<_>>(), [TaskState::Done, TaskState::Failed], "the notifications end each task by its id");
        assert_eq!(
            (t.tasks[0].ended.as_deref(), t.tasks[0].summary.as_deref()),
            (Some("2026-11-03T10:02:00Z"), Some("The slowest test is daily_test.exs:41, at 3.2 s."))
        );
        assert_eq!(t.messages.iter().filter(|m| m.role == Role::User).count(), 1, "a notification is no message of the person's");
        assert!(read(Transcripts::Claude, &path).unwrap().tasks.is_empty(), "tasks are read with the tool calls only");

        // A spawn whose result comes back whole, and a background job that was not asked for.
        lines[2]["toolUseResult"] = json!({"status": "completed"});
        lines[2]["message"]["content"][0]["content"] = json!("The slowest test is daily_test.exs:41");
        lines[3]["message"]["content"][0]["input"]["run_in_background"] = json!(false);
        let path = write(dir.path(), "-r-api/c-10.jsonl", &lines[..6]);
        let t = read_detail(Transcripts::Claude, &path, Detail::Tools).unwrap();
        assert_eq!(
            shown(&t),
            [(TaskKind::Subagent, "toolu_agent", "Find the slow test", TaskState::Done, "toolu_agent")],
            "a synchronous spawn is over with its result"
        );
        assert_eq!(
            task_notification("<task-notification>\n<task-id>x1</task-id>\n<status>killed</status>\n</task-notification>"),
            Some(Notice { id: "x1".into(), status: "killed".into(), summary: None, output_file: None })
        );
        assert_eq!(task_notification("Fix the rounding"), None);
        assert_eq!(background_id("Command running in background with ID: b7k2m9xq1. Output is being written to: /tmp/x").as_deref(), Some("b7k2m9xq1"));
    }

    #[test]
    fn a_subagent_s_file_reads_when_asked_and_is_found_beside_its_parent() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let parent = write(dir.path(), "-r-api/c-9.jsonl", &claude_task_lines()[..8]);
        let child_lines = [
            json!({"type": "user", "isSidechain": true, "agentId": "a1f2e3d4", "sessionId": "c-9", "cwd": "/r/api", "timestamp": "2026-11-03T10:00:06Z", "message": {"role": "user", "content": "Find the slowest test under test/quotas"}}),
            json!({"type": "assistant", "isSidechain": true, "agentId": "a1f2e3d4", "timestamp": "2026-11-03T10:00:07Z", "message": {"id": "s1", "model": "claude-haiku-5-5", "content": [{"type": "tool_use", "id": "toolu_sub", "name": "Bash", "input": {"command": "mix test --slowest 1"}}]}}),
            json!({"type": "user", "isSidechain": true, "agentId": "a1f2e3d4", "timestamp": "2026-11-03T10:00:09Z", "message": {"role": "user", "content": [{"tool_use_id": "toolu_sub", "type": "tool_result", "content": "daily_test.exs:41 3.2s"}]}}),
            json!({"type": "assistant", "isSidechain": true, "agentId": "a1f2e3d4", "timestamp": "2026-11-03T10:00:10Z", "message": {"id": "s2", "content": [{"type": "text", "text": "The slowest test is daily_test.exs:41, at 3.2 s."}]}}),
        ];
        let child = write(dir.path(), "-r-api/c-9/subagents/agent-a1f2e3d4.jsonl", &child_lines);
        // A followed file reads whole lines only: the agent ends each with a newline.
        std::fs::OpenOptions::new().append(true).open(&child).unwrap().write_all(b"\n").unwrap();
        assert_eq!(read_detail(Transcripts::Claude, &child, Detail::Tools), None, "the plain reader skips sidechain lines: nothing typed");
        let t = read_subagent(Transcripts::Claude, &child, Detail::Tools).unwrap();
        assert_eq!(t.title, "Find the slowest test under test/quotas");
        assert_eq!(
            t.messages.iter().map(|m| (m.role, m.text.as_str(), m.tool.as_ref().map(|c| c.name.as_str()))).collect::<Vec<_>>(),
            [
                (Role::User, "Find the slowest test under test/quotas", None),
                (Role::Assistant, "", Some("Bash")),
                (Role::Assistant, "The slowest test is daily_test.exs:41, at 3.2 s.", None)
            ]
        );
        assert_eq!(t.model.as_deref(), Some("claude-haiku-5-5"));
        let mut follow = Follow::subagent(Transcripts::Claude, &child, Detail::Tools);
        assert_eq!(follow.read().map(|t| t.messages.len()), Some(3), "followed, the same");
        let all = all_in(Transcripts::Claude, dir.path());
        assert_eq!(all.len(), 1, "the session index lists the parent alone");
        assert!(all.contains_key(parent.to_string_lossy().as_ref()));

        let parent_t = read_detail(Transcripts::Claude, &parent, Detail::Tools).unwrap();
        let found = subagents_of(Transcripts::Claude, &parent, &parent_t);
        assert_eq!(found, [Found { id: "a1f2e3d4".into(), path: child.clone() }]);
        let mut tasks = parent_t.tasks.clone();
        match_subagents(&mut tasks, &found);
        assert_eq!(tasks[0].transcript.as_deref(), Some(child.as_path()), "matched by the id the result named");
        assert_eq!(tasks[1].transcript, None, "a background job has no file");
        // A spawn whose result named no id gets the file in order.
        let mut unnamed = parent_t.tasks.clone();
        unnamed[0].id = "toolu_agent".into();
        match_subagents(&mut unnamed, &found);
        assert_eq!(unnamed[0].transcript.as_deref(), Some(child.as_path()));
        assert!(
            subagents_of(Transcripts::Claude, &write(dir.path(), "-r-api/c-11.jsonl", &claude_task_lines()[..1]), &parent_t).is_empty(),
            "no folder, none found"
        );
    }

    #[test]
    fn a_codex_rollout_codex_spawned_reads_when_asked_and_is_found_in_its_parent_s_folder() {
        let dir = tempfile::tempdir().unwrap();
        let codex = dir.path().join("codex");
        let today = chrono::Local::now().format("%Y/%m/%d").to_string();
        let parent_id = "0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a5b";
        let child_id = "0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a5c";
        let mut parent_lines = codex_lines(parent_id, "/r/api", "main");
        parent_lines[7] = json!({"timestamp": "2026-11-03T10:00:11.000Z", "type": "response_item", "payload": {"type": "function_call", "name": "spawn_agent", "arguments": "{\"description\":\"Find the slow test\",\"prompt\":\"Find it\"}", "call_id": "c1"}});
        parent_lines[8] = json!({"timestamp": "2026-11-03T10:00:12.000Z", "type": "response_item", "payload": {"type": "function_call_output", "call_id": "c1", "output": "spawned"}});
        let parent = write(&codex, &format!("sessions/{today}/rollout-2026-11-03T10-00-00-{parent_id}.jsonl"), &parent_lines);
        let mut child_lines = codex_lines(child_id, "/r/api", "main");
        child_lines[0]["payload"]["source"] = json!({"subagent": {"thread_spawn": {"parent_thread_id": parent_id, "depth": 1}}});
        let child = write(&codex, &format!("sessions/{today}/rollout-2026-11-03T10-00-05-{child_id}.jsonl"), &child_lines);
        let _homes = Homes::set(&[("CODEX_HOME", &codex)]);
        assert_eq!(read_detail(Transcripts::Codex, &child, Detail::Tools), None, "the plain reader leaves it out");
        let t = read_subagent(Transcripts::Codex, &child, Detail::Tools).unwrap();
        assert_eq!((t.id.as_str(), t.messages.len()), (child_id, 3));
        let parent_t = read_detail(Transcripts::Codex, &parent, Detail::Tools).unwrap();
        assert_eq!(
            parent_t.tasks.iter().map(|x| (x.kind, x.name.as_str(), x.state, x.tool_use_id.as_str())).collect::<Vec<_>>(),
            [(TaskKind::Subagent, "Find the slow test", TaskState::Done, "c1")]
        );
        let found = subagents_of(Transcripts::Codex, &parent, &parent_t);
        assert_eq!(found.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), [child_id], "the rollout it spawned in the same folder, not itself");
        assert_eq!(all(Transcripts::Codex).len(), 2, "the index lists both files as before");
    }

    #[test]
    fn a_claude_transcript_keeps_a_paste_and_says_its_mode_focus_and_what_a_call_is_for() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "-r-api/c-2.jsonl",
            &[
                json!({"type": "attachment", "attachment": {"type": "prompt_snapshot", "systemPrompt": ["You are Claude Code.\n\n# Focus mode\nThe user has focus mode enabled."]}}),
                json!({"type": "user", "cwd": "/r/api", "sessionId": "c-2", "permissionMode": "plan", "timestamp": "2026-11-03T10:00:00Z", "message": {"role": "user", "content": "Fix it"}}),
                json!({"type": "permission-mode", "permissionMode": "auto", "sessionId": "c-2"}),
                json!({"type": "system", "subtype": "local_command", "content": "<local-command-stdout>Focus view disabled</local-command-stdout>"}),
                json!({"type": "system", "subtype": "local_command", "content": "<local-command-stdout>Focus view enabled</local-command-stdout>"}),
                json!({"type": "user", "timestamp": "2026-11-03T10:01:00Z", "message": {"role": "user", "content": "\n\n<pasted_content id=\"cb5e\">\n## Design Feedback\nPlaceholder is cropped\n</pasted_content>"}}),
                json!({"type": "assistant", "timestamp": "2026-11-03T10:01:05Z", "message": {"id": "m1", "content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "git diff", "description": "Show the diff"}}]}}),
            ],
        );
        let t = read_detail(Transcripts::Claude, &path, Detail::Tools).unwrap();
        assert_eq!(t.messages[1].text, "## Design Feedback\nPlaceholder is cropped", "a paste is what the person sent, its wrapper off");
        assert_eq!((t.mode.as_deref(), t.focus), (Some("auto"), Some(true)));
        assert_eq!(t.messages[2].tool.as_ref().and_then(|c| c.about.as_deref()), Some("Show the diff"));
    }

    #[test]
    fn a_followed_claude_transcript_reads_on_from_where_it_stopped_and_agrees_with_a_whole_read() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "-r-api/c-2.jsonl",
            &[
                json!({"type": "user", "cwd": "/r/api", "sessionId": "c-2", "timestamp": "2026-11-03T10:00:00Z", "message": {"role": "user", "content": "Fix the export"}}),
                json!({"type": "assistant", "timestamp": "2026-11-03T10:00:05Z", "message": {"id": "m1", "content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "mix test"}}]}}),
            ],
        );
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file).unwrap();
        let mut follow = Follow::new(Transcripts::Claude, &path, Detail::Tools);
        assert_eq!(follow.read().map(|t| t.messages.len()), Some(2));
        // The call's result lands after the call was read, and a line is still being written.
        writeln!(file, "{}", json!({"type": "user", "timestamp": "2026-11-03T10:00:07Z", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "is_error": false, "content": "ok"}]}})).unwrap();
        file.write_all(br#"{"type": "assistant", "timestamp": "2026-11-03T10:00:09Z", "message": {"content": [{"type": "text", "text": "Fixed."}]"#).unwrap();
        file.flush().unwrap();
        let read = follow.read().unwrap();
        assert_eq!(read.messages.len(), 2, "the unfinished line is not read yet");
        assert_eq!(read.messages[1].tool.as_ref().and_then(|t| t.ok), Some(true), "a result reaches a call read before it");
        writeln!(file, "}}}}").unwrap();
        let read = follow.read().unwrap().clone();
        assert_eq!(read.messages.last().map(|m| m.text.as_str()), Some("Fixed."));
        assert_eq!(read, read_detail(Transcripts::Claude, &path, Detail::Tools).unwrap(), "the same as reading it whole");
        // Another file put in its place, longer than the first: read from its start, not on from the old offset.
        let other = dir.path().join("other.jsonl");
        let mut lines = String::new();
        for n in 0..6 {
            lines.push_str(&format!("{}\n", json!({"type": "user", "cwd": "/r/api", "sessionId": "c-2", "timestamp": "2026-11-03T11:00:00Z", "message": {"role": "user", "content": format!("Replaced {n}")}})));
        }
        std::fs::write(&other, lines).unwrap();
        std::fs::rename(&other, &path).unwrap();
        let read = follow.read().unwrap();
        assert_eq!(read.messages.first().map(|m| m.text.as_str()), Some("Replaced 0"), "the new file, whole");
        assert_eq!(read.messages.len(), 6);
    }

    #[test]
    fn a_transcript_says_the_effort_and_speed_its_last_turn_ran_with() {
        let dir = tempfile::tempdir().unwrap();
        let reply = |id: &str, effort: &str, speed: &str| {
            json!({"type": "assistant", "timestamp": "2026-11-03T10:00:05Z", "effort": effort, "message": {"id": id, "model": "claude-opus-5-5",
                   "usage": {"input_tokens": 1, "output_tokens": 1, "speed": speed}, "content": [{"type": "text", "text": "Reading the limiter."}]}})
        };
        let typed = json!({"type": "user", "cwd": "/r/api", "sessionId": "c-2", "timestamp": "2026-11-03T10:00:00Z", "message": {"role": "user", "content": "Enforce the quota"}});
        let path = write(dir.path(), "-r-api/c-2.jsonl", &[typed, reply("m1", "high", "standard"), reply("m2", "xhigh", "fast")]);
        let t = read(Transcripts::Claude, &path).unwrap();
        assert_eq!((t.effort.as_deref(), t.speed.as_deref()), (Some("xhigh"), Some("fast")));
        let id = "0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a6d";
        let mut lines = codex_lines(id, "/r/api", "main");
        for line in lines.iter_mut().filter(|l| l["type"] == "turn_context") {
            line["payload"]["effort"] = json!("ultra");
        }
        let path = write(dir.path(), &format!("sessions/2026/11/03/rollout-2026-11-03T10-00-00-{id}.jsonl"), &lines);
        let t = read(Transcripts::Codex, &path).unwrap();
        assert_eq!((t.effort.as_deref(), t.speed.as_deref()), (Some("ultra"), None), "Codex's rollout names no speed");
    }

    #[test]
    fn a_codex_rollout_reads_its_meta_messages_and_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let id = "0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a5d";
        let path = write(dir.path(), &format!("sessions/2026/11/03/rollout-2026-11-03T10-00-00-{id}.jsonl"), &codex_lines(id, "/r/api", "me/eng-412-quotas"));
        let t = read(Transcripts::Codex, &path).unwrap();
        assert_eq!((t.id.as_str(), t.cwd.as_str(), t.branch.as_deref(), t.model.as_deref()), (id, "/r/api", Some("me/eng-412-quotas"), Some("gpt-acme-1")));
        assert_eq!(t.title, "Enforce the daily quota for ENG-412", "no thread name: the first prompt");
        assert_eq!(
            t.messages.iter().map(|m| (m.role, m.text.as_str())).collect::<Vec<_>>(),
            [(Role::User, "Enforce the daily quota for ENG-412"), (Role::Assistant, "Done: the limiter counts per day.")]
        );
        assert_eq!(t.messages[1].at, "2026-11-03T10:01:00.000Z");
        assert_eq!((t.usage.input, t.usage.cache_read, t.usage.output, t.usage.context, t.usage.window), (2000, 8000, 300, Some(6000), Some(200_000)));
        assert_eq!(t.usage.context_percent(), Some(3.0));
        assert_eq!(usage(Transcripts::Codex, &path), Some(t.usage.clone()));
        assert_eq!(id_of(Transcripts::Codex, &path).as_deref(), Some(id));
        assert_eq!(turn(Transcripts::Codex, &path), Some(Turn::Done));
        let mut working = codex_lines(id, "/r/api", "main");
        working.truncate(9);
        let working = write(dir.path(), "sessions/2026/11/03/rollout-2026-11-03T11-00-00-0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a5e.jsonl", &working);
        assert_eq!(turn(Transcripts::Codex, &working), Some(Turn::Working));
        assert_eq!(turn(Transcripts::Claude, &path), None);
        let mut aborted = codex_lines(id, "/r/api", "main");
        aborted.truncate(9);
        aborted.push(json!({"timestamp": "2026-11-03T11:02:00Z", "type": "event_msg", "payload": {"type": "turn_aborted", "reason": "interrupted"}}));
        let aborted = write(dir.path(), "sessions/2026/11/03/rollout-2026-11-03T12-00-00-0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a5f.jsonl", &aborted);
        assert_eq!(turn(Transcripts::Codex, &aborted), Some(Turn::Interrupted));

        let told = read_detail(Transcripts::Codex, &path, Detail::Tools).unwrap();
        assert_eq!(
            told.messages.iter().map(|m| (m.text.as_str(), m.tool.as_ref().map(|t| (t.name.as_str(), t.summary.as_str())))).collect::<Vec<_>>(),
            [("Enforce the daily quota for ENG-412", None), ("", Some(("exec_command", "rg limiter"))), ("Done: the limiter counts per day.", None)]
        );
    }

    #[test]
    fn a_claude_transcript_tells_a_finished_turn_from_a_running_one() {
        let dir = tempfile::tempdir().unwrap();
        let ended = json!({"type": "system", "subtype": "turn_duration", "durationMs": 1200, "isSidechain": false});
        let reply =
            json!({"type": "assistant", "timestamp": "2026-11-03T10:01:00Z", "message": {"role": "assistant", "content": [{"type": "text", "text": "Done."}]}});
        let working = write(dir.path(), "-r-api/c-2.jsonl", &[json!({"type": "user", "message": {"role": "user", "content": "go"}}), reply.clone()]);
        assert_eq!(turn(Transcripts::Claude, &working), Some(Turn::Working));
        let done =
            write(dir.path(), "-r-api/c-3.jsonl", &[reply.clone(), ended.clone(), json!({"type": "last-prompt", "text": "go"}), json!({"type": "cost-state"})]);
        assert_eq!(turn(Transcripts::Claude, &done), Some(Turn::Done), "the bookkeeping written after a turn says nothing");
        let sidechain = json!({"type": "assistant", "isSidechain": true, "message": {"role": "assistant", "content": []}});
        let sub = write(dir.path(), "-r-api/c-4.jsonl", &[reply, ended, sidechain]);
        assert_eq!(turn(Transcripts::Claude, &sub), Some(Turn::Done), "a subagent still writing does not reopen the turn");
        let stopped = json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "[Request interrupted by user for tool use]"}]}});
        let said = json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "Running the tests."}]}});
        let interrupted = write(dir.path(), "-r-api/c-5.jsonl", &[said.clone(), stopped.clone()]);
        assert_eq!(turn(Transcripts::Claude, &interrupted), Some(Turn::Interrupted));
        let closed = write(dir.path(), "-r-api/c-6.jsonl", &[said, stopped, json!({"type": "system", "subtype": "turn_duration", "durationMs": 900})]);
        assert_eq!(turn(Transcripts::Claude, &closed), Some(Turn::Interrupted), "closed, it still ended interrupted");
    }

    pub(crate) fn codex_lines(id: &str, cwd: &str, branch: &str) -> Vec<Value> {
        let tokens = |input: u64, cached: u64, output: u64, last: u64| {
            json!({"timestamp": "2026-11-03T10:01:01.000Z", "type": "event_msg", "payload": {"type": "token_count", "info": {
                "total_token_usage": {"input_tokens": input, "cached_input_tokens": cached, "output_tokens": output, "total_tokens": input + output},
                "last_token_usage": {"input_tokens": last, "cached_input_tokens": 0, "output_tokens": 0, "total_tokens": last},
                "model_context_window": 200_000}, "rate_limits": null}})
        };
        vec![
            json!({"timestamp": "2026-11-03T10:00:00.000Z", "type": "session_meta", "payload": {"id": id, "timestamp": "2026-11-03T10:00:00.000Z", "cwd": cwd, "source": "cli", "originator": "codex-tui", "git": {"branch": branch}}}),
            json!({"timestamp": "2026-11-03T10:00:00.100Z", "type": "event_msg", "payload": {"type": "task_started", "model_context_window": 200_000}}),
            json!({"timestamp": "2026-11-03T10:00:00.200Z", "type": "response_item", "payload": {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "<permissions instructions>sandboxed</permissions instructions>"}]}}),
            json!({"timestamp": "2026-11-03T10:00:00.300Z", "type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>\n  <cwd>/r/api</cwd>\n</environment_context>"}]}}),
            json!({"timestamp": "2026-11-03T10:00:00.400Z", "type": "turn_context", "payload": {"cwd": cwd, "model": "gpt-acme-1"}}),
            json!({"timestamp": "2026-11-03T10:00:00.500Z", "type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Enforce the daily quota for ENG-412"}]}}),
            json!({"timestamp": "2026-11-03T10:00:10.000Z", "type": "response_item", "payload": {"type": "reasoning", "summary": [], "encrypted_content": "zzz"}}),
            json!({"timestamp": "2026-11-03T10:00:11.000Z", "type": "response_item", "payload": {"type": "function_call", "name": "exec_command", "arguments": "{\"cmd\":\"rg limiter\"}", "call_id": "c1"}}),
            json!({"timestamp": "2026-11-03T10:00:12.000Z", "type": "response_item", "payload": {"type": "function_call_output", "call_id": "c1", "output": "zebrafish: lib/limiter.ex"}}),
            tokens(4000, 3000, 100, 4100),
            json!({"timestamp": "2026-11-03T10:01:00.000Z", "type": "response_item", "payload": {"type": "message", "role": "assistant", "phase": "final_answer", "content": [{"type": "output_text", "text": "Done: the limiter counts per day."}]}}),
            tokens(10_000, 8000, 300, 6000),
            json!({"timestamp": "2026-11-03T10:01:02.000Z", "type": "event_msg", "payload": {"type": "task_complete", "last_agent_message": "Done"}}),
        ]
    }

    #[test]
    fn transcripts_are_found_by_id_and_by_the_folder_a_terminal_works_in() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work/api");
        std::fs::create_dir_all(&work).unwrap();
        let cwd = work.to_string_lossy().into_owned();
        let codex = dir.path().join("codex");
        let id = "0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a5b";
        let today = chrono::Local::now().format("%Y/%m/%d").to_string();
        let path = write(&codex, &format!("sessions/{today}/rollout-2026-11-03T10-00-00-{id}.jsonl"), &codex_lines(id, &cwd, "main"));
        let mut exec = codex_lines("0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a5c", &cwd, "main");
        exec[0]["payload"]["source"] = json!("exec");
        let exec = write(&codex, &format!("sessions/{today}/rollout-2026-11-03T10-05-00-0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a5c.jsonl"), &exec);
        std::fs::write(codex.join("session_index.jsonl"), json!({"id": id, "thread_name": "Daily quotas", "updated_at": "2026-11-03T10:00:00Z"}).to_string())
            .unwrap();
        let _homes = Homes::set(&[("CODEX_HOME", &codex)]);
        assert_eq!(find(Transcripts::Codex, id, "").as_deref(), Some(path.as_path()));
        assert_eq!(locate(id, "", None).map(|(k, _)| k), Some(Transcripts::Codex));
        assert_eq!(kind_of(&path), Some(Transcripts::Codex));
        let since = modified_ms(&path).saturating_sub(60_000);
        assert_eq!(latest(Transcripts::Codex, &cwd, since).as_deref(), Some(path.as_path()), "an exec run in the same folder is not the terminal's");
        assert_eq!(latest(Transcripts::Codex, &cwd, modified_ms(&path) + 60_000), None, "a session older than the terminal is not its");
        assert_eq!(latest(Transcripts::Codex, "/r/elsewhere", since), None);
        assert_eq!(read(Transcripts::Codex, &path).unwrap().title, "Daily quotas", "the thread name Codex gave it");
        assert_eq!(all(Transcripts::Codex).len(), 2);
        assert!(all(Transcripts::Codex).contains_key(exec.to_string_lossy().as_ref()));
    }

    fn lines(v: &[Value]) -> String {
        v.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")
    }

    /// Each layered format under a home of its own: where it is, which session a folder holds, what it says.
    #[test]
    fn pi_qwen_and_copilot_sessions_read_like_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("api");
        std::fs::create_dir_all(&work).unwrap();
        let cwd = work.to_string_lossy().into_owned();
        let pi_home = dir.path().join("pi");
        let pi = pi_home.join(format!("sessions/--{}--/2026-11-03T10-00-00-000Z_p-1.jsonl", cwd.trim_start_matches('/').replace('/', "-")));
        std::fs::create_dir_all(pi.parent().unwrap()).unwrap();
        let spent = json!({"input": 10, "output": 5, "cacheRead": 9000, "cacheWrite": 0, "totalTokens": 9015, "cost": {"total": 0.004}});
        std::fs::write(&pi, lines(&[
            json!({"type": "session", "version": 3, "id": "p-1", "timestamp": "2026-11-03T10:00:00.000Z", "cwd": cwd}),
            json!({"type": "message", "id": "a1", "parentId": null, "timestamp": "2026-11-03T10:00:01.000Z", "message": {"role": "user", "content": "Rename the quota module"}}),
            json!({"type": "message", "id": "a2", "parentId": "a1", "timestamp": "2026-11-03T10:00:09.000Z", "message": {"role": "assistant", "model": "acme-large", "usage": spent,
                "content": [{"type": "thinking", "thinking": "hm"}, {"type": "toolCall", "name": "bash", "arguments": {"command": "git mv quota limits"}}, {"type": "text", "text": "Renamed to limits."}]}}),
            json!({"type": "session_info", "id": "a3", "timestamp": "2026-11-03T10:00:10.000Z", "name": "Quota rename"}),
        ])).unwrap();
        let qwen_home = dir.path().join("qwen");
        let qwen = qwen_home.join("projects/-r-api/chats/q-1.jsonl");
        std::fs::create_dir_all(qwen.parent().unwrap()).unwrap();
        std::fs::write(&qwen, lines(&[
            json!({"uuid": "u1", "sessionId": "q-1", "timestamp": "2026-11-03T10:00:00Z", "type": "user", "cwd": cwd, "gitBranch": "me/eng-412-quotas", "message": {"role": "user", "parts": [{"text": "Add tests for the limiter"}]}}),
            json!({"uuid": "u2", "sessionId": "q-1", "timestamp": "2026-11-03T10:00:07Z", "type": "assistant", "cwd": cwd, "model": "acme-coder", "message": {"role": "model", "parts": [{"text": "pondering", "thought": true}, {"functionCall": {"name": "write_file", "args": {"file_path": "test/limiter_test.exs"}}}, {"text": "Added three tests."}]},
                "usageMetadata": {"promptTokenCount": 9000, "candidatesTokenCount": 30, "cachedContentTokenCount": 8000}, "contextWindowSize": 90_300}),
            json!({"uuid": "u3", "sessionId": "q-1", "timestamp": "2026-11-03T10:00:08Z", "type": "system", "subtype": "custom_title", "cwd": cwd, "systemPayload": {"customTitle": "Limiter tests"}}),
        ])).unwrap();
        let copilot_home = dir.path().join("copilot");
        let copilot = copilot_home.join("session-state/5c1e0000-0000-4000-8000-000000000412/events.jsonl");
        std::fs::create_dir_all(copilot.parent().unwrap()).unwrap();
        std::fs::write(&copilot, lines(&[
            json!({"id": "e1", "timestamp": "2026-11-03T10:00:00.000Z", "type": "session.start", "data": {"sessionId": "5c1e0000-0000-4000-8000-000000000412", "selectedModel": "acme-mini", "context": {"cwd": cwd, "branch": "main"}}}),
            json!({"id": "e2", "timestamp": "2026-11-03T10:00:01.000Z", "type": "user.message", "data": {"content": "Run the tests"}}),
            json!({"id": "e3", "timestamp": "2026-11-03T10:00:05.000Z", "type": "assistant.message", "data": {"content": "All 42 pass.", "toolRequests": [{"name": "shell", "arguments": {"command": "mix test"}}]}}),
            json!({"id": "e4", "timestamp": "2026-11-03T10:00:05.100Z", "type": "assistant.usage", "data": {"model": "acme-mini", "inputTokens": 8000, "outputTokens": 120, "cacheReadTokens": 6000, "cacheWriteTokens": 0}}),
            json!({"id": "e5", "timestamp": "2026-11-03T10:00:05.200Z", "type": "session.usage_info", "data": {"tokenLimit": 64_000, "currentTokens": 16_000}}),
        ])).unwrap();
        let _homes = Homes::set(&[("PI_CODING_AGENT_DIR", &pi_home), ("QWEN_HOME", &qwen_home), ("COPILOT_HOME", &copilot_home)]);
        let since = modified_ms(&pi).saturating_sub(60_000);
        let p = read(Transcripts::Pi, &pi).unwrap();
        assert_eq!((p.id.as_str(), p.cwd.as_str(), p.title.as_str(), p.model.as_deref()), ("p-1", cwd.as_str(), "Quota rename", Some("acme-large")));
        assert_eq!(p.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["Rename the quota module", "Renamed to limits."]);
        assert_eq!((p.usage.context, p.usage.cost), (Some(9010), Some(0.004)));
        assert_eq!((find(Transcripts::Pi, "p-1", "").as_deref(), latest(Transcripts::Pi, &cwd, since).as_deref()), (Some(pi.as_path()), Some(pi.as_path())));
        let q = read(Transcripts::Qwen, &qwen).unwrap();
        assert_eq!(
            (q.id.as_str(), q.title.as_str(), q.branch.as_deref(), q.model.as_deref()),
            ("q-1", "Limiter tests", Some("me/eng-412-quotas"), Some("acme-coder"))
        );
        assert_eq!(q.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["Add tests for the limiter", "Added three tests."]);
        assert_eq!((q.usage.input, q.usage.cache_read, q.usage.context_percent()), (1000, 8000, Some(10.0)));
        assert_eq!(usage(Transcripts::Qwen, &qwen).unwrap().context_percent(), Some(10.0));
        assert_eq!(latest(Transcripts::Qwen, &cwd, since).as_deref(), Some(qwen.as_path()));
        let c = read(Transcripts::Copilot, &copilot).unwrap();
        assert_eq!((c.id.as_str(), c.title.as_str(), c.model.as_deref()), ("5c1e0000-0000-4000-8000-000000000412", "Run the tests", Some("acme-mini")));
        assert_eq!((c.usage.input, c.usage.context_percent()), (8000, Some(25.0)));
        assert_eq!(id_of(Transcripts::Copilot, &copilot).as_deref(), Some(c.id.as_str()));
        assert_eq!(locate(&c.id, "", None).map(|(k, _)| k), Some(Transcripts::Copilot));
        assert_eq!(all(Transcripts::Copilot).len(), 1);

        let called = |kind, path: &Path| {
            read_detail(kind, path, Detail::Tools).unwrap().messages.iter().filter_map(|m| m.tool.clone()).map(|t| (t.name, t.summary)).collect::<Vec<_>>()
        };
        assert_eq!(called(Transcripts::Pi, &pi), [("bash".to_string(), "git mv quota limits".to_string())]);
        assert_eq!(called(Transcripts::Qwen, &qwen), [("write_file".to_string(), "test/limiter_test.exs".to_string())]);
        assert_eq!(called(Transcripts::Copilot, &copilot), [("shell".to_string(), "mix test".to_string())]);
        assert_eq!(p.messages.iter().filter(|m| m.tool.is_some()).count(), 0, "words alone by default");
    }

    /// Gemini CLI's session: a header, messages written again whole as they grow, a rewind and a summary.
    #[test]
    fn a_gemini_session_reads_each_message_once_as_last_written() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("api");
        std::fs::create_dir_all(&work).unwrap();
        let cwd = work.to_string_lossy().into_owned();
        let home = dir.path().join("g");
        let project = home.join(".gemini/tmp/api");
        std::fs::create_dir_all(project.join("chats/9d2e0000-0000-4000-8000-000000000412")).unwrap();
        std::fs::write(project.join(".project_root"), format!("{cwd}\n")).unwrap();
        let id = "9d2e0000-0000-4000-8000-000000000412";
        let path = project.join("chats/session-2026-11-03T10-00-9d2e0000.jsonl");
        let used = json!({"input": 9000, "output": 40, "cached": 6000, "thoughts": 20, "tool": 0, "total": 9060});
        let call = json!([{"id": "c1", "name": "run_shell_command", "displayName": "Shell", "args": {"command": "mix test"}, "status": "success", "timestamp": "2026-11-03T10:00:04Z"}]);
        std::fs::write(&path, lines(&[
            json!({"sessionId": id, "projectHash": "ab12", "startTime": "2026-11-03T10:00:00Z", "lastUpdated": "2026-11-03T10:00:00Z"}),
            json!({"id": "m0", "timestamp": "2026-11-03T10:00:01Z", "type": "user", "content": "/model"}),
            json!({"id": "m1", "timestamp": "2026-11-03T10:00:02Z", "type": "user", "content": [{"text": "Run the quota tests"}]}),
            json!({"$set": {"lastUpdated": "2026-11-03T10:00:02Z"}}),
            json!({"id": "m2", "timestamp": "2026-11-03T10:00:03Z", "type": "gemini", "content": "Running them.", "model": "gemini-acme-pro"}),
            json!({"id": "m2", "timestamp": "2026-11-03T10:00:03Z", "type": "gemini", "content": "Running them.", "model": "gemini-acme-pro", "toolCalls": call, "tokens": used}),
            json!({"id": "m3", "timestamp": "2026-11-03T10:00:05Z", "type": "gemini", "content": "One fails.", "model": "gemini-acme-pro"}),
            json!({"$rewindTo": "m3"}),
            json!({"id": "m4", "timestamp": "2026-11-03T10:00:06Z", "type": "gemini", "content": "All 42 pass.", "model": "gemini-acme-pro"}),
            json!({"$set": {"summary": "Quota tests"}}),
        ])).unwrap();
        // A subagent's session is its parent's business.
        std::fs::write(project.join(format!("chats/{id}/sub-1.jsonl")), lines(&[json!({"sessionId": "sub-1", "kind": "subagent"})])).unwrap();
        let _homes = Homes::set(&[("GEMINI_CLI_HOME", &home)]);
        let g = read(Transcripts::Gemini, &path).unwrap();
        assert_eq!((g.id.as_str(), g.cwd.as_str(), g.title.as_str(), g.model.as_deref()), (id, cwd.as_str(), "Quota tests", Some("gemini-acme-pro")));
        assert_eq!(
            g.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            ["Run the quota tests", "Running them.", "All 42 pass."],
            "no command, no rewound answer"
        );
        assert_eq!((g.usage.input, g.usage.cache_read, g.usage.output), (3000, 6000, 60), "the message counted once, as last written");
        assert_eq!(usage(Transcripts::Gemini, &path).unwrap().context, Some(9000));
        let tools: Vec<(String, String, Option<bool>)> = read_detail(Transcripts::Gemini, &path, Detail::Tools)
            .unwrap()
            .messages
            .into_iter()
            .filter_map(|m| m.tool)
            .map(|t| (t.name, t.summary, t.ok))
            .collect();
        assert_eq!(tools, [("Shell".to_string(), "mix test".to_string(), Some(true))]);
        assert_eq!(id_of(Transcripts::Gemini, &path).as_deref(), Some(id));
        assert_eq!((find(Transcripts::Gemini, id, "").as_deref(), kind_of(&path)), (Some(path.as_path()), Some(Transcripts::Gemini)));
        let since = modified_ms(&path).saturating_sub(60_000);
        assert_eq!(latest(Transcripts::Gemini, &cwd, since).as_deref(), Some(path.as_path()), "its folder from .project_root");
        assert_eq!(all(Transcripts::Gemini).len(), 1, "the subagent's file is left out");
    }

    #[test]
    fn gemini_replays_content_patches_checkpoints_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let user = json!({"id": "u1", "type": "user", "timestamp": "2026-11-03T10:00:00Z", "content": "Check the quota"});
        let answer = |id: &str, text: &str| json!({"id": id, "type": "gemini", "timestamp": "2026-11-03T10:00:01Z", "content": text});
        std::fs::write(
            &path,
            lines(&[
                json!({"sessionId": "acme-gemini"}),
                user.clone(),
                answer("discarded", "A discarded answer"),
                json!({"$set": {"messages": [user, answer("a1", "Old answer"), answer("a2", "Second answer")]}}),
                json!({"$patch": {"id": "a1", "content": "Patched answer"}}),
                json!({"$patch": {"updates": [{"id": "a2", "content": "Updated second answer"}], "orderIds": ["a2", "a1"]}}),
            ]),
        )
        .unwrap();
        let t = read(Transcripts::Gemini, &path).unwrap();
        assert_eq!(t.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["Check the quota", "Updated second answer", "Patched answer"]);
        let mut kept = HashMap::from([("a1".into(), json!({"toolCalls": [{"id": "c1", "result": "old"}]}))]);
        gemini_patch(&mut kept, &json!({"id": "a1", "toolCalls": [{"id": "c1", "result": [{"text": "new"}]}]}));
        assert_eq!(kept["a1"]["toolCalls"][0]["result"], json!([{"text": "new"}]));
    }

    #[test]
    fn a_session_with_nothing_typed_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let mut guardian = codex_lines("0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a5f", "/r/api", "main");
        guardian[0]["payload"]["source"] = json!({"subagent": {"other": "guardian"}});
        let guardian = write(dir.path(), "sessions/2026/11/03/rollout-2026-11-03T12-00-00-0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a5f.jsonl", &guardian);
        assert_eq!(read(Transcripts::Codex, &guardian), None, "a session Codex started for itself");
        let path = write(dir.path(), "-r/c-2.jsonl", &[json!({"type": "user", "message": {"content": "<command-name>/clear</command-name>"}})]);
        assert_eq!(read(Transcripts::Claude, &path), None);
        assert_eq!(read(Transcripts::Claude, &dir.path().join("missing.jsonl")), None);
    }

    #[test]
    fn a_line_too_long_is_skipped_and_the_next_one_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s-big.jsonl");
        let big = json!({"type": "user", "message": {"content": "x".repeat(LINE_CAP)}}).to_string();
        let small = json!({"type": "user", "message": {"content": "after the picture"}}).to_string();
        std::fs::write(&path, format!("{big}\n{small}")).unwrap();
        assert_eq!(read(Transcripts::Claude, &path).unwrap().text(1000), "after the picture");
    }
}
