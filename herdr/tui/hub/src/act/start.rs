//! Start work: a prompt, a ticket or a PR becomes a worktree, a space and an agent. `plan` says what a start does and
//! what blocks it, changing nothing; `work` runs it step by step, shows each step in the `start` view and keeps the runs
//! in `start-runs.json`, and a failed step or a Cancel removes what the start made.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, Once};
use std::time::{Duration, Instant};

use cockpit_core::agent::{self, Agent};
use cockpit_core::{accounts, files, frontend};
use serde_json::{Map, Value, json};

use super::{Ctx, arg, inbox, land, tickets};
use crate::config::{self, Config};
use cockpit_core::modes;

use crate::herdr;
use crate::proc;
use crate::value::{Res, join, list, now_ms, quote, text};

/// The steps a run reports, in the order it runs them.
const STEPS: [&str; 6] = ["fetch", "worktree", "space", "setup", "agent", "ticket"];

const SETUP_LIMIT: Duration = Duration::from_secs(60 * 60);
/// How long a fetch or a `git worktree add` may take: hooks and LFS can be slow.
const GIT_LIMIT: Duration = Duration::from_secs(10 * 60);
/// The text the agent's argv holds where its prompt goes, read from a file when the line runs.
const PROMPT_MARK: &str = "COCKPIT_START_PROMPT";

// ---------------------------------------------------------------------------------------------------------------------
// What is asked, and the world it is asked of

/// What the person asked for, as a form sends it.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub id: String,
    /// `prompt` reads the text as a prompt whatever it holds; anything else finds a PR or a ticket in its first word.
    pub source: String,
    pub text: String,
    pub repo: String,
    pub base: String,
    /// Works in the repo's own checkout instead of a new worktree.
    pub here: bool,
    pub branch: String,
    /// The worktree's folder, a name under the repo's worktrees or a full path, instead of the branch's.
    pub path: String,
    /// Takes the branch that exists already rather than asking for another name.
    pub existing: bool,
    /// Starts in a checkout holding uncommitted changes.
    pub dirty_ok: bool,
    pub agent: String,
    pub model: String,
    /// The effort level, one of the agent's; empty leaves it to the agent.
    pub effort: String,
    /// Fast mode on or off; None leaves it to the agent.
    pub fast: Option<bool>,
    pub mode: String,
    /// A session cockpit drives itself rather than an agent in a terminal.
    pub driven: bool,
    /// Full access confirmed.
    pub confirm: bool,
    /// The setup command's hash the person approved.
    pub trust: String,
    pub skip_setup: bool,
    /// Runs the setup command in a fork's tree, which is off unless asked.
    pub fork_setup: bool,
    /// Falls back on the repo of the last start; a start nobody watches pick a repo (`lastRepo: false`) does not.
    pub last_repo: bool,
    /// The new space's name, when the caller names it ("review · api") rather than the source.
    pub label: String,
    /// The next form's default repo, agent and mode stay as they were (`remember: false`).
    pub keep_defaults: bool,
    /// The config folder of the account the agent runs on; empty for the default one.
    pub account: String,
}

impl Request {
    pub fn from(v: &Value) -> Request {
        let s = |k: &str| v[k].as_str().unwrap_or_default().trim().to_string();
        Request {
            id: s("id"),
            source: s("source"),
            text: s("text"),
            repo: s("repo"),
            base: s("base"),
            here: v["worktree"] == "here",
            branch: s("branch"),
            path: s("path"),
            existing: v["existing"] == true,
            dirty_ok: v["dirty"] == true,
            agent: s("agent"),
            model: s("model"),
            effort: s("effort"),
            fast: v["fast"].as_bool(),
            mode: s("mode"),
            driven: v["driven"] == true,
            confirm: v["confirm"] == true,
            trust: s("trust"),
            skip_setup: v["setup"] == false,
            fork_setup: v["forkSetup"] == true,
            last_repo: v["lastRepo"] != false,
            label: clean(&s("label")).replace('\n', " "),
            keep_defaults: v["remember"] == false,
            account: s("account"),
        }
    }
}

/// Where spaces open and lines are typed: the person's window, or a test's record of calls.
pub(crate) trait Desk {
    /// The open space of this label: its id and the folder it shows ("" when the frontend does not say).
    fn space_named(&self, label: &str) -> Option<(String, String)>;
    /// The id of the space open on a checkout.
    fn space_on(&self, path: &str) -> Option<String>;
    /// The key of a live session cockpit drives in a checkout.
    fn session_on(&self, path: &str) -> Option<String>;
    /// Brings a space, or a session cockpit drives (`session_on`'s key), into view.
    fn focus_id(&self, space: &str);
    /// A new space in `cwd`, as its creation answered: its id ("" when unknown) and the handle of its terminal.
    fn open(&self, cwd: &str, label: &str) -> Res<(String, String)>;
    fn type_line(&self, pane: &str, line: &str) -> bool;
    fn close(&self, space: &str) -> bool;
}

/// Moves a ticket to In Progress and assigns it, as far as needed: what changed.
pub(crate) type Take = fn(&str, &Map<String, Value>) -> Res<String>;

/// What a plan and a run read: the config, the inbox, where worktrees and records go, the agents, and the outside world.
pub(crate) struct World<'a> {
    pub cfg: Config,
    pub inbox: Value,
    /// The home whose `.herdr/worktrees` takes new worktrees.
    pub home: PathBuf,
    /// Where the runs, the approved setup commands and the last choices are kept.
    pub cache: PathBuf,
    pub agents: Vec<Agent>,
    pub default_agent: String,
    pub desk: &'a dyn Desk,
    /// A PR as GitHub has it now ({repo, pr}, as `land::detail` answers).
    pub github: &'a dyn Fn(&Value) -> Res<Value>,
    /// A ticket as the tracker has it, None without a token.
    pub tracker: &'a dyn Fn(&str) -> Res<Option<Value>>,
    pub take: Take,
    /// The Claude Code accounts on this machine, the default first (`accounts::all`).
    pub accounts: Vec<accounts::Account>,
    /// The config folder the hub runs with, which a session it drives runs on.
    pub own_account: String,
}

struct RealDesk;

impl Desk for RealDesk {
    fn space_named(&self, label: &str) -> Option<(String, String)> {
        match frontend::current() {
            frontend::Frontend::App => frontend::app_spaces()
                .into_iter()
                .find(|s| s["name"] == label)
                .map(|s| (format!("{}{}", frontend::APP, text(&s["id"])), s["folder"].as_str().unwrap_or_default().to_string())),
            frontend::Frontend::Tui => herdr::spaces().into_iter().find(|w| w["label"] == label).map(|w| {
                let folder = w["worktree"]["checkout_path"].as_str().or(w["cwd"].as_str()).unwrap_or_default().to_string();
                (text(&w["workspace_id"]), folder)
            }),
        }
    }

    fn space_on(&self, path: &str) -> Option<String> {
        inbox::space_on(path).map(|w| text(&w["workspace_id"]))
    }

    fn session_on(&self, path: &str) -> Option<String> {
        let real = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
        let wanted = real(path);
        driven_sessions().into_iter().find(|s| s["cwd"].as_str().is_some_and(|c| !c.is_empty() && real(c) == wanted)).map(|s| text(&s["key"]))
    }

    fn focus_id(&self, space: &str) {
        if !driven_sessions().iter().any(|s| s["key"] == space) {
            return inbox::focus_space(space);
        }
        // The window opens its chat; the TUI lists no session it drives, so there is nothing to bring up there.
        if frontend::current() == frontend::Frontend::App {
            frontend::app_do(&format!("cockpit.via:chat|chat:open:{space}"), &json!({}));
        }
    }

    fn open(&self, cwd: &str, label: &str) -> Res<(String, String)> {
        match frontend::current() {
            frontend::Frontend::App => {
                let made = frontend::app_do("space.new", &json!({"name": label, "folder": cwd})).ok_or("the Herdr app opened no space")?;
                frontend::app_show_work();
                let terminal = made["terminal"].as_str().filter(|t| !t.is_empty()).ok_or("the new space has no terminal")?;
                let space = made["space"].as_str().map(str::to_string).or_else(|| frontend::app_place(terminal).map(|(s, _)| s));
                Ok((space.map(|s| format!("{}{s}", frontend::APP)).unwrap_or_default(), format!("{}{terminal}", frontend::APP)))
            }
            frontend::Frontend::Tui => {
                let made = frontend::herdr(&["workspace", "create", "--cwd", cwd, "--label", label, "--focus"]).ok_or("herdr opened no space")?;
                let pane = made["root_pane"]["pane_id"].as_str().ok_or("the new space has no pane")?.to_string();
                let space = made["workspace"]["workspace_id"].as_str().or(made["workspace_id"].as_str()).or(made["root_pane"]["workspace_id"].as_str());
                Ok((space.unwrap_or_default().to_string(), pane))
            }
        }
    }

    fn type_line(&self, pane: &str, line: &str) -> bool {
        frontend::run_in(pane, line)
    }

    fn close(&self, space: &str) -> bool {
        !space.is_empty() && frontend::close_space(space)
    }
}

/// The live sessions cockpit drives, as the `session` view lists them (`{key, cwd, …}`).
fn driven_sessions() -> Vec<Value> {
    list(&crate::views::build("session", &json!({"limit": 1}))["sessions"]).to_vec()
}

/// The runs this machine kept, read back once per hub: before the first view, plan or action.
fn loaded() {
    static LOADED: Once = Once::new();
    if !cfg!(test) {
        LOADED.call_once(|| load_runs(&files::cache()));
    }
}

/// The real world: this machine's config, inbox, window, GitHub and tracker.
fn real<R>(f: impl FnOnce(&World) -> R) -> R {
    loaded();
    let cfg = config::load();
    let github = |pr: &Value| land::detail(&cfg, pr);
    let cache = files::cache();
    let last = files::load(&cache.join("start.json")).unwrap_or(Value::Null);
    let default_agent = last["agent"].as_str().filter(|a| agent::get(a).is_some()).map(str::to_string).unwrap_or_else(|| agent::default().id);
    let world = World {
        cfg: cfg.clone(),
        inbox: config::read_inbox(),
        home: files::home(),
        cache,
        agents: agent::all(),
        default_agent,
        desk: &RealDesk,
        github: &github,
        tracker: &tickets::issue,
        take: tickets::take,
        accounts: accounts::all(),
        own_account: accounts::own(),
    };
    f(&world)
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading the source

/// What a start is about.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Prompt,
    /// {id, title, url, statusType, branch}
    Ticket(Value),
    /// A word shaped like a ticket's key that neither the inbox nor the config knows: the tracker decides.
    Maybe(String),
    /// {url, owner, repo, number, title, branch, base, cross, headOwner, headOid}; no branch until GitHub or the inbox
    /// says it.
    Pr(Value),
}

fn is_key(word: &str) -> bool {
    let Some((letters, digits)) = word.rsplit_once('-') else { return false };
    !letters.is_empty()
        && letters.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && letters.chars().all(|c| c.is_ascii_alphanumeric())
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
}

/// The inbox's PRs, each kind of list at once.
fn inbox_prs(inbox: &Value) -> impl Iterator<Item = &Value> {
    ["mine", "direct", "team"].into_iter().flat_map(|k| list(&inbox[k]).iter())
}

