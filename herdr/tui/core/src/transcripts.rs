//! Agents' past sessions from the files each agent keeps: where they are, which one a terminal holds, and a reader per
//! format (`agent::Transcripts`) giving its folder, title, times, model, branch, messages, tokens and context use.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

pub use crate::agent::Transcripts;
use crate::files;

/// The most of one message kept: a pasted log or a long answer says what it is in far less.
const MESSAGE_CAP: usize = 16 * 1024;
const TITLE_CAP: usize = 100;
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
    pub branch: Option<String>,
    pub messages: Vec<Message>,
    pub usage: Usage,
}

impl Transcript {
    /// Its messages' words, one message per line, up to `cap` bytes (cut on a character).
    pub fn text(&self, cap: usize) -> String {
        let mut out = String::new();
        for m in &self.messages {
            keep(&mut out, &m.text, cap);
        }
        out
    }
}

/// A file's size and modification time in nanoseconds: it changed when either did.
pub type Stamp = (u64, u128);

/// Every format with a reader.
pub const KINDS: [Transcripts; 6] = [Transcripts::Claude, Transcripts::Codex, Transcripts::Pi, Transcripts::Omp, Transcripts::Qwen, Transcripts::Copilot];

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
        _ => Vec::new(),
    }
}

/// The folder a session of a layered format says it ran in, from its first records.
fn header_cwd(kind: Transcripts, path: &Path) -> Option<String> {
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

/// The session in a transcript; None when it holds nothing the person typed, or cannot be read.
pub fn read(kind: Transcripts, path: &Path) -> Option<Transcript> {
    let mut t = match kind {
        Transcripts::None => return None,
        Transcripts::Claude => read_claude(path)?,
        Transcripts::Codex => read_codex(path)?,
        Transcripts::Pi | Transcripts::Omp => read_pi(path)?,
        Transcripts::Qwen => read_qwen(path)?,
        Transcripts::Copilot => read_copilot(path)?,
    };
    t.kind = kind;
    t.path = path.to_path_buf();
    if t.id.is_empty() {
        t.id = id_of(kind, path)?;
    }
    t.title = tidy(&t.title);
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
    }
}

/// Where an agent stands in its session, read off its transcript's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    Working,
    /// Waiting on the person: an approval or a question, when the format records it.
    Blocked,
    Done,
}

/// Whether the agent is in a turn, from the last turn event its transcript holds: for agents that report no status to
/// Herdr. None for formats with no such events.
pub fn turn(kind: Transcripts, path: &Path) -> Option<Turn> {
    match kind {
        // Claude Code closes a turn with a `turn_duration` line; the bookkeeping it writes after says nothing.
        Transcripts::Claude => {
            let tail = tail(path, TAIL)?;
            tail.split(|b| *b == b'\n').rev().find_map(|l| {
                let v: Value = serde_json::from_slice(l).ok()?;
                if v["isSidechain"] == true {
                    return None;
                }
                match (v["type"].as_str()?, v["subtype"].as_str().unwrap_or_default()) {
                    ("system", "turn_duration") => Some(Turn::Done),
                    ("assistant" | "user", _) => Some(Turn::Working),
                    _ => None,
                }
            })
        }
        Transcripts::Codex => {
            let tail = tail(path, TAIL)?;
            tail.split(|b| *b == b'\n').rev().filter(|l| contains(l, b"\"event_msg\"")).find_map(|l| {
                let v: Value = serde_json::from_slice(l).ok()?;
                match v["payload"]["type"].as_str()? {
                    "task_started" => Some(Turn::Working),
                    "task_complete" | "turn_aborted" => Some(Turn::Done),
                    t if t.ends_with("_approval_request") || t == "request_user_input" => Some(Turn::Blocked),
                    _ => None,
                }
            })
        }
        _ => None,
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
    !text.is_empty() && !tagged && !text.starts_with("[Request interrupted") && !text.starts_with("# AGENTS.md instructions")
}

fn message(role: Role, text: &str, at: &str) -> Message {
    let mut end = text.len().min(MESSAGE_CAP);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Message { role, text: text[..end].to_string(), at: at.to_string() }
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
    message: Option<ClaudeMessage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
    #[serde(deserialize_with = "texts")]
    content: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation_input_tokens: u64,
}

/// A message's text: the content when it is a string, else its `text` blocks (tool calls, results and thinking dropped).
fn texts<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    struct Texts;
    struct Block(Option<String>);
    impl<'de> Deserialize<'de> for Block {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Fields;
            impl<'de> Visitor<'de> for Fields {
                type Value = Block;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a content block")
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Block, A::Error> {
                    let (mut kind, mut text) = (None::<String>, None::<String>);
                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "type" => kind = Some(map.next_value()?),
                            "text" => text = map.next_value()?,
                            _ => {
                                map.next_value::<IgnoredAny>()?;
                            }
                        }
                    }
                    Ok(Block(text.filter(|_| kind.as_deref() == Some("text"))))
                }
            }
            d.deserialize_any(Fields)
        }
    }
    impl<'de> Visitor<'de> for Texts {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or a list of content blocks")
        }
        fn visit_str<E>(self, s: &str) -> Result<Vec<String>, E> {
            Ok(vec![s.to_string()])
        }
        fn visit_unit<E>(self) -> Result<Vec<String>, E> {
            Ok(Vec::new())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
            let mut out = Vec::new();
            while let Some(Block(text)) = seq.next_element::<Block>()? {
                out.extend(text);
            }
            Ok(out)
        }
    }
    d.deserialize_any(Texts)
}

fn read_claude(path: &Path) -> Option<Transcript> {
    let mut t = Transcript::default();
    let (mut prompt, mut summary, mut named, mut ai_title) = (None::<String>, None, None, None);
    let mut counted: Option<String> = None;
    let mut lines = lines_of(path)?;
    while let Some(line) = lines.next() {
        let Ok(line) = serde_json::from_slice::<ClaudeLine>(line) else { continue };
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
            "summary" => summary = line.summary.or(summary),
            "custom-title" => named = line.custom_title.or(named),
            "ai-title" => ai_title = line.ai_title.or(ai_title),
            "user" | "assistant" if ![line.is_sidechain, line.is_compact_summary, line.is_meta].contains(&Some(true)) => {
                let Some(m) = line.message else { continue };
                let role = if line.kind == "user" { Role::User } else { Role::Assistant };
                if role == Role::Assistant {
                    if let Some(model) = m.model.filter(|m| !m.is_empty() && !m.starts_with('<')) {
                        t.model = Some(model);
                    }
                    // A reply streamed over several lines repeats its usage on each.
                    if let Some(u) = m.usage
                        && (m.id.is_none() || m.id != counted)
                    {
                        counted = m.id.clone();
                        t.usage.input += u.input_tokens;
                        t.usage.output += u.output_tokens;
                        t.usage.cache_read += u.cache_read_input_tokens;
                        t.usage.cache_write += u.cache_creation_input_tokens;
                        t.usage.context = Some(u.input_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens);
                    }
                }
                for text in m.content {
                    if role == Role::User && !is_typed(&text) {
                        continue;
                    }
                    if role == Role::User && prompt.is_none() {
                        prompt = Some(text.clone());
                    }
                    t.messages.push(message(role, &text, &at));
                }
            }
            _ => {}
        }
    }
    let prompt = prompt?;
    t.title = named.or(summary).or(ai_title).unwrap_or(prompt);
    Some(t)
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

fn read_codex(path: &Path) -> Option<Transcript> {
    let mut t = Transcript::default();
    let mut prompt: Option<String> = None;
    let mut window = None;
    let mut lines = lines_of(path)?;
    let mut first_line = true;
    while let Some(raw) = lines.next() {
        let first = std::mem::take(&mut first_line);
        // Most records are tool calls, their output and reasoning: left unparsed.
        let wanted =
            first || [&b"\"type\":\"message\""[..], b"token_count", b"turn_context", b"task_started", b"session_meta"].iter().any(|n| contains(raw, n));
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
                if codex_subagent(p) {
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
            }
            ("event_msg", "task_started") => window = p["model_context_window"].as_u64().or(window),
            ("event_msg", "token_count") => {
                if let Some(u) = codex_usage(&p["info"]) {
                    t.usage = u;
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
    t.title = codex_titles().remove(&t.id).unwrap_or(prompt);
    Some(t)
}

// ---------------------------------------------------------------------------------------------------------------------
// pi and oh-my-pi: <agent dir>/sessions/<folder of the cwd>/<time>_<session>.jsonl, a header then one entry per line
// (badlogic/pi-mono `session-manager.ts`). Entries form a tree; every branch's messages are read.

fn pi_context(u: &Value) -> u64 {
    ["input", "cacheRead", "cacheWrite"].iter().map(|k| u[*k].as_u64().unwrap_or(0)).sum()
}

/// A message's words: its content when a string, else its `text` blocks.
fn block_texts(content: &Value) -> Vec<String> {
    match content {
        Value::String(s) => vec![s.clone()],
        Value::Array(blocks) => blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    }
}

fn read_pi(path: &Path) -> Option<Transcript> {
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
                for text in block_texts(&m["content"]) {
                    if role == Role::User && !is_typed(&text) {
                        continue;
                    }
                    if role == Role::User && prompt.is_none() {
                        prompt = Some(text.clone());
                    }
                    t.messages.push(message(role, &text, &at));
                }
            }
            _ => {}
        }
    }
    t.usage.cost = (cost > 0.0).then_some(cost);
    t.title = named.filter(|n| !n.trim().is_empty()).or(prompt.clone())?;
    prompt.map(|_| t)
}

// ---------------------------------------------------------------------------------------------------------------------
// Qwen Code: <runtime>/projects/<cwd, non-alphanumerics as "-">/chats/<session>.jsonl, one record per line
// (QwenLM/qwen-code `chatRecordingService.ts`)