/// A PR's URL read as {url, owner, repo, number}.
fn pr_url(word: &str) -> Option<Value> {
    let url = word.trim_end_matches('/');
    let parts: Vec<&str> = url.split('/').collect();
    let at = parts.iter().position(|p| *p == "pull")?;
    let number: u64 = parts.get(at + 1)?.parse().ok()?;
    (at >= 2 && url.starts_with("http")).then(|| json!({"url": url, "owner": parts[at - 2], "repo": parts[at - 1], "number": number}))
}

fn ticket_of(t: &Value) -> Value {
    json!({
        "id": text(&t["id"]).to_uppercase(), "title": t["title"], "url": t["url"], "statusType": t["statusType"],
        "branch": t["branch"].as_str().or(t["branchName"].as_str()).unwrap_or_default(),
    })
}

/// The source as a target: a PR by its GitHub URL, a ticket the inbox or the config's prefixes know by its key or URL,
/// a key-shaped word for the tracker to confirm, else a prompt; and the words after the PR or ticket, which the agent
/// gets too.
fn resolve(req: &Request, inbox: &Value, cfg: &Config) -> (Target, String) {
    if req.source == "prompt" {
        return (Target::Prompt, req.text.clone());
    }
    let mut words = req.text.splitn(2, char::is_whitespace);
    let first = words.next().unwrap_or_default().trim().to_string();
    let rest = words.next().unwrap_or_default().trim().to_string();
    if let Some(parsed) = pr_url(&first) {
        let url = parsed["url"].as_str().unwrap_or_default();
        let known = inbox_prs(inbox).find(|pr| pr["url"].as_str().map(|u| u.trim_end_matches('/')) == Some(url));
        let mut pr = parsed.clone();
        if let Some(known) = known {
            pr["repo"] = json!(short(known["repo"].as_str().unwrap_or_default()));
            for (to, from) in
                [("title", "title"), ("branch", "branch"), ("base", "base"), ("cross", "isCrossRepository"), ("headOwner", "headOwner"), ("headOid", "headOid")]
            {
                pr[to] = known[from].clone();
            }
        }
        return (Target::Pr(pr), rest);
    }
    let tickets = list(&inbox["tickets"]);
    let named = |k: &str| tickets.iter().find(|t| t["id"].as_str().is_some_and(|id| id.eq_ignore_ascii_case(k)));
    let prefixes: Vec<String> = list(&cfg.0["tickets"]["keys"]).iter().map(|k| text(k).to_uppercase()).collect();
    let prefixed = |k: &str| k.rsplit_once('-').is_some_and(|(p, _)| prefixes.contains(&p.to_uppercase()));
    if first.starts_with("http") {
        if let Some(t) = tickets.iter().find(|t| t["url"].as_str().is_some_and(|u| u.trim_end_matches('/') == first.trim_end_matches('/'))) {
            return (Target::Ticket(ticket_of(t)), rest);
        }
        if let Some(key) = first.trim_end_matches('/').split('/').find(|seg| is_key(seg)) {
            return match named(key) {
                Some(t) => (Target::Ticket(ticket_of(t)), rest),
                None if prefixed(key) => (Target::Ticket(json!({"id": key.to_uppercase(), "url": first, "branch": ""})), rest),
                None => (Target::Maybe(key.to_uppercase()), rest),
            };
        }
        return (Target::Prompt, req.text.clone());
    }
    if !is_key(&first) {
        return (Target::Prompt, req.text.clone());
    }
    match named(&first) {
        Some(t) => (Target::Ticket(ticket_of(t)), rest),
        None if prefixed(&first) => (Target::Ticket(json!({"id": first.to_uppercase(), "branch": ""})), rest),
        None => (Target::Maybe(first.to_uppercase()), rest),
    }
}

/// `api` from `acme/api`.
fn short(repo: &str) -> String {
    repo.rsplit('/').next().unwrap_or_default().to_string()
}

/// The text without control characters (a newline and a tab stay), for what goes to an agent from a title or a prompt.
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect()
}

/// Whether an id is one to use in a file name.
fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

// ---------------------------------------------------------------------------------------------------------------------
// Names

/// Words a slug leaves out, so the ones that say something fit.
const FILLER: [&str; 12] = ["a", "an", "the", "to", "of", "for", "and", "in", "on", "with", "at", "by"];

/// Up to five words of the text that say something, lowercased, as a branch's or a space's name.
pub fn slug(words: &str) -> String {
    let lowered = words.to_lowercase();
    let kept: Vec<String> =
        lowered.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty() && !FILLER.contains(w)).take(5).map(str::to_string).collect();
    let mut out = String::new();
    for word in kept {
        if !out.is_empty() && out.len() + word.len() + 1 > 40 {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&word);
    }
    out
}

/// A branch name with its empty parts gone: `/-fix` from a template with no prefix or key reads `fix`.
fn tidy(name: &str) -> String {
    let parts: Vec<String> = name
        .split('/')
        .map(|part| {
            let mut kept = String::new();
            for c in part.chars().filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)) {
                if c == '-' && (kept.is_empty() || kept.ends_with('-')) {
                    continue;
                }
                kept.push(c);
            }
            kept.trim_matches(['-', '.']).to_string()
        })
        .filter(|p| !p.is_empty())
        .collect();
    parts.join("/")
}

/// The branch a ticket or a prompt gets: the config's `start.branch` template filled with `start.prefix`, the ticket's
/// key and the words' slug; empty when none of them says anything.
pub fn branch_name(cfg: &Config, key: &str, words: &str) -> String {
    let start = &cfg.0["start"];
    let template = start["branch"].as_str().filter(|t| !t.is_empty()).unwrap_or("{prefix}/{key}-{slug}");
    let prefix = start["prefix"].as_str().unwrap_or_default();
    let (key, slug) = (key.to_lowercase(), slug(words));
    if key.is_empty() && slug.is_empty() {
        return String::new();
    }
    tidy(&template.replace("{prefix}", prefix).replace("{key}", &key).replace("{slug}", &slug))
}