fn qwen_context(u: &Value) -> u64 {
    u["promptTokenCount"].as_u64().unwrap_or(0) + u["candidatesTokenCount"].as_u64().unwrap_or(0)
}

fn read_qwen(path: &Path) -> Option<Transcript> {
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
        for text in parts.iter().filter(|p| p["thought"] != true).filter_map(|p| p["text"].as_str()) {
            if role == Role::User && !is_typed(text) {
                continue;
            }
            if role == Role::User && prompt.is_none() {
                prompt = Some(text.to_string());
            }
            t.messages.push(message(role, text, &at));
        }
    }
    t.title = named.filter(|n| !n.trim().is_empty()).or(prompt.clone())?;
    prompt.map(|_| t)
}

// ---------------------------------------------------------------------------------------------------------------------
// GitHub Copilot CLI: $COPILOT_HOME/session-state/<session>/events.jsonl, one event per line ({type, data, timestamp};
// github/copilot-sdk's generated `session-events`)

fn read_copilot(path: &Path) -> Option<Transcript> {
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
                t.model = d["model"].as_str().map(str::to_string).or(t.model.take());
            }
            _ => {}
        }
    }
    t.title = named.filter(|n| !n.trim().is_empty()).or(prompt.clone())?;
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
    fn a_claude_transcript_reads_its_messages_model_branch_and_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let used = |i: u64, r: u64| json!({"input_tokens": i, "output_tokens": 10, "cache_read_input_tokens": r, "cache_creation_input_tokens": 5});
        let path = write(
            dir.path(),
            "-r-api/c-1.jsonl",
            &[
                json!({"type": "user", "cwd": "/r/api", "sessionId": "c-1", "gitBranch": "me/eng-412-quotas", "timestamp": "2026-11-03T10:00:00Z", "message": {"role": "user", "content": "Enforce the daily quota for ENG-412"}}),
                json!({"type": "assistant", "timestamp": "2026-11-03T10:00:05Z", "message": {"id": "m1", "model": "claude-opus-4-5", "usage": used(100, 1000), "content": [{"type": "thinking", "thinking": "hmm"}, {"type": "text", "text": "Reading the limiter."}]}}),
                json!({"type": "assistant", "timestamp": "2026-11-03T10:00:06Z", "message": {"id": "m1", "model": "claude-opus-4-5", "usage": used(100, 1000), "content": [{"type": "tool_use", "name": "Bash", "input": {}}]}}),
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
                "content": [{"type": "thinking", "thinking": "hm"}, {"type": "toolCall", "name": "bash"}, {"type": "text", "text": "Renamed to limits."}]}}),
            json!({"type": "session_info", "id": "a3", "timestamp": "2026-11-03T10:00:10.000Z", "name": "Quota rename"}),
        ])).unwrap();
        let qwen_home = dir.path().join("qwen");
        let qwen = qwen_home.join("projects/-r-api/chats/q-1.jsonl");
        std::fs::create_dir_all(qwen.parent().unwrap()).unwrap();
        std::fs::write(&qwen, lines(&[
            json!({"uuid": "u1", "sessionId": "q-1", "timestamp": "2026-11-03T10:00:00Z", "type": "user", "cwd": cwd, "gitBranch": "me/eng-412-quotas", "message": {"role": "user", "parts": [{"text": "Add tests for the limiter"}]}}),
            json!({"uuid": "u2", "sessionId": "q-1", "timestamp": "2026-11-03T10:00:07Z", "type": "assistant", "cwd": cwd, "model": "acme-coder", "message": {"role": "model", "parts": [{"text": "pondering", "thought": true}, {"text": "Added three tests."}]},
                "usageMetadata": {"promptTokenCount": 9000, "candidatesTokenCount": 30, "cachedContentTokenCount": 8000}, "contextWindowSize": 90_300}),
            json!({"uuid": "u3", "sessionId": "q-1", "timestamp": "2026-11-03T10:00:08Z", "type": "system", "subtype": "custom_title", "cwd": cwd, "systemPayload": {"customTitle": "Limiter tests"}}),
        ])).unwrap();
        let copilot_home = dir.path().join("copilot");
        let copilot = copilot_home.join("session-state/5c1e0000-0000-4000-8000-000000000412/events.jsonl");
        std::fs::create_dir_all(copilot.parent().unwrap()).unwrap();
        std::fs::write(&copilot, lines(&[
            json!({"id": "e1", "timestamp": "2026-11-03T10:00:00.000Z", "type": "session.start", "data": {"sessionId": "5c1e0000-0000-4000-8000-000000000412", "selectedModel": "acme-mini", "context": {"cwd": cwd, "branch": "main"}}}),
            json!({"id": "e2", "timestamp": "2026-11-03T10:00:01.000Z", "type": "user.message", "data": {"content": "Run the tests"}}),
            json!({"id": "e3", "timestamp": "2026-11-03T10:00:05.000Z", "type": "assistant.message", "data": {"content": "All 42 pass.", "toolRequests": []}}),
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