/// A stable hash of a setup command, to tell when it changed.
pub fn hash(command: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in command.trim().bytes() {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

// ---------------------------------------------------------------------------------------------------------------------
// git

fn git(dir: &Path, args: &[&str]) -> proc::Out {
    land::git(dir, args)
}

/// A git call that may take minutes: a fetch, a worktree's checkout with its hooks.
fn git_long(dir: &Path, args: &[&str]) -> proc::Out {
    let argv: Vec<String> = ["git", "-C", &dir.to_string_lossy()].into_iter().map(str::to_string).chain(args.iter().map(|a| a.to_string())).collect();
    proc::run(&argv, GIT_LIMIT, None)
}

fn has_ref(main: &Path, name: &str) -> bool {
    git(main, &["rev-parse", "--verify", "-q", name]).ok()
}

fn tip(main: &Path, name: &str) -> String {
    git(main, &["rev-parse", "--verify", "-q", name]).stdout.trim().to_string()
}

/// The remote to fetch from: the one whose URL names `owner/name` on GitHub when given, else origin, else the only one.
fn remote_of(main: &Path, owner_repo: Option<(&str, &str)>) -> Option<String> {
    let names: Vec<String> = git(main, &["remote"]).stdout.lines().map(str::to_string).filter(|n| !n.is_empty()).collect();
    if let Some((owner, name)) = owner_repo {
        let want = format!("{}/{}", owner.to_lowercase(), name.to_lowercase());
        for remote in &names {
            let url = git(main, &["remote", "get-url", remote]).stdout.trim().to_lowercase();
            let path = url.trim_end_matches(".git").trim_end_matches('/');
            if path.ends_with(&format!("/{want}")) || path.ends_with(&format!(":{want}")) {
                return Some(remote.clone());
            }
        }
    }
    if names.iter().any(|n| n == "origin") {
        return Some("origin".into());
    }
    (names.len() == 1).then(|| names[0].clone())
}

/// The remote's default branch as its HEAD names it, else the only one of main and master it has; None when it says
/// neither.
fn default_base(main: &Path, remote: &str) -> Option<String> {
    let head = git(main, &["symbolic-ref", "--quiet", "--short", &format!("refs/remotes/{remote}/HEAD")]);
    if let Some(name) = head.stdout.trim().strip_prefix(&format!("{remote}/")).filter(|_| head.ok()) {
        return Some(name.to_string());
    }
    let found: Vec<&str> = ["main", "master"].into_iter().filter(|b| has_ref(main, &format!("refs/remotes/{remote}/{b}"))).collect();
    (found.len() == 1).then(|| found[0].to_string())
}

/// A few of the remote's branches, to offer as a base.
fn remote_branches(main: &Path, remote: &str) -> Vec<String> {
    let listed = git(main, &["for-each-ref", "--format=%(refname:short)", "--count=6", &format!("refs/remotes/{remote}/")]);
    listed.stdout.lines().filter_map(|l| l.strip_prefix(&format!("{remote}/"))).filter(|b| *b != "HEAD").map(str::to_string).collect()
}

/// The first of `<branch>-2`, `<branch>-3`… that names no branch and no worktree folder.
fn free_name(main: &Path, home: &Path, repo: &str, remote: &str, branch: &str) -> String {
    (2..100)
        .map(|n| format!("{branch}-{n}"))
        .find(|b| !has_ref(main, &format!("refs/heads/{b}")) && !has_ref(main, &format!("refs/remotes/{remote}/{b}")) && !worktree_path(home, repo, b).exists())
        .unwrap_or_else(|| format!("{branch}-{}", now_ms()))
}

/// Where a worktree of the branch goes: herdr's convention, `~/.herdr/worktrees/<repo>/<branch>`.
fn worktree_path(home: &Path, repo: &str, branch: &str) -> PathBuf {
    home.join(".herdr/worktrees").join(repo).join(branch.replace('/', "-"))
}

/// The first `<folder>-2`, `<folder>-3`… free beside it.
fn free_folder(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    (2..100).map(|n| path.with_file_name(format!("{name}-{n}"))).find(|p| !p.exists()).unwrap_or_else(|| path.with_file_name(format!("{name}-{}", now_ms())))
}

/// The repo's checkouts in the repos folder, by name.
fn checkouts(cfg: &Config) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(cfg.repos())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(".git").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// Whether a folder is the checkout `main` or one of its worktrees.
fn same_repo(folder: &str, main: &Path) -> bool {
    if folder.is_empty() {
        return true;
    }
    let common = git(Path::new(folder), &["rev-parse", "--path-format=absolute", "--git-common-dir"]).stdout.trim().to_string();
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    !common.is_empty() && real(Path::new(&common)) == real(&main.join(".git"))
}

// ---------------------------------------------------------------------------------------------------------------------
// The plan

#[derive(Clone, Debug, Default)]
pub(crate) struct Blocker {
    pub step: &'static str,
    pub text: String,
    /// Each fix's label and what it sets in the request.
    pub fixes: Vec<(String, Value)>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Plan {
    pub kind: &'static str,
    pub title: String,
    pub label: String,
    /// The next form's default repo, agent and mode stay as they were.
    pub keep_defaults: bool,
    pub repo: String,
    pub main: PathBuf,
    pub remote: String,
    pub base: String,
    pub here: bool,
    pub branch: String,
    /// Where the branch name comes from: pr | typed | tracker | template | current.
    pub branch_from: &'static str,
    pub path: String,
    /// worktree | checkout | here | space | focus
    pub place: &'static str,
    /// The worktree takes a new branch from <remote>/<base>.
    pub new_branch: bool,
    /// The branch exists on the remote only, and is tracked from there.
    pub remote_only: bool,
    /// The branch is on the remote, and fetched before its worktree is added.
    pub on_remote: bool,
    /// A fork's PR: its number, fetched as pull/N/head into `refs/remotes/<remote>/pr/N` for the branch `pr/N`.
    pub cross: Option<String>,
    /// The setup command and its hash.
    pub setup: Option<(String, String)>,
    pub agent: Option<Agent>,
    pub model: String,
    /// The effort level passed to the agent: empty when none was picked or it takes none.
    pub effort: String,
    /// Fast mode passed to the agent: None when not picked or it has none.
    pub fast: Option<bool>,
    /// The mode passed to the agent: empty when its table row takes none.
    pub mode: String,
    /// The config folder of the account the agent runs on, set in its `account_env`: empty when it has one account.
    pub account: String,
    /// That account's name.
    pub account_name: String,
    pub driven: bool,
    pub confirm: bool,
    pub prompt: String,
    pub ticket: Option<Value>,
    pub issue: Option<Value>,
    /// The space to bring into view, when place is `space` or `focus`.
    pub space: String,
    /// That space is a session cockpit drives, opened rather than focused.
    pub session: bool,
    pub steps: Vec<(&'static str, String)>,
    pub blockers: Vec<Blocker>,
    /// Choices that block nothing: read the source as a prompt, run a fork's setup.
    pub options: Vec<(String, Value)>,
    pub notes: Vec<String>,
    pub repos: Vec<String>,
}

impl Plan {
    fn block(&mut self, step: &'static str, text: impl Into<String>, fixes: Vec<(&str, Value)>) {
        self.blockers.push(Blocker { step, text: text.into(), fixes: fixes.into_iter().map(|(l, v)| (l.to_string(), v)).collect() });
    }

    pub fn ok(&self) -> bool {
        self.blockers.is_empty()
    }

    fn has(&self, step: &str) -> bool {
        self.steps.iter().any(|(s, _)| *s == step)
    }

    /// What the agent is told it works on, in a few words.
    fn what(&self) -> &'static str {
        match self.kind {
            "ticket" => "the ticket",
            "pr" => "the PR",
            _ => "your prompt",
        }
    }

    pub fn json(&self) -> Value {
        let agent = self.agent.as_ref();
        let pairs = |list: &[(String, Value)]| list.iter().map(|(l, set)| json!({"label": l, "set": set})).collect::<Vec<_>>();
        json!({
            "ok": self.ok(), "kind": self.kind, "title": self.title, "label": self.label, "place": self.place,
            "repo": self.repo, "main": self.main, "remote": self.remote, "base": self.base, "worktree": if self.here { "here" } else { "new" },
            "branch": self.branch, "branchFrom": self.branch_from, "newBranch": self.new_branch, "path": self.path,
            "setup": self.setup.as_ref().map(|(c, h)| json!({"command": c, "hash": h})),
            "agent": agent.map(|a| a.id.clone()), "agentName": agent.map(|a| a.name.clone()), "model": self.model, "effort": self.effort, "fast": self.fast, "mode": self.mode,
            "account": (!self.account.is_empty()).then_some(&self.account), "accountName": (!self.account.is_empty()).then_some(&self.account_name),
            "driven": self.driven, "session": self.session, "prompt": self.prompt, "cross": self.cross.is_some(),
            "steps": self.steps.iter().map(|(id, t)| json!({"id": id, "text": t})).collect::<Vec<_>>(),
            "blockers": self.blockers.iter().map(|b| json!({"step": b.step, "text": b.text, "fixes": pairs(&b.fixes)})).collect::<Vec<_>>(),
            "options": pairs(&self.options),
            "notes": self.notes,
        })
    }
}

/// The modes an agent takes at start, as its table row says (`modes::json`, the ones its menus offer): none when its
/// `mode` column is empty.
fn modes_of(agent: &Agent) -> Vec<Value> {
    if agent.mode.is_none() {
        return Vec::new();
    }
    let help = cockpit_core::switches::of(agent, || {});
    modes::json(modes::words_of(agent), &help.modes).as_array().into_iter().flatten().filter(|m| m["offered"] == true).cloned().collect()
}

/// The choices a form offers: the repos, the installed agents (the default first) with the modes each takes and what a
/// person picks for it beside the prompt (`picks`, its catalog's models once read), with what it would pick itself, and
/// the accounts it can run on when this machine has several (`accounts`, the default first).
fn choices(world: &World) -> Value {
    let mut agents: Vec<&Agent> = world.agents.iter().filter(|a| a.installed() || a.id == world.default_agent).collect();
    let accounts: Vec<Value> = if world.accounts.len() > 1 { world.accounts.iter().map(accounts::Account::json).collect() } else { Vec::new() };
    agents.sort_by_key(|a| a.id != world.default_agent);
    let last = files::load(&world.cache.join("start.json")).unwrap_or(Value::Null);
    json!({
        "repos": checkouts(&world.cfg),
        // `drives`: cockpit can run it as a session of its own (a form starts it so), else it runs in a terminal.
        "agents": agents
            .iter()
            .map(|a| json!({"id": a.id, "name": a.name, "icon": agent::icon_in(&world.agents, &a.id), "installed": a.installed(), "models": !a.model.is_empty(), "modes": modes_of(a), "drives": a.stream.is_some(), "picks": a.catalog_picks(|| {}),
                "accounts": if a.account_env.is_some() { accounts.clone() } else { Vec::new() }}))
            .collect::<Vec<_>>(),
        "agent": world.default_agent, "repo": last["repo"], "mode": last["mode"].as_str().unwrap_or("default"),
    })
}

/// The label a space of this start gets, made unique against a space of that label on another repo.
fn label_for(world: &World, label: &str, main: &Path, repo: &str) -> (String, Option<String>) {
    match world.desk.space_named(label) {
        Some((id, folder)) if same_repo(&folder, main) => (label.to_string(), Some(id)),
        Some(_) => {
            let other = format!("{label} ({repo})");
            let open = world.desk.space_named(&other).filter(|(_, f)| same_repo(f, main)).map(|(id, _)| id);
            (other, open)
        }
        None => (label.to_string(), None),
    }
}

/// What a start would do for this request and what blocks it, changing nothing.
pub(crate) fn plan_in(world: &World, req: &Request) -> Plan {
    let mut p = Plan {
        model: req.model.clone(),
        effort: req.effort.clone(),
        fast: req.fast,
        driven: req.driven,
        confirm: req.confirm,
        here: req.here,
        keep_defaults: req.keep_defaults,
        ..Plan::default()
    };
    p.repos = checkouts(&world.cfg);
    let (mut target, extra) = resolve(req, &world.inbox, &world.cfg);
    let tracker_name = world.cfg.0["tickets"]["name"].as_str().unwrap_or("Linear").to_string();
    let mut issue: Option<Value> = None;
    if let Target::Maybe(key) = &target {
        target = match (world.tracker)(key) {
            Ok(Some(found)) => {
                let ticket = json!({"id": key, "title": found["title"], "url": found["url"], "statusType": found["state"]["type"], "branch": ""});
                issue = Some(found);
                Target::Ticket(ticket)
            }
            _ => Target::Prompt,
        };
    }
    let text = if matches!(target, Target::Prompt) { req.text.clone() } else { extra.clone() };
    let last = files::load(&world.cache.join("start.json")).unwrap_or(Value::Null);
    let mut repo = req.repo.clone();
    let mut fixed_branch: Option<(String, &'static str)> = None;
    let mut ask = String::new();
    if !matches!(target, Target::Prompt) {
        p.options.push(("Use as a prompt".into(), json!({"source": "prompt"})));
    }
    match &target {
        Target::Prompt | Target::Maybe(_) => {
            p.kind = "prompt";
            p.title = req.text.lines().next().unwrap_or_default().chars().take(80).collect();
        }
        Target::Ticket(t) => {
            p.kind = "ticket";
            let id = text_of(&t["id"]);
            p.label = named_or(req, &id.to_lowercase());
            p.title = format!("{id} {}", t["title"].as_str().unwrap_or_default()).trim().to_string();
            let mut ticket = t.clone();
            let looked = match issue.take() {
                Some(found) => Ok(Some(found)),
                None => (world.tracker)(&id),
            };
            match looked {
                Ok(Some(issue)) => {
                    if ticket["title"].is_null() {
                        ticket["title"] = issue["title"].clone();
                        ticket["url"] = issue["url"].clone();
                        p.title = format!("{id} {}", text_of(&issue["title"]));
                    }
                    if let Some(b) = issue["branchName"].as_str().filter(|b| !b.is_empty()) {
                        ticket["branch"] = json!(b);
                    }
                    match issue["state"]["type"].as_str() {
                        Some("completed" | "canceled") => {
                            p.block("source", format!("{id} is {}: reopen it first", issue["state"]["name"].as_str().unwrap_or("closed")), vec![])
                        }
                        Some("started") => {}
                        _ => p.steps.push(("ticket", format!("move {id} to In Progress"))),
                    }
                    match issue["assignee"].as_object() {
                        None => p.steps.push(("ticket", format!("assign {id} to you"))),
                        Some(a) if a.get("isMe") != Some(&json!(true)) => p.block(
                            "source",
                            format!(
                                "{id} is assigned to {}: take it over in {tracker_name} first",
                                a.get("displayName").and_then(Value::as_str).unwrap_or("someone else")
                            ),
                            vec![],
                        ),
                        Some(_) => {}
                    }
                    p.issue = Some(issue);
                }
                Ok(None) => {
                    if ticket["statusType"] != "started" {
                        ask = format!(" First move {id} to In Progress and assign it to me in {tracker_name}.");
                        p.steps.push(("ticket", format!("the agent moves {id} to In Progress ({tracker_name} has no token here)")));
                    }
                }
                Err(e) => {
                    ask = format!(" First move {id} to In Progress and assign it to me in {tracker_name}.");
                    p.notes.push(format!("{tracker_name} did not answer ({e}): the agent moves {id} itself"));
                }
            }
            let mine = list(&world.inbox["mine"]).iter().find(|pr| pr["branch"].as_str().is_some_and(|b| land::names_ticket(b, &id)));
            if let Some(pr) = mine {
                let pr_repo = short(pr["repo"].as_str().unwrap_or_default());
                if repo.is_empty() || repo == pr_repo {
                    repo = pr_repo;
                    fixed_branch = Some((text_of(&pr["branch"]), "pr"));
                    p.notes.push(format!("{id} has a PR already: {}", text_of(&pr["url"])));
                }
            }
            if fixed_branch.is_none()
                && let Some(b) = ticket["branch"].as_str().filter(|b| !b.is_empty())
            {
                fixed_branch = Some((b.to_string(), "tracker"));
            }
            p.ticket = Some(ticket);
        }
        Target::Pr(pr) => {
            p.kind = "pr";
            let mut pr = pr.clone();
            repo = short(pr["repo"].as_str().unwrap_or_default());
            let named = format!("{repo}#{}", text_of(&pr["number"]));
            p.label = named_or(req, &named);
            p.title = format!("{named} {}", pr["title"].as_str().unwrap_or_default()).trim().to_string();
            // Whether it comes from a fork is GitHub's to say when the inbox did not.
            if pr["branch"].as_str().unwrap_or_default().is_empty() || !pr["cross"].is_boolean() {
                match (world.github)(&json!({"url": pr["url"]})) {
                    Ok(d) => {
                        let found = &d["pr"];
                        repo = short(d["repo"]["nameWithOwner"].as_str().unwrap_or(&repo));
                        pr["title"] = found["title"].clone();
                        pr["branch"] = found["headRefName"].clone();
                        pr["base"] = found["baseRefName"].clone();
                        pr["cross"] = json!(found["isCrossRepository"] == true);
                        pr["headOwner"] = found["headRepositoryOwner"]["login"].clone();
                        pr["headOid"] = found["headRefOid"].clone();
                        p.title = format!("{repo}#{} {}", text_of(&pr["number"]), pr["title"].as_str().unwrap_or_default()).trim().to_string();
                        if found["state"] != "OPEN" {
                            let state = found["state"].as_str().unwrap_or("closed").to_lowercase();
                            p.block("source", format!("{} is {state}", p.label), vec![]);
                        }
                    }
                    Err(e) => {
                        p.repo = repo;
                        p.agent = agent_checks(world, req, &mut p);
                        p.block("github", format!("GitHub did not answer: {e}"), vec![("Try again", json!({}))]);
                        return p;
                    }
                }
            }
            if pr["cross"] == true {
                p.cross = Some(text_of(&pr["number"]));
                fixed_branch = Some((format!("pr/{}", text_of(&pr["number"])), "pr"));
            } else {
                fixed_branch = Some((text_of(&pr["branch"]), "pr"));
            }
            p.ticket = Some(pr);
        }
    }
    if repo.is_empty() && req.last_repo {
        let only = (p.repos.len() == 1).then(|| p.repos[0].as_str());
        repo = last["repo"].as_str().filter(|r| p.repos.iter().any(|x| x == r)).or(only).unwrap_or_default().to_string();
    }
    p.agent = agent_checks(world, req, &mut p);
    if repo.is_empty() {
        let repos = p.repos.clone();
        let why = if req.last_repo { "Pick the repo it works in" } else { "Pick the repo it works in: start it from the Start work form" };
        p.block("repo", why, repos.iter().take(6).map(|r| (r.as_str(), json!({"repo": r}))).collect());
        return p;
    }
    if !p.repos.contains(&repo) {
        p.block("repo", format!("No checkout of {repo} in {}: clone it there first", world.cfg.repos()), vec![]);
        p.repo = repo;
        return p;
    }
    let main = PathBuf::from(world.cfg.repos()).join(&repo);
    p.repo = repo.clone();
    p.main = main.clone();
    let owner_repo = match &target {
        Target::Pr(pr) => pr["owner"].as_str().zip(pr["repo"].as_str()).map(|(o, r)| (o.to_string(), short(r))),
        _ => None,
    };
    let Some(remote) = remote_of(&main, owner_repo.as_ref().map(|(o, r)| (o.as_str(), r.as_str()))) else {
        p.block("fetch", format!("{repo} has no remote to fetch from"), vec![]);
        return p;
    };
    p.remote = remote.clone();
    if !p.label.is_empty() {
        let (label, open) = label_for(world, &p.label, &main, &repo);
        p.label = label;
        if let Some(space) = open {
            return focus_plan(p, space);
        }
    }
    p.base = if !req.base.is_empty() {
        req.base.clone()
    } else if let Target::Pr(pr) = &target
        && let Some(b) = pr["base"].as_str().filter(|b| !b.is_empty())
    {
        b.to_string()
    } else {
        default_base(&main, &remote).unwrap_or_default()
    };
    let dirty = git(&main, &["--no-optional-locks", "status", "--porcelain"]).stdout.lines().count();
    let changes = format!("{dirty} uncommitted change{}", if dirty == 1 { "" } else { "s" });
    if req.here {
        here_plan(world, req, &mut p, &target, fixed_branch.as_ref(), dirty, &changes);
    } else {
        worktree_plan(world, req, &mut p, &target, fixed_branch, dirty, &changes, &text);
    }
    if p.place == "focus" || p.place == "space" {
        return p;
    }
    p.prompt = clean(&prompt_for(&p, &target, &ask, &extra, &text));
    let agent_name = p.agent.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| "the agent".into());
    let mode_word = p.agent.as_ref().and_then(|a| modes::of(modes::words_of(a), &p.mode)).map(|m| m.label.to_lowercase()).unwrap_or_default();
    let effort = if p.effort.is_empty() { String::new() } else { format!("{} effort", p.effort) };
    let fast = match p.fast {
        Some(true) => "fast mode".to_string(),
        Some(false) => "standard speed".to_string(),
        None => String::new(),
    };
    let account = if p.account.is_empty() { String::new() } else { format!("{} account", p.account_name) };
    let with = [account, p.model.clone(), effort, fast, mode_word].into_iter().filter(|w| !w.is_empty() && w != "asks first").collect::<Vec<_>>();
    let how = if with.is_empty() { String::new() } else { format!(" ({})", with.join(", ")) };
    if p.driven {
        p.steps.push(("agent", format!("start {agent_name}{how} as a session cockpit drives, on {}", p.what())));
    } else {
        let named = if p.label.is_empty() { String::new() } else { format!(" \u{201c}{}\u{201d}", p.label) };
        p.steps.push(("space", format!("open a space{named} there")));
        let takes = p.agent.as_ref().is_none_or(Agent::start_takes_prompt);
        let on = match (p.prompt.is_empty(), takes) {
            (true, _) => " with no prompt".to_string(),
            (false, true) => format!(" on {}", p.what()),
            // It starts with no prompt: the words on the work go through its queue, once it rests.
            (false, false) => format!(", then queue {} for it ({agent_name} takes no prompt as it starts)", p.what()),
        };
        p.steps.push(("agent", format!("start {agent_name}{how}{on}")));
    }
    p.steps.sort_by_key(|(id, _)| STEPS.iter().position(|s| s == id));
    p
}

/// A JSON value as text, without quotes for a string.
fn text_of(v: &Value) -> String {
    text(v)
}

/// The label the caller gave, else the one the source makes.
fn named_or(req: &Request, made: &str) -> String {
    if req.label.is_empty() { made.to_string() } else { req.label.clone() }
}

fn focus_plan(mut p: Plan, space: String) -> Plan {
    p.place = "focus";
    p.blockers.clear();
    p.options.clear();
    p.space = space;
    p.steps = vec![("space", format!("\u{201c}{}\u{201d} already runs: bring it into view", p.label))];
    p
}

/// The agent asked for (else the default) and the mode and model it takes, and whatever keeps it from starting as asked.
fn agent_checks(world: &World, req: &Request, p: &mut Plan) -> Option<Agent> {
    let id = if req.agent.is_empty() { world.default_agent.as_str() } else { req.agent.as_str() };
    let Some(found) = world.agents.iter().find(|a| a.id == id || a.aliases.iter().any(|x| x == id)).cloned() else {
        p.block("agent", format!("No agent {id:?} in cockpit's table"), vec![]);
        return None;
    };
    if !found.installed() {
        p.block("agent", format!("{} is not installed: {} is not found", found.name, found.bin), vec![]);
    }
    if req.driven && found.stream.is_none() {
        p.block("agent", format!("cockpit drives {} only in a terminal", found.name), vec![("Start it in a terminal", json!({"driven": false}))]);
    }
    if !req.model.is_empty() && (found.model.is_empty() || !agent::model_id(&req.model) || !found.takes_model(&req.model)) {
        p.notes.push(format!("{} does not take the model {}: it starts on its own", found.name, req.model));
        p.model.clear();
    }
    if !req.effort.is_empty() && found.effort_argv(&req.effort).is_none() {
        p.notes.push(format!("{} takes no effort level {} at start: it starts on its own", found.name, req.effort));
        p.effort.clear();
    }
    if req.fast.is_some() && found.fast.is_none() {
        p.notes.push(format!("{} has no fast mode: it starts at its own speed", found.name));
        p.fast = None;
    }
    account_checks(world, req, p, &found);
    p.mode = if found.mode.is_some() { if req.mode.is_empty() { "default".into() } else { req.mode.clone() } } else { String::new() };
    let words = modes::words_of(&found);
    let known = modes::of(words, &p.mode);
    if !p.mode.is_empty() && known.is_none() {
        p.block("agent", format!("No mode {:?}: one of {}", p.mode, modes::ids(words).join(", ")), vec![("Ask first", json!({"mode": "default"}))]);
        p.mode = "default".into();
    } else if let Some(mode) = known.filter(|m| m.unasked && !req.confirm) {
        // "Full access", "auto mode": a one-word label is a mode's name.
        let name = if mode.label.contains(' ') { mode.label.to_lowercase() } else { format!("{} mode", mode.label.to_lowercase()) };
        let said = format!("{}{} lets the agent run tools without asking", name[..1].to_uppercase(), &name[1..]);
        p.block("agent", said, vec![(&format!("Allow {name}"), json!({"confirm": true}))]);
    }
    Some(found)
}

/// The account the agent runs on, when it takes one and this machine has several: the one asked for, else the default.
/// A session cockpit drives runs on the hub's own account only.
fn account_checks(world: &World, req: &Request, p: &mut Plan, found: &Agent) {
    if found.account_env.is_none() || world.accounts.len() < 2 {
        if !req.account.is_empty() {
            p.notes.push(format!("{} runs on one account here: it starts on its own", found.name));
        }
        return;
    }
    let wanted = if req.account.is_empty() { None } else { Some(accounts::key(&req.account)) };
    let picked = match &wanted {
        Some(dir) => world.accounts.iter().find(|a| a.dir == *dir),
        None => world.accounts.iter().find(|a| a.default).or(world.accounts.first()),
    };
    let Some(account) = picked else {
        let folder = accounts::shown(wanted.as_deref().unwrap_or_default());
        p.block("agent", format!("No {} account in {folder}", found.name), vec![("Use the default account", json!({"account": ""}))]);
        return;
    };
    (p.account, p.account_name) = (account.dir.clone(), account.name.clone());
    if req.driven && account.dir != world.own_account {
        p.block(
            "agent",
            format!("cockpit drives {} only on the account it runs with, not {}", found.name, account.name),
            vec![("Start it in a terminal", json!({"driven": false}))],
        );
    }
}

/// Working in the repo's own checkout: on its current branch, refused among uncommitted changes unless asked.
fn here_plan(world: &World, req: &Request, p: &mut Plan, target: &Target, fixed: Option<&(String, &'static str)>, dirty: usize, changes: &str) {
    p.place = "here";
    p.path = p.main.to_string_lossy().into_owned();
    p.branch = git(&p.main, &["branch", "--show-current"]).stdout.trim().to_string();
    p.branch_from = "current";
    if p.label.is_empty() {
        let named = named_or(req, &slug(&req.text));
        let (label, open) = label_for(world, if named.is_empty() { &p.repo } else { &named }, &p.main.clone(), &p.repo.clone());
        p.label = label;
        if let Some(space) = open {
            *p = focus_plan(std::mem::take(p), space);
            return;
        }
    }
    let on = if p.branch.is_empty() { "a detached HEAD".to_string() } else { p.branch.clone() };
    if let (Target::Pr(_), Some((branch, _))) = (target, fixed)
        && *branch != p.branch
    {
        p.block("worktree", format!("Your checkout of {} is on {on}, not {branch}", p.repo), vec![("Use a new worktree", json!({"worktree": "new"}))]);
    }
    if dirty > 0 && !req.dirty_ok {
        p.block(
            "worktree",
            format!("Your checkout of {} has {changes}, which the agent would work on top of", p.repo),
            vec![("Use a new worktree", json!({"worktree": "new"})), ("Start there anyway", json!({"dirty": true}))],
        );
    }
    if p.branch.is_empty() {
        p.notes.push(format!("Your checkout of {} is on a detached HEAD", p.repo));
    }
}

/// A worktree of the branch: a new branch from <remote>/<base>, the PR's or an existing one, or the checkout that has it.
#[allow(clippy::too_many_arguments)]
fn worktree_plan(world: &World, req: &Request, p: &mut Plan, target: &Target, fixed: Option<(String, &'static str)>, dirty: usize, changes: &str, words: &str) {
    let main = p.main.clone();
    let remote = p.remote.clone();
    let (branch, from) = match fixed {
        Some((b, "pr")) => (b, "pr"),
        _ if !req.branch.is_empty() => (req.branch.clone(), "typed"),
        Some(found) => found,
        None => {
            let (key, words) = match target {
                Target::Ticket(t) => (text_of(&t["id"]), t["title"].as_str().unwrap_or_default().to_string()),
                _ => (String::new(), words.to_string()),
            };
            (branch_name(&world.cfg, &key, &words), "template")
        }
    };
    p.branch = branch.clone();
    p.branch_from = from;
    if branch.is_empty() {
        let stamp = chrono_stamp();
        let named = tidy(&format!("{}/work-{stamp}", world.cfg.0["start"]["prefix"].as_str().unwrap_or_default()));
        let step = if req.text.trim().is_empty() { "empty" } else { "branch" };
        let why = if step == "empty" { "Type a prompt, a ticket's key or a PR's URL, or start in the checkout with no prompt" } else { "Name the branch" };
        p.block(step, why, vec![("Start in the checkout", json!({"worktree": "here"})), (&format!("Name it {named}"), json!({"branch": named}))]);
        return;
    }
    if p.label.is_empty() {
        let (label, open) = label_for(world, &named_or(req, branch.rsplit('/').next().unwrap_or(&branch)), &main, &p.repo.clone());
        p.label = label;
        if let Some(space) = open {
            *p = focus_plan(std::mem::take(p), space);
            return;
        }
    }
    if !git(&main, &["check-ref-format", "--branch", &branch]).ok() {
        p.block("branch", format!("{branch:?} is not a name git takes for a branch"), vec![]);
        return;
    }
    let local = has_ref(&main, &format!("refs/heads/{branch}"));
    let remote_has = has_ref(&main, &format!("refs/remotes/{remote}/{branch}"));
    let pr_head = match target {
        Target::Pr(pr) => pr["headOid"].as_str().unwrap_or_default().to_string(),
        _ => String::new(),
    };
    if let Some(found) = inbox::checkout_in(&main.to_string_lossy(), &branch).filter(|_| p.cross.is_none() || tip(&main, &branch) == pr_head) {
        p.path = found.clone();
        if let Some(space) = world.desk.space_on(&found) {
            *p = focus_plan(std::mem::take(p), space);
            p.place = "space";
            p.steps = vec![("space", format!("{branch} is open in a space already: bring it into view"))];
        } else if let Some(key) = world.desk.session_on(&found) {
            *p = focus_plan(std::mem::take(p), key);
            (p.place, p.session) = ("space", true);
            p.steps = vec![("space", format!("a session cockpit drives works on {branch} already: bring it up"))];
        } else {
            p.place = "checkout";
            p.notes.push(format!("{branch} is checked out in {found} already: the agent starts there"));
        }
        return;
    }
    p.place = "worktree";
    if p.base.is_empty() && from != "pr" {
        let offered = remote_branches(&main, &remote);
        p.block("base", format!("{remote} names no default branch: pick the base"), offered.iter().take(4).map(|b| (b.as_str(), json!({"base": b}))).collect());
    } else if from != "pr" && !req.base.is_empty() && !has_ref(&main, &format!("refs/remotes/{remote}/{}", p.base)) {
        let heads = git(&main, &["ls-remote", "--heads", &remote, &p.base]);
        if heads.ok() && heads.stdout.trim().is_empty() {
            p.block("base", format!("{remote} has no branch {}", p.base), vec![]);
        } else if !heads.ok() {
            p.notes.push(format!("{remote}/{} is not fetched yet and {remote} did not answer: the fetch step tells", p.base));
        }
    }
    let path = match req.path.as_str() {
        "" => worktree_path(&world.home, &p.repo, &branch),
        given if Path::new(given).is_absolute() => PathBuf::from(given),
        given => world.home.join(".herdr/worktrees").join(&p.repo).join(given.replace('/', "-")),
    };
    p.path = path.to_string_lossy().into_owned();
    if let Some(n) = p.cross.clone() {
        p.on_remote = true;
        if local && tip(&main, &branch) != pr_head {
            p.block("branch", format!("A branch {branch} is here already and is not this PR's head: rename or remove it first"), vec![]);
        }
        p.new_branch = !local;
        p.notes.push(format!("#{n} comes from a fork: its branch is fetched as {branch}, never onto a branch of yours"));
        p.notes.push("Your checkout's git hooks (post-checkout) run on the fork's code when its worktree is added".into());
    } else if from == "pr" {
        p.on_remote = true;
        p.remote_only = !local;
    } else if local || remote_has {
        if req.existing {
            p.remote_only = !local;
            p.on_remote = remote_has;
        } else {
            let place = if local { "" } else { " on the remote" };
            let other = free_name(&main, &world.home, &p.repo, &remote, &branch);
            p.block(
                "branch",
                format!("The branch {branch} exists already{place}"),
                vec![("Use it", json!({"existing": true})), ("Rename", json!({"branch": other}))],
            );
        }
    } else {
        p.new_branch = true;
    }
    if path.exists() {
        let other = free_folder(&path);
        let mut fixes = vec![("Use another folder", json!({"path": other.to_string_lossy()}))];
        let renamed = free_name(&main, &world.home, &p.repo, &remote, &branch);
        if from != "pr" {
            fixes.push(("Rename the branch", json!({"branch": renamed})));
        }
        p.block("worktree", format!("{} exists already and is not a worktree of {branch}", p.path), fixes);
    }
    if dirty > 0 {
        p.notes.push(format!("Your checkout of {} has {changes}: the worktree starts from {remote} without them", p.repo));
    }
    let fetched = match (&p.cross, p.new_branch) {
        (Some(n), _) => Some(format!("fetch the fork's pull/{n}/head into {remote}/pr/{n}")),
        (None, true) => Some(format!("fetch {remote}/{}", p.base)),
        (None, false) => p.on_remote.then(|| format!("fetch {branch} from {remote}")),
    };
    if let Some(fetched) = fetched {
        p.steps.push(("fetch", fetched));
    }
    let made = if p.new_branch && p.cross.is_none() { format!("a new branch {branch} from {remote}/{}", p.base) } else { format!("the branch {branch}") };
    p.steps.push(("worktree", format!("add a worktree at {} on {made}", p.path)));
    let command = world.cfg.0["start"]["setup"][&p.repo].as_str().map(str::trim).filter(|c| !c.is_empty());
    let fork_off = p.cross.is_some() && !req.fork_setup;
    if let Some(command) = command.filter(|_| fork_off) {
        p.options.push(("Run setup from the fork's tree".into(), json!({"forkSetup": true})));
        p.notes.push(format!("{}'s setup ({command}) is off for a fork's code", p.repo));
    }
    if let Some(command) = command.filter(|_| !req.skip_setup && !fork_off) {
        let h = hash(command);
        let approved = files::load(&world.cache.join("start-trust.json")).unwrap_or(Value::Null);
        let trusted = approved[main.to_string_lossy().as_ref()].as_str() == Some(h.as_str()) || req.trust == h;
        if !trusted {
            let seen = if approved[main.to_string_lossy().as_ref()].is_string() { "changed since you last ran it" } else { "is new here" };
            p.block(
                "setup",
                format!("{}'s setup command {seen}: {command}", p.repo),
                vec![("Run it", json!({"trust": h})), ("Skip it this time", json!({"setup": false}))],
            );
        }
        p.steps.push(("setup", format!("run \u{201c}{command}\u{201d} in it, then the agent once it succeeds")));
        p.setup = Some((command.to_string(), h));
    }
}

/// A short stamp of now, `1112-0605`, for a branch nothing names.
fn chrono_stamp() -> String {
    chrono::Local::now().format("%m%d-%H%M").to_string()
}

/// The first message the agent gets.
fn prompt_for(p: &Plan, target: &Target, ask: &str, extra: &str, text: &str) -> String {
    let extra = if extra.is_empty() { String::new() } else { format!(" {extra}") };
    let place = match p.place {
        "worktree" if p.new_branch && p.cross.is_none() => {
            format!("a new worktree of {} at {}, on the branch {} made from {}/{}", p.repo, p.path, p.branch, p.remote, p.base)
        }
        "worktree" | "checkout" => format!("a worktree of {} at {}, on the branch {}", p.repo, p.path, p.branch),
        _ => format!("your checkout of {} at {}, on {}", p.repo, p.path, if p.branch.is_empty() { "a detached HEAD" } else { &p.branch }),
    };
    match target {
        Target::Prompt | Target::Maybe(_) => text.to_string(),
        Target::Ticket(t) => format!(
            "Start {} ({}): {}.{ask} You are in {place}. Read the ticket and propose a plan before coding.{extra}",
            text_of(&t["id"]),
            t["url"].as_str().unwrap_or_default(),
            t["title"].as_str().unwrap_or_default()
        ),
        Target::Pr(pr) => format!(
            "Work on {} ({}). You are in {place}. Read the PR's state, review comments and CI, and summarize where it stands before changing anything.{extra}",
            text_of(&pr["url"]),
            pr["title"].as_str().unwrap_or_default()
        ),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Runs

#[derive(Clone, Debug, Default)]
struct Made {
    /// The main checkout the worktree and the branch belong to.
    main: PathBuf,
    worktree: Option<String>,
    /// The branch the start made, and the commit it made it at.
    branch: Option<(String, String)>,
    space: Option<String>,
    pane: Option<String>,
    /// The space was handed over (its terminal ran the line): what is in the worktree may be the person's now.
    handed: bool,
    /// The line typed into the pane, typed again on Retry.
    line: String,
    marker: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// The act runs its steps.
    Steps,
    /// The steps are done; the setup command runs in the terminal, or the hub runs it for a driven session.
    Setup,
    Over,
}

#[derive(Clone, Debug)]
struct Run {
    id: String,
    label: String,
    title: String,
    state: &'static str,
    phase: Phase,
    steps: Vec<(String, String, &'static str, String)>,
    error: Option<(String, String)>,
    request: Value,
    made: Made,
    cancel: bool,
    at: i64,
    cache: PathBuf,
    /// The tracker's issue, moved once setup succeeds.
    issue: Option<Map<String, Value>>,
    take: Option<Take>,
}

const STATES: [&str; 4] = ["running", "done", "failed", "cancelled"];

impl Run {
    /// Its setup runs in a terminal it handed a line to, which goes on whatever happens to the hub.
    fn in_terminal(&self) -> bool {
        self.phase == Phase::Setup && self.made.pane.is_some() && !self.made.line.is_empty()
    }

    fn json(&self) -> Value {
        let error = self.error.as_ref().map(|(step, text)| json!({"step": step, "text": text}));
        let made = &self.made;
        json!({
            "id": self.id, "label": self.label, "title": self.title, "state": self.state, "at": self.at, "request": self.request,
            "steps": self.steps.iter().map(|(id, t, s, n)| json!({"id": id, "text": t, "state": s, "note": n})).collect::<Vec<_>>(),
            "error": error, "retry": self.state == "failed",
            "cancel": (self.state == "failed" && (made.worktree.is_some() || made.space.is_some())) || self.state == "running",
            "setup": self.phase == Phase::Setup,
            "made": {"main": made.main, "worktree": made.worktree, "branch": made.branch, "space": made.space, "pane": made.pane, "handed": made.handed, "line": made.line, "marker": made.marker},
        })
    }

    /// A run as `start-runs.json` keeps it. A run the hub itself ran (its steps, a driven start's setup or session) was
    /// cut short by the restart and reads as failed; a terminal's setup runs on, and is followed again.
    fn from_json(v: &Value, cache: &Path) -> Option<Run> {
        let state = STATES.iter().copied().find(|s| v["state"] == *s)?;
        let m = &v["made"];
        let made = Made {
            main: PathBuf::from(text_of(&m["main"])),
            worktree: m["worktree"].as_str().map(str::to_string),
            branch: m["branch"].as_array().filter(|a| a.len() == 2).map(|a| (text_of(&a[0]), text_of(&a[1]))),
            space: m["space"].as_str().map(str::to_string),
            pane: m["pane"].as_str().map(str::to_string),
            handed: m["handed"] == true,
            line: text_of(&m["line"]),
            marker: PathBuf::from(text_of(&m["marker"])),
        };
        let steps = list(&v["steps"])
            .iter()
            .map(|s| {
                (
                    text_of(&s["id"]),
                    text_of(&s["text"]),
                    ["todo", "running", "done", "failed", "skipped"].into_iter().find(|x| s["state"] == *x).unwrap_or("todo"),
                    text_of(&s["note"]),
                )
            })
            .collect();
        let mut run = Run {
            id: text_of(&v["id"]),
            label: text_of(&v["label"]),
            title: text_of(&v["title"]),
            state,
            phase: if state == "running" && v["setup"] == true {
                Phase::Setup
            } else if state == "running" {
                Phase::Steps
            } else {
                Phase::Over
            },
            steps,
            error: v["error"].as_object().map(|e| (text_of(&e["step"]), text_of(&e["text"]))),
            request: v["request"].clone(),
            made,
            cancel: false,
            at: v["at"].as_i64().unwrap_or(0),
            cache: cache.to_path_buf(),
            issue: None,
            take: None,
        };
        if run.state == "running" && !run.in_terminal() {
            let step = run.steps.iter().find(|s| s.2 == "running").map_or_else(|| "start".to_string(), |s| s.0.clone());
            for s in run.steps.iter_mut() {
                s.2 = match s.2 {
                    "running" => "failed",
                    "todo" => "skipped",
                    other => other,
                };
            }
            run.state = "failed";
            run.phase = Phase::Over;
            run.error = Some((step, "the hub restarted during it: retry it, or cancel to remove what it made".into()));
        }
        Some(run)
    }
}

fn runs() -> MutexGuard<'static, Vec<Run>> {
    static RUNS: Mutex<Vec<Run>> = Mutex::new(Vec::new());
    RUNS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The runs a cache folder kept, read back after a restart; a setup that was running is followed again.
pub(crate) fn load_runs(cache: &Path) {
    let kept = files::load(&cache.join("start-runs.json")).unwrap_or(Value::Null);
    let loaded: Vec<Run> = list(&kept["runs"]).iter().filter_map(|v| Run::from_json(v, cache)).collect();
    let watch: Vec<(String, PathBuf)> = loaded.iter().filter(|r| r.in_terminal()).map(|r| (r.id.clone(), r.made.marker.clone())).collect();
    {
        let mut all = runs();
        for run in loaded {
            if !all.iter().any(|r| r.id == run.id) {
                all.push(run);
            }
        }
    }
    for (id, marker) in watch {
        watch_setup(id, marker);
    }
}

/// Writes the runs of this cache folder, for a hub that starts again and for the inbox's rows.
fn save(cache: &Path) {
    static WRITING: Mutex<()> = Mutex::new(());
    let _held = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let kept: Vec<Value> = runs().iter().filter(|r| r.cache == cache).map(Run::json).collect();
    let _ = files::write(&cache.join("start-runs.json"), &json!({"runs": kept}));
}

/// Changes the run, keeps it, then shows it to the `start` view's subscribers and the inbox.
fn update(id: &str, f: impl FnOnce(&mut Run)) {
    let cache = {
        let mut all = runs();
        let Some(run) = all.iter_mut().find(|r| r.id == id) else { return };
        f(run);
        run.cache.clone()
    };
    save(&cache);
    crate::views::touch("start");
    crate::views::touch("inbox");
}

fn set_step(id: &str, step: &str, state: &'static str, note: &str) {
    update(id, |run| {
        if let Some(s) = run.steps.iter_mut().find(|s| s.0 == step) {
            s.2 = state;
            s.3 = note.to_string();
        }
    });
}

fn cancelled(id: &str) -> bool {
    runs().iter().find(|r| r.id == id).is_some_and(|r| r.cancel || r.state == "cancelled")
}

/// Whether the hub itself runs a start now (its steps, or a driven start's setup and session), which a hand-over or an
/// idle exit would cut short; a terminal's setup goes on without it.
pub fn running() -> bool {
    runs().iter().any(|r| r.phase != Phase::Over && !r.in_terminal())
}

/// The process group of each driven start's setup while it runs, by run id, so Cancel can end it.
fn setups() -> MutexGuard<'static, std::collections::HashMap<String, u32>> {
    static SETUPS: std::sync::OnceLock<Mutex<std::collections::HashMap<String, u32>>> = std::sync::OnceLock::new();
    SETUPS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

fn made(id: &str) -> Made {
    runs().iter().find(|r| r.id == id).map(|r| r.made.clone()).unwrap_or_default()
}

/// The `start` view: every run, newest first, or the one `args.id` names.
pub fn view(args: &Value, _: crate::views::Doc) -> Value {
    loaded();
    let all = runs();
    match args["id"].as_str().filter(|id| !id.is_empty()) {
        Some(id) => json!({"run": all.iter().find(|r| r.id == id).map(Run::json)}),
        None => json!({"runs": all.iter().rev().map(Run::json).collect::<Vec<_>>()}),
    }
}

/// Removes what the run made: the space it opened, the worktree it added and its new branch while it holds no commit of
/// its own. Once its space was handed over, the worktree goes only when nothing in it changed.
fn undo(desk: &dyn Desk, made: &Made) -> Vec<String> {
    let mut done = Vec::new();
    if let Some(space) = made.space.as_ref().filter(|s| !s.is_empty())
        && desk.close(space)
    {
        done.push("closed its space".to_string());
    }
    if let Some(path) = &made.worktree {
        let args: &[&str] = if made.handed { &["worktree", "remove", path] } else { &["worktree", "remove", "--force", path] };
        let removed = git(&made.main, args);
        if removed.ok() {
            done.push(format!("removed {path}"));
        } else if made.handed && Path::new(path).exists() {
            done.push(format!("kept {path}: it has changes"));
        } else {
            // A worktree whose `add` never finished: its folder, then git's record of it.
            let _ = std::fs::remove_dir_all(path);
            let _ = git(&made.main, &["worktree", "prune"]);
            if !Path::new(path).exists() {
                done.push(format!("removed {path}"));
            }
        }
    }
    let kept_tree = made.worktree.as_ref().is_some_and(|p| Path::new(p).exists());
    if let Some((branch, at)) = &made.branch {
        let now = tip(&made.main, &format!("refs/heads/{branch}"));
        if now.is_empty() {
            // Gone already, or never made.
        } else if kept_tree {
            done.push(format!("kept the branch {branch}, checked out there"));
        } else if (now == *at || at.is_empty()) && git(&made.main, &["branch", "-D", branch]).ok() {
            done.push(format!("deleted the branch {branch}"));
        } else {
            done.push(format!("kept the branch {branch}: it has commits of its own"));
        }
    }
    done
}

/// The step failed: what the run made is removed, the steps after it are skipped, and the run says which step and why.
fn fail(world: &World, id: &str, step: &str, why: String) -> Res<String> {
    let undone = undo(world.desk, &made(id));
    let note = if undone.is_empty() { String::new() } else { format!("{}.", undone.join(", ")) };
    update(id, |run| {
        for s in run.steps.iter_mut() {
            if s.0 == step {
                s.2 = "failed";
                s.3 = why.clone();
            } else if s.2 == "todo" || s.2 == "running" {
                s.2 = "skipped";
            }
        }
        run.state = "failed";
        run.phase = Phase::Over;
        run.error = Some((step.to_string(), why.clone()));
        run.made = Made { main: run.made.main.clone(), ..Made::default() };
    });
    Err(format!("{step}: {why}{}", if note.is_empty() { String::new() } else { format!(" ({note})") }))
}

/// A run stopped by Cancel: what it made is removed, as far as it is safe to.
fn stop(world: &World, id: &str) -> Res<String> {
    let undone = undo(world.desk, &made(id));
    update(id, |run| {
        run.state = "cancelled";
        run.phase = Phase::Over;
        for s in run.steps.iter_mut().filter(|s| s.2 == "todo" || s.2 == "running") {
            s.2 = "skipped";
        }
    });
    Ok(if undone.is_empty() { "cancelled".into() } else { format!("cancelled: {}", undone.join(", ")) })
}

/// The script a start's terminal runs, written beside its runs: the setup command first when there is one, under
/// `cockpit setup-run` so the hub takes nothing it asks for the person's, its exit status kept where the hub reads it,
/// then the agent with its prompt read from a file, so nothing from a title or a prompt is ever typed into a shell.
fn script_for(plan: &Plan, marker: &Path, prompt_file: &Path) -> String {
    let agent = plan.agent.clone().unwrap_or_else(agent::default);
    let picked = agent::Picked { model: Some(plan.model.as_str()), effort: Some(plan.effort.as_str()), fast: plan.fast };
    let prompt = (!plan.prompt.is_empty()).then_some(PROMPT_MARK);
    let mut argv = agent.start_picked(prompt, &picked);
    if let Some(mode) = Some(plan.mode.as_str()).filter(|m| !m.is_empty() && *m != "default").and_then(|m| agent.mode_argv(m)) {
        argv.extend(mode);
    }
    let argv = agent.typed(&argv);
    let words: Vec<String> = argv
        .iter()
        .map(|w| if w.contains(PROMPT_MARK) { format!("'{}'", w.replace('\'', "'\"'\"'")).replace(PROMPT_MARK, "'\"$(cat \"$P\")\"'") } else { quote(w) })
        .collect();
    let mut lines = vec!["# Start work: the setup, then the agent.".to_string(), format!("P={}", quote(&prompt_file.to_string_lossy()))];
    // The account it runs on: its config folder, in the variable its table row names.
    if let (Some(var), false) = (agent.account_env.as_deref(), plan.account.is_empty()) {
        lines.push(format!("export {var}={}", quote(&plan.account)));
    }
    if let Some((command, _)) = &plan.setup {
        lines.push(format!("{} setup-run /bin/sh -c {}", quote(&cockpit_bin()), quote(command)));
        lines.push("s=$?".into());
        lines.push(format!("printf %s \"$s\" > {}", quote(&marker.to_string_lossy())));
        lines.push("[ \"$s\" = 0 ] || exit \"$s\"".into());
    }
    lines.push(format!("exec {}", words.join(" ")));
    lines.join("\n") + "\n"
}

/// The `cockpit` binary the hub runs as, which runs a start's setup as its own work.
fn cockpit_bin() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "cockpit".into())
}

/// The record of a new run, its steps all to do; refused while a run of that id is live. Finished runs past the last
/// thirty go first.
fn begin(world: &World, id: &str, plan: &Plan, request: &Value) -> Res {
    {
        let mut all = runs();
        if all.iter().any(|r| r.id == id && r.phase != Phase::Over) {
            return Err("this start runs already".into());
        }
        all.retain(|r| r.id != id);
        while all.iter().filter(|r| r.phase == Phase::Over).count() > 30 {
            let Some(at) = all.iter().position(|r| r.phase == Phase::Over) else { break };
            all.remove(at);
        }
        let steps = plan.steps.iter().map(|(sid, t)| (sid.to_string(), t.clone(), "todo", String::new())).collect();
        all.push(Run {
            id: id.to_string(),
            label: plan.label.clone(),
            title: plan.title.clone(),
            state: "running",
            phase: Phase::Steps,
            steps,
            error: None,
            request: request.clone(),
            made: Made { main: plan.main.clone(), ..Made::default() },
            cancel: false,
            at: now_ms(),
            cache: world.cache.clone(),
            issue: plan.issue.as_ref().and_then(Value::as_object).cloned(),
            take: Some(world.take),
        });
    }
    save(&world.cache);
    crate::views::touch("start");
    crate::views::touch("inbox");
    Ok(())
}

/// A run refused before its first step, recorded as failed so every surface can say why and offer Retry.
fn refused(world: &World, id: &str, plan: &Plan, request: &Value, why: String) -> Res<String> {
    if begin(world, id, plan, request).is_ok() {
        update(id, |run| {
            run.state = "failed";
            run.phase = Phase::Over;
            for s in run.steps.iter_mut() {
                s.2 = "skipped";
            }
            run.error = Some(("plan".into(), why.clone()));
        });
    }
    Err(why)
}

/// Runs the request: the plan's checks, then each step in turn; a note on success, the failing step's error otherwise.
pub(crate) fn run_in(world: &World, request: &Value) -> Res<String> {
    let req = Request::from(request);
    let id = if valid_id(&req.id) { req.id.clone() } else { format!("s{}", now_ms()) };
    let plan = plan_in(world, &req);
    if !plan.ok() {
        let why: Vec<String> = plan.blockers.iter().map(|b| b.text.clone()).collect();
        return refused(world, &id, &plan, request, format!("can't start: {}", why.join("; ")));
    }
    begin(world, &id, &plan, request)?;
    let finish = |note: String| {
        update(&id, |run| {
            run.state = "done";
            run.phase = Phase::Over;
            for s in run.steps.iter_mut().filter(|s| s.2 != "failed") {
                s.2 = "done";
            }
        });
        Ok(note)
    };
    if plan.place == "focus" || plan.place == "space" {
        // The session's key is what the chat that asked follows.
        if plan.session {
            update(&id, |run| run.made.pane = Some(plan.space.clone()));
        }
        world.desk.focus_id(&plan.space);
        if plan.session {
            return finish(format!("\u{201c}{}\u{201d} is open already: brought it up", plan.label));
        }
        return finish(format!("\u{201c}{}\u{201d} already runs: brought it into view", plan.label));
    }
    let main = plan.main.clone();
    let remote = plan.remote.clone();
    if plan.has("fetch") {
        set_step(&id, "fetch", "running", "");
        let fetched = match (&plan.cross, plan.new_branch) {
            (Some(n), _) => git_long(&main, &["fetch", "-q", &remote, &format!("+pull/{n}/head:refs/remotes/{remote}/pr/{n}")]),
            (None, true) => git_long(&main, &["fetch", "-q", &remote, &plan.base]),
            (None, false) => git_long(&main, &["fetch", "-q", &remote, &plan.branch]),
        };
        if !fetched.ok() {
            return fail(world, &id, "fetch", format!("git fetch: {}", first_line(&fetched.stderr)));
        }
        if let Some(n) = &plan.cross {
            let head = tip(&main, &format!("refs/remotes/{remote}/pr/{n}"));
            let local = tip(&main, &format!("refs/heads/{}", plan.branch));
            if !local.is_empty() && local != head {
                return fail(world, &id, "fetch", format!("a branch {} is here already and is not this PR's head", plan.branch));
            }
        }
        set_step(&id, "fetch", "done", "");
        if cancelled(&id) {
            return stop(world, &id);
        }
    }
    if plan.has("worktree") {
        set_step(&id, "worktree", "running", "");
        if let Some(dir) = Path::new(&plan.path).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let start = match &plan.cross {
            Some(n) => format!("refs/remotes/{remote}/pr/{n}"),
            None => format!("{remote}/{}", plan.base),
        };
        let tracked = format!("{remote}/{}", plan.branch);
        let make_branch = plan.new_branch || (plan.remote_only && plan.cross.is_none());
        let from = if plan.new_branch { start.clone() } else { tracked.clone() };
        // What the add will make is recorded first: a timeout or a hook's failure leaves it half-made.
        let at = if make_branch { tip(&main, &from) } else { String::new() };
        update(&id, |run| {
            run.made.worktree = Some(plan.path.clone());
            if make_branch {
                run.made.branch = Some((plan.branch.clone(), at.clone()));
            }
        });
        let args: Vec<&str> = if plan.new_branch {
            vec!["worktree", "add", "--no-track", "-b", &plan.branch, &plan.path, &start]
        } else if make_branch {
            vec!["worktree", "add", "--track", "-b", &plan.branch, &plan.path, &tracked]
        } else {
            vec!["worktree", "add", &plan.path, &plan.branch]
        };
        let added = git_long(&main, &args);
        if !added.ok() {
            return fail(world, &id, "worktree", format!("git worktree add: {}", first_line(&added.stderr)));
        }
        set_step(&id, "worktree", "done", "");
        if cancelled(&id) {
            return stop(world, &id);
        }
    }
    if let Some((_, h)) = &plan.setup
        && req.trust == *h
    {
        approve(&world.cache, &main, h);
    }
    if plan.driven {
        return drive(world, &id, &plan);
    }
    set_step(&id, "space", "running", "");
    let (space, pane) = match world.desk.open(&plan.path, &plan.label) {
        Ok(opened) => opened,
        Err(e) => return fail(world, &id, "space", e),
    };
    update(&id, |run| {
        run.made.space = Some(space.clone()).filter(|s| !s.is_empty());
        run.made.pane = Some(pane.clone());
    });
    set_step(&id, "space", "done", "");
    if cancelled(&id) {
        return stop(world, &id);
    }
    let folder = world.cache.join("start");
    let _ = std::fs::create_dir_all(&folder);
    // Names of this start's own that no other process could have made first: each file is new, written whole.
    let name = format!("{id}-{}", inbox::suffix());
    let (marker, prompt_file, script) = (folder.join(format!("{name}.setup")), folder.join(format!("{name}.prompt")), folder.join(format!("{name}.sh")));
    let written = inbox::write_private(&prompt_file, &plan.prompt).and_then(|()| inbox::write_private(&script, &script_for(&plan, &marker, &prompt_file)));
    if let Err(e) = written {
        return fail(world, &id, "agent", format!("the start's script was not written: {e}"));
    }
    let line = join(&["sh".to_string(), script.to_string_lossy().into_owned()]);
    update(&id, |run| {
        run.made.line = line.clone();
        run.made.marker = marker.clone();
        run.made.handed = true;
    });
    let step = if plan.setup.is_some() { "setup" } else { "agent" };
    set_step(&id, step, "running", "");
    if !world.desk.type_line(&pane, &line) {
        return fail(world, &id, step, "the terminal took no line".into());
    }
    remember(&world.cache, &plan);
    let name = plan.agent.as_ref().map(|a| a.name.clone()).unwrap_or_default();
    let queued = queue_first_prompt(&plan, &pane);
    if plan.setup.is_some() {
        update(&id, |run| run.phase = Phase::Setup);
        watch_setup(id.clone(), marker);
        return Ok(format!("\u{201c}{}\u{201d}: {} starts once its setup succeeds{queued}", plan.label, name));
    }
    let moved = ticket_step(&id);
    finish(format!("started {name} in \u{201c}{}\u{201d}{moved}{queued}", plan.label))
}

/// An agent that takes no prompt as it starts gets it through its queue (`queue tell`), sent once it is up and rests:
/// ` · what became of it`, or nothing for an agent that took it on its command line.
fn queue_first_prompt(plan: &Plan, handle: &str) -> String {
    if plan.prompt.is_empty() || plan.agent.as_ref().is_none_or(Agent::start_takes_prompt) {
        return String::new();
    }
    match crate::views::act("queue", "tell", &[json!(handle), json!(plan.prompt)]) {
        Some(told) if told["error"].is_null() => format!(" · the prompt {}", told["note"].as_str().unwrap_or("is queued")),
        Some(told) => format!(" · the prompt was not queued: {}", told["error"].as_str().unwrap_or("the queue refused it")),
        None => " · the prompt was not queued: paste it once the agent is up".into(),
    }
}

fn first_line(text: &str) -> String {
    text.trim().lines().find(|l| !l.trim().is_empty()).unwrap_or("failed").to_string()
}

/// The ticket moved and assigned when the tracker has a token: ` · what changed`, or nothing.
fn ticket_step(id: &str) -> String {
    let (issue, take, has) = {
        let all = runs();
        let Some(run) = all.iter().find(|r| r.id == id) else { return String::new() };
        (run.issue.clone(), run.take, run.steps.iter().any(|s| s.0 == "ticket"))
    };
    if !has {
        return String::new();
    }
    let (Some(issue), Some(take)) = (issue, take) else {
        set_step(id, "ticket", "done", "the agent does it");
        return String::new();
    };
    let key = issue.get("identifier").and_then(Value::as_str).unwrap_or_default().to_string();
    match take(&key, &issue) {
        Ok(moved) => {
            set_step(id, "ticket", "done", &moved);
            if moved.is_empty() { String::new() } else { format!(" · {moved}") }
        }
        Err(e) => {
            set_step(id, "ticket", "failed", &e);
            format!(" · {key} was not moved: {e}")
        }
    }
}

/// Follows the setup command's exit status in the background: the run is done once it succeeds, failed when it does not
/// or says nothing within the hour.
fn watch_setup(id: String, marker: PathBuf) {
    std::thread::spawn(move || {
        let deadline = Instant::now() + SETUP_LIMIT;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(400));
            let phase = runs().iter().find(|r| r.id == id).map(|r| r.phase.clone());
            if phase != Some(Phase::Setup) {
                return;
            }
            let Ok(code) = std::fs::read_to_string(&marker) else { continue };
            let code = code.trim().to_string();
            if code.is_empty() {
                continue;
            }
            let _ = std::fs::remove_file(&marker);
            setup_ended(&id, &code);
            return;
        }
        if runs().iter().any(|r| r.id == id && r.phase == Phase::Setup) {
            setup_ended(&id, "timeout");
        }
    });
}

fn setup_ended(id: &str, code: &str) {
    let ok = code == "0";
    update(id, |run| {
        for s in run.steps.iter_mut() {
            if s.0 == "setup" {
                s.2 = if ok { "done" } else { "failed" };
                s.3 = match code {
                    "0" => String::new(),
                    "timeout" => "no word from it within the hour".into(),
                    _ => format!("exited with {code}"),
                };
            } else if s.0 == "agent" {
                s.2 = if ok { "done" } else { "skipped" };
            }
        }
        if !ok {
            run.state = "failed";
            run.phase = Phase::Over;
            run.error = Some((
                "setup".into(),
                if code == "timeout" {
                    "the setup command said nothing within the hour: look at its space".into()
                } else {
                    format!("the setup command exited with {code}: its output is in the space")
                },
            ));
        }
    });
    if ok {
        ticket_step(id);
        update(id, |run| {
            run.state = "done";
            run.phase = Phase::Over;
        });
    }
}

/// A session cockpit drives: setup and the session start in the background, since setup may take minutes; setup runs
/// in a login shell like a terminal's, its output kept beside the run.
fn drive(world: &World, id: &str, plan: &Plan) -> Res<String> {
    let (id, plan) = (id.to_string(), plan.clone());
    let cache = world.cache.clone();
    update(&id, |run| run.phase = Phase::Setup);
    let label = plan.label.clone();
    // The session starts on behalf of whoever asked: its mode is checked against where they stood when they asked, not
    // against this thread or a pid that may be gone by then.
    let sender = cockpit_core::peer::carry();
    std::thread::spawn(move || {
        cockpit_core::peer::adopt(sender);
        drive_steps(&id, &plan, &cache)
    });
    Ok(format!("\u{201c}{label}\u{201d}: the session starts once its worktree is ready"))
}

/// The driven start's setup, then its session and prompt; a Cancel is looked at between steps, and a run it cancelled
/// stays cancelled whatever its last step said.
fn drive_steps(id: &str, plan: &Plan, cache: &Path) {
    // The run's end, unless a Cancel came first.
    let over = |state: &'static str, error: Option<(&str, String)>, step: Option<(&str, &'static str)>| {
        update(id, |run| {
            if run.cancel || run.state == "cancelled" {
                return;
            }
            run.state = state;
            run.phase = Phase::Over;
            if let Some((at, why)) = error {
                run.error = Some((at.to_string(), why));
            }
            for s in run.steps.iter_mut() {
                match step {
                    Some((at, set)) if s.0 == at => s.2 = set,
                    _ if state == "done" && s.2 != "failed" => s.2 = "done",
                    _ if state == "failed" && s.2 == "todo" => s.2 = "skipped",
                    _ => {}
                }
            }
        });
    };
    if let Some((command, _)) = &plan.setup {
        if cancelled(id) {
            return;
        }
        set_step(id, "setup", "running", "");
        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into());
        let out = proc::run_grouped(&[shell, "-lc".into(), command.clone()], SETUP_LIMIT, Some(Path::new(&plan.path)), |pgid| {
            setups().insert(id.to_string(), pgid);
        });
        setups().remove(id);
        let log = cache.join("start").join(format!("{id}.log"));
        let _ = std::fs::create_dir_all(cache.join("start"));
        let _ = std::fs::write(&log, format!("{}{}", out.stdout, out.stderr));
        if cancelled(id) {
            return;
        }
        if !out.ok() {
            let why = format!("the setup command exited with {}: its output is in {}", out.code, log.display());
            return over("failed", Some(("setup", why)), Some(("setup", "failed")));
        }
        set_step(id, "setup", "done", "");
    }
    if cancelled(id) {
        return;
    }
    set_step(id, "agent", "running", "");
    let agent = plan.agent.as_ref().map(|a| a.id.clone()).unwrap_or_default();
    let mode = if plan.mode.is_empty() { "default" } else { plan.mode.as_str() };
    let picks = json!({"agent": agent, "cwd": plan.path, "mode": mode, "confirm": plan.confirm, "model": plan.model, "effort": plan.effort, "fast": plan.fast});
    let started = crate::views::act("session", "start", &[picks]).unwrap_or_else(|| json!({"error": "no session view"}));
    if let Some(error) = started["error"].as_str() {
        return over("failed", Some(("agent", error.to_string())), Some(("agent", "failed")));
    }
    let key = started["note"].as_str().unwrap_or_default().trim_start_matches("started ").to_string();
    // Cancelled while it started: the session it started goes too, before it works in a folder that is gone.
    if cancelled(id) {
        crate::views::act("session", "stop", &[json!({"key": key, "confirm": true})]);
        return;
    }
    if !plan.prompt.is_empty() {
        let sent = crate::views::act("session", "send", &[json!({"key": key, "text": plan.prompt})]).unwrap_or_else(|| json!({"error": "no session view"}));
        if let Some(error) = sent["error"].as_str() {
            return over("failed", Some(("agent", format!("the prompt was not sent: {error}"))), Some(("agent", "failed")));
        }
    }
    remember(cache, plan);
    update(id, |run| run.made.pane = Some(key.clone()));
    if cancelled(id) {
        return;
    }
    ticket_step(id);
    over("done", None, None);
}

static FILES: Mutex<()> = Mutex::new(());

/// The setup command approved for this checkout, so it is not asked about again until it changes.
fn approve(cache: &Path, main: &Path, h: &str) {
    let _held = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let path = cache.join("start-trust.json");
    let mut approved = files::load(&path).filter(Value::is_object).unwrap_or_else(|| json!({}));
    approved[main.to_string_lossy().as_ref()] = json!(h);
    let _ = files::write(&path, &approved);
}

/// The repo, agent and mode of the last start, which the next form picks first.
fn remember(cache: &Path, plan: &Plan) {
    if plan.keep_defaults {
        return;
    }
    let _held = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let last = json!({"repo": plan.repo, "agent": plan.agent.as_ref().map(|a| a.id.clone()), "mode": plan.mode});
    let _ = files::write(&cache.join("start.json"), &last);
}

/// Cancel: a run still in its steps stops after the current one; a run whose setup runs or failed has what it made
/// removed now, as far as it is safe to.
pub(crate) fn cancel_in(world: &World, id: &str) -> Res<String> {
    let Some(run) = runs().iter().find(|r| r.id == id).cloned() else { return Err("no such start".into()) };
    match (run.state, &run.phase) {
        ("running", Phase::Steps) => {
            update(id, |run| run.cancel = true);
            Ok("cancelling after the current step".into())
        }
        ("running", Phase::Setup) => {
            update(id, |run| run.cancel = true);
            // A driven start's setup is ended first: the worktree goes only once nothing runs in it.
            let pgid = setups().get(id).copied();
            if let Some(pgid) = pgid {
                proc::end_group(pgid);
                let deadline = Instant::now() + Duration::from_secs(5);
                while setups().contains_key(id) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            stop(world, id)
        }
        ("failed", _) => stop(world, id),
        ("done", _) => Err(format!("\u{201c}{}\u{201d} started already: close its space to stop it", run.label)),
        _ => Ok("cancelled".into()),
    }
}

/// Retry: a failed setup runs again where it ran; any other failure runs the whole request again.
pub(crate) fn retry_in(world: &World, id: &str) -> Res<String> {
    let Some(run) = runs().iter().find(|r| r.id == id).cloned() else { return Err("no such start".into()) };
    if run.phase != Phase::Over {
        return Err("this start runs already".into());
    }
    let setup_failed = run.error.as_ref().is_some_and(|(step, _)| step == "setup");
    if setup_failed && let Some(pane) = run.made.pane.clone().filter(|_| !run.made.line.is_empty()) {
        let _ = std::fs::remove_file(&run.made.marker);
        update(id, |r| {
            r.state = "running";
            r.phase = Phase::Setup;
            r.error = None;
            for s in r.steps.iter_mut().filter(|s| s.0 == "setup" || s.0 == "agent") {
                s.2 = if s.0 == "setup" { "running" } else { "todo" };
                s.3.clear();
            }
        });
        if !world.desk.type_line(&pane, &run.made.line) {
            update(id, |r| {
                r.state = "failed";
                r.phase = Phase::Over;
                r.error = Some(("setup".into(), "the terminal took no line: its space may be closed".into()));
            });
            return Err("setup: the terminal took no line".into());
        }
        watch_setup(id.to_string(), run.made.marker.clone());
        return Ok(format!("running the setup of \u{201c}{}\u{201d} again", run.label));
    }
    if setup_failed && run.made.worktree.is_some() {
        let req = Request::from(&run.request);
        let mut plan = plan_in(world, &req);
        if plan.driven && plan.place == "checkout" {
            // The worktree is there already, so the plan has no setup of its own: the one approved for the repo runs again.
            plan.setup = setup_again(world, &plan, &req)?;
            update(id, |r| {
                r.state = "running";
                r.phase = Phase::Setup;
                r.error = None;
                for s in r.steps.iter_mut().filter(|s| s.2 != "done" || s.0 == "setup") {
                    (s.2, s.3) = ("todo", String::new());
                }
            });
            return drive(world, id, &plan);
        }
    }
    let mut request = run.request.clone();
    request["id"] = json!(id);
    run_in(world, &request)
}

/// The repo's setup command for a driven start's Retry, while it is the one approved for its checkout; refused once it
/// changed, which a new start asks about.
fn setup_again(world: &World, plan: &Plan, req: &Request) -> Res<Option<(String, String)>> {
    let Some(command) = world.cfg.0["start"]["setup"][&plan.repo].as_str().map(str::trim).filter(|c| !c.is_empty()) else { return Ok(None) };
    let h = hash(command);
    let approved = files::load(&world.cache.join("start-trust.json")).unwrap_or(Value::Null);
    if approved[plan.main.to_string_lossy().as_ref()].as_str() == Some(h.as_str()) || req.trust == h {
        return Ok(Some((command.to_string(), h)));
    }
    Err(format!("{}'s setup command changed since this start: start it again to approve it", plan.repo))
}

// ---------------------------------------------------------------------------------------------------------------------
// The actions and the fetch

/// `fetch start_work {request}`: the plan, with the choices a form offers.
pub fn plan_json(args: &Value) -> Value {
    real(|world| {
        let request = args.get("request").unwrap_or(args);
        let mut out = plan_in(world, &Request::from(request)).json();
        out["choices"] = choices(world);
        out
    })
}

/// `start work [request]`: the start run, its steps shown in the `start` view.
pub fn work(ctx: &mut Ctx, args: &[Value]) -> Res {
    let request = arg(args, 0)?.clone();
    let note = real(|world| run_in(world, &request))?;
    ctx.note(note);
    Ok(())
}

/// `start cancel [id]`.
pub fn cancel(ctx: &mut Ctx, args: &[Value]) -> Res {
    let id = text_of(arg(args, 0)?);
    let note = real(|world| cancel_in(world, &id))?;
    ctx.note(note);
    Ok(())
}

/// `start retry [id]`.
pub fn retry(ctx: &mut Ctx, args: &[Value]) -> Res {
    let id = text_of(arg(args, 0)?);
    let note = real(|world| retry_in(world, &id))?;
    ctx.note(note);
    Ok(())
}

/// `start dismiss [id]`: a finished run's record goes once its outcome was seen.
pub fn dismiss(ctx: &mut Ctx, args: &[Value]) -> Res {
    let id = text_of(arg(args, 0)?);
    real(|world| {
        runs().retain(|r| r.id != id || r.phase != Phase::Over);
        save(&world.cache);
    });
    crate::views::touch("start");
    crate::views::touch("inbox");
    ctx.note("dismissed");
    Ok(())
}

/// `inbox start_cancel [id]` (armed `cancel-start-<id>`): the inbox row's Cancel, asked first since it removes what the
/// start made.
pub fn inbox_cancel(ctx: &mut Ctx, args: &[Value]) -> Res {
    let id = text_of(arg(args, 0)?);
    if !ctx.confirmed(&format!("cancel-start-{id}")) {
        return Ok(());
    }
    cancel(ctx, args)
}

#[cfg(test)]
mod tests;
