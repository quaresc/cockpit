//! What keeps a worktree from being removed: changes not committed, commits not pushed, ignored files other than
//! builds, a program still in it, a lock, submodules, or a branch not merged. Every surface names them alike.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// One reason a worktree stays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocker {
    Uncommitted(usize),
    Unpushed(usize),
    /// Ignored files other than builds, which a removal deletes with the folder.
    Ignored(usize),
    /// What runs in it: an agent's name, else the program's.
    Running(String),
    /// What could not be told: what runs in it, its changes, its commits, its lock or its merge.
    Unchecked(String),
    /// `git worktree lock`ed, with its reason.
    Locked(String),
    /// It holds submodules, which git does not remove.
    Submodules,
    /// Not merged into this branch (`main`).
    NotMerged(String),
    /// No `origin/HEAD`, so what merged means is unknown.
    UnknownBase,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Blocker {
    pub fn id(&self) -> &'static str {
        match self {
            Blocker::Uncommitted(_) => "uncommitted",
            Blocker::Unpushed(_) => "unpushed",
            Blocker::Ignored(_) => "ignored",
            Blocker::Running(_) => "running",
            Blocker::Unchecked(_) => "unchecked",
            Blocker::Locked(_) => "locked",
            Blocker::Submodules => "submodules",
            Blocker::NotMerged(_) => "not_merged",
            Blocker::UnknownBase => "unknown_base",
        }
    }

    /// The words every surface shows: "2 uncommitted changes", "Claude Code still runs in it".
    pub fn words(&self) -> String {
        match self {
            Blocker::Uncommitted(n) => plural(*n, "uncommitted change", "uncommitted changes"),
            Blocker::Unpushed(n) => format!("{} not pushed", plural(*n, "commit", "commits")),
            Blocker::Ignored(n) => format!("{} other than builds", plural(*n, "ignored file", "ignored files")),
            Blocker::Running(what) => format!("{what} still runs in it"),
            Blocker::Unchecked(what) => format!("cannot tell {what}"),
            Blocker::Locked(why) if why.is_empty() => "locked".into(),
            Blocker::Locked(why) => format!("locked: {why}"),
            Blocker::Submodules => "it holds submodules".into(),
            Blocker::NotMerged(base) => format!("not merged into {base}"),
            Blocker::UnknownBase => "its default branch is unknown (git remote set-head origin --auto)".into(),
        }
    }

    /// Whether only the branch's merge holds it: a fetch may settle that, nothing else does.
    pub fn is_merge(&self) -> bool {
        matches!(self, Blocker::NotMerged(_) | Blocker::UnknownBase)
    }

    pub fn json(&self) -> Value {
        json!({"id": self.id(), "words": self.words()})
    }
}

/// The blockers as one line: "2 uncommitted changes, not merged into main".
pub fn said(blockers: &[Blocker]) -> String {
    blockers.iter().map(Blocker::words).collect::<Vec<_>>().join(", ")
}

/// Folders that are builds: ignored, made again by a build, never a reason to keep a worktree. The routines' cleanup
/// template lists the same ones.
pub const BUILDS: [&str; 9] = ["node_modules", "target", "_build", "deps", "dist", "build", ".next", ".venv", "__pycache__"];

/// How long a local git command may take; a fetch gets `FETCH_FOR`.
const GIT_FOR: Duration = Duration::from_secs(10);
const FETCH_FOR: Duration = Duration::from_secs(15);

/// What a git command said: its output, and its error, or None when it could not run in time.
pub struct GitOut {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// git in `dir`, never asking for a password or a passphrase, within `limit`; None when it could not start or ran out
/// of time. `ssh` puts ssh in batch mode, for a fetch, unless the person set their own ssh command.
pub fn git_out(dir: &Path, args: &[&str], limit: Duration, ssh: bool) -> Option<GitOut> {
    let mut command = Command::new("git");
    command.args(["-c", "credential.interactive=never", "-C"]).arg(dir).args(args).env("GIT_TERMINAL_PROMPT", "0");
    if ssh && std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() && !has_ssh_command(dir) {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=10");
    }
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut out = String::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_string(&mut out);
            }
            out
        })
    };
    let out = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let (stdout, stderr) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    Some(GitOut { ok: status.success(), stdout: stdout.trim().to_string(), stderr: stderr.trim().to_string() })
}

/// Whether the repository names an ssh command of its own (`core.sshCommand`), which a fetch keeps.
fn has_ssh_command(dir: &Path) -> bool {
    git(dir, &["config", "--get", "core.sshCommand"]).is_some_and(|c| !c.is_empty())
}

/// A local git command in `dir`, bounded: its output, None when it failed or ran out of time.
pub fn git(dir: &Path, args: &[&str]) -> Option<String> {
    git_out(dir, args, GIT_FOR, false).filter(|o| o.ok).map(|o| o.stdout)
}

/// `git fetch --prune origin` in `repo`, never asking anything: the commit each pruned branch had (by branch name), or
/// None when the fetch failed. What a fetch of the last minute said is said again, a failure too.
pub fn fetch(repo: &Path) -> Option<HashMap<String, String>> {
    type Fetched = HashMap<PathBuf, (Instant, Option<HashMap<String, String>>)>;
    static FETCHED: Mutex<Option<Fetched>> = Mutex::new(None);
    if let Some((_, said)) =
        FETCHED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get(repo).filter(|(at, _)| at.elapsed() < Duration::from_secs(60))
    {
        return said.clone();
    }
    // `--porcelain` prints each ref it changed, a pruned one as "- <old> <new> refs/remotes/origin/<branch>"; a git
    // without it fetches plainly and says no prune.
    let said = git_out(repo, &["fetch", "--prune", "--porcelain", "origin"], FETCH_FOR, true)
        .filter(|o| o.ok)
        .map(|o| o.stdout)
        .or_else(|| git_out(repo, &["fetch", "-q", "--prune", "origin"], FETCH_FOR, true).filter(|o| o.ok).map(|_| String::new()))
        .map(|out| {
            out.lines()
                .filter_map(|l| {
                    let mut words = l.split_whitespace();
                    let (flag, old, _, name) = (words.next()?, words.next()?, words.next()?, words.next()?);
                    let branch = name.strip_prefix("refs/remotes/origin/").filter(|_| flag == "-")?;
                    Some((branch.to_string(), old.to_string()))
                })
                .collect::<HashMap<_, _>>()
        });
    FETCHED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(repo.to_path_buf(), (Instant::now(), said.clone()));
    said
}

/// The repository's default branch as a ref (`refs/remotes/origin/main`), from `origin/HEAD`, read again at most every
/// 30 s and after a change here.
pub fn default_branch(repo: &Path) -> Option<String> {
    type Known = HashMap<PathBuf, (Instant, u64, Option<String>)>;
    static KNOWN: Mutex<Option<Known>> = Mutex::new(None);
    let now = generation();
    let mut known = KNOWN.lock().unwrap_or_else(|e| e.into_inner());
    let map = known.get_or_insert_with(HashMap::new);
    if let Some((_, _, base)) = map.get(repo).filter(|(at, made, _)| at.elapsed() < Duration::from_secs(30) && *made == now) {
        return base.clone();
    }
    let base = git(repo, &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"]).filter(|r| !r.is_empty());
    map.insert(repo.to_path_buf(), (Instant::now(), now, base.clone()));
    base
}

/// Whether the branch has commits of its own: its tip moved since it was made (its reflog), else, with no reflog,
/// commits `base` does not have; None when git could not say.
pub fn own_commits(repo: &Path, branch: &str, base: Option<&str>) -> Option<bool> {
    if branch.is_empty() {
        return Some(false);
    }
    let reference = format!("refs/heads/{branch}");
    let log = git(repo, &["reflog", "show", "--format=%H", &reference, "--"])?;
    let (tip, born) = (log.lines().next().unwrap_or_default(), log.lines().last().unwrap_or_default());
    if !tip.is_empty() {
        return Some(tip != born);
    }
    match base {
        Some(base) => git(repo, &["rev-list", "--count", &format!("{base}..{reference}")]).map(|n| n != "0"),
        None => Some(false),
    }
}

/// `own_commits`, a failure read as none.
pub fn has_own_commits(repo: &Path, branch: &str, base: Option<&str>) -> bool {
    own_commits(repo, branch, base).unwrap_or(false)
}

/// Whether commits of the branch's own reached `base`. A gone upstream alone says nothing: a closed pull request's
/// branch goes too.
pub fn merged(repo: &Path, branch: &str, base: &str) -> bool {
    !branch.is_empty()
        && has_own_commits(repo, branch, Some(base))
        && git(repo, &["merge-base", "--is-ancestor", &format!("refs/heads/{branch}"), base]).is_some()
}

/// A program and the folder it works in, as `lsof` or /proc report them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub cwd: PathBuf,
}

/// The programs on this machine and their working folders, from Linux's /proc, else `lsof` (10 s at most); None when
/// neither can say. lsof failing with nothing said found nothing; failing with words, or out of time, cannot tell.
pub fn processes() -> Option<Vec<Process>> {
    // On Linux lsof warns about every process of another user it cannot read, which would read as not knowing.
    if cfg!(target_os = "linux")
        && let Some(found) = from_proc()
    {
        return Some(found);
    }
    if let Ok(mut child) = Command::new("lsof").args(["-a", "-d", "cwd", "-Fcn"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        let (out, err) = (child.stdout.take(), child.stderr.take());
        let read = |pipe: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut text = String::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_string(&mut text);
                }
                text
            })
        };
        let (out, err) = (read(out.map(|p| Box::new(p) as Box<dyn Read + Send>)), read(err.map(|p| Box::new(p) as Box<dyn Read + Send>)));
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() < Duration::from_secs(10) => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        };
        let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
        return (status.success() || err.trim().is_empty()).then(|| parse_lsof(&out));
    }
    from_proc()
}

/// The processes /proc lists with a working folder this account can read; None without /proc.
fn from_proc() -> Option<Vec<Process>> {
    let procs = std::fs::read_dir("/proc").ok()?;
    let found: Vec<Process> = procs
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()))
        .filter_map(|e| {
            let pid = e.file_name().to_string_lossy().parse().ok()?;
            let cwd = std::fs::read_link(e.path().join("cwd")).ok()?;
            let name = std::fs::read_to_string(e.path().join("comm")).ok()?.trim().to_string();
            Some(Process { pid, name, cwd })
        })
        .collect();
    (!found.is_empty()).then_some(found)
}

/// `lsof -Fcn` output (`p<pid>`, `c<command>`, `n<path>` lines) as programs and folders.
pub fn parse_lsof(text: &str) -> Vec<Process> {
    let mut out = Vec::new();
    let (mut pid, mut name) = (0, String::new());
    for line in text.lines() {
        let (tag, rest) = line.split_at(line.chars().next().map_or(0, char::len_utf8));
        match tag {
            "p" => (pid, name) = (rest.parse().unwrap_or(0), String::new()),
            "c" => name = rest.to_string(),
            "n" => out.push(Process { pid, name: name.clone(), cwd: PathBuf::from(rest) }),
            _ => {}
        }
    }
    out
}

/// These processes and every one they started, from `ps`; None when `ps` cannot say.
pub fn tree_of(roots: &[u32]) -> Option<std::collections::HashSet<u32>> {
    let out = Command::new("ps").args(["-A", "-o", "pid=,ppid="]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let pairs: Vec<(u32, u32)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            Some((words.next()?.parse().ok()?, words.next()?.parse().ok()?))
        })
        .collect();
    let mut found: std::collections::HashSet<u32> = roots.iter().copied().collect();
    loop {
        let before = found.len();
        found.extend(pairs.iter().filter(|(_, parent)| found.contains(parent)).map(|(pid, _)| *pid).collect::<Vec<_>>());
        if found.len() == before {
            return Some(found);
        }
    }
}

/// Shells only sit in a terminal a cleanup closes with the space; lsof, git, cockpit and herdr's own only look.
pub const IDLE: [&str; 12] = ["sh", "bash", "zsh", "fish", "dash", "lsof", "cockpit", "git", "herdr", "herdr-core", "herdr-app", "hc"];

/// An agent the hub knows works somewhere: its name, its folder, and the pane and space it runs in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Working {
    pub name: String,
    pub cwd: PathBuf,
    pub pane: String,
    pub space: String,
}

/// The agents the hub knows of (its `agents` document), the folders they work in and their spaces.
pub fn agents() -> Vec<Working> {
    let doc = crate::files::load(&crate::files::cache().join("agents.json")).unwrap_or(Value::Null);
    let now = crate::seen::now_ms();
    doc.as_object()
        .into_iter()
        .flatten()
        // A row a session stopped refreshing a quarter of an hour ago is a session gone.
        .filter(|(_, a)| a["at"].as_u64().is_some_and(|at| now.saturating_sub(at) < 15 * 60_000))
        .filter_map(|(pane, a)| {
            let cwd = a["cwd"].as_str().filter(|c| !c.is_empty())?;
            let name = a["agent"].as_str().and_then(crate::agent::get).map_or_else(|| "Claude Code".to_string(), |agent| agent.name);
            Some(Working { name, cwd: PathBuf::from(cwd), pane: pane.clone(), space: a["space"].as_str().unwrap_or_default().to_string() })
        })
        .collect()
}

/// What runs in a worktree, as far as can be told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inside {
    Idle,
    /// An agent's name, else the program's.
    Running(String),
    /// Neither the hub's agents nor the machine's programs could say.
    Unknown,
}

/// What runs in the worktree: an agent the hub knows works there, else the first agent program, else the first other
/// program.
pub fn running(path: &Path, processes: Option<&[Process]>, agents: &[Working]) -> Inside {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let within = |dir: &Path| dir.starts_with(&real) || dir.starts_with(path);
    if let Some(agent) = agents.iter().find(|a| within(&a.cwd)) {
        return Inside::Running(agent.name.clone());
    }
    let Some(processes) = processes else { return Inside::Unknown };
    let inside: Vec<&Process> = processes.iter().filter(|p| within(&p.cwd)).collect();
    let agent = inside.iter().find_map(|p| crate::agent::of_program(&p.name).map(|a| a.name));
    match agent.or_else(|| inside.iter().map(|p| p.name.clone()).find(|n| !IDLE.contains(&n.trim_start_matches('-')))) {
        Some(what) => Inside::Running(what),
        None => Inside::Idle,
    }
}

/// The file whose time says a worktree changed, for every process that keeps a scan of them.
fn stamp() -> PathBuf {
    crate::files::cache().join("worktrees.stamp")
}

static CHANGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Says a worktree or its repository changed (a removal, a fetch): every scan of them, in any process, reads again.
pub fn changed() {
    CHANGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let _ = std::fs::write(stamp(), crate::seen::now_ms().to_string());
}

/// What a scan is up to date with: the changes said here and the stamp's, from any process.
pub fn generation() -> u64 {
    let here = CHANGED.load(std::sync::atomic::Ordering::Relaxed);
    let there = std::fs::read_to_string(stamp()).ok().and_then(|t| t.trim().parse::<u64>().ok()).unwrap_or(0);
    here.wrapping_add(there)
}

/// What the cleanup knows besides the worktree itself.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// What runs on this machine; None when it cannot be told.
    pub processes: Option<Vec<Process>>,
    /// The agents the hub knows of and their folders.
    pub agents: Vec<Working>,
    /// The head commit the code host merged the branch at, when it says it merged.
    pub merged_at: Option<String>,
    /// The commit the branch had on the remote before a fetch just pruned it.
    pub pruned: Option<String>,
    /// main or master stand in for an unknown `origin/HEAD` (the routines' run worktrees).
    pub guess_base: bool,
}

impl Facts {
    /// What runs on this machine and the agents the hub knows, read now.
    pub fn now() -> Facts {
        Facts { processes: processes(), agents: agents(), ..Facts::default() }
    }
}

/// A worktree as the cleanup sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub branch: String,
    /// Its main checkout.
    pub repo: PathBuf,
    /// Changes `git status` lists, untracked files included.
    pub changes: usize,
    /// Merged, by the code host's word for this very commit, or by commits of its own that reached the default branch.
    pub merged: bool,
    /// Why it stays, in the order every surface lists them; empty when it can go.
    pub blockers: Vec<Blocker>,
}

/// Where a worktree belongs: its branch (empty when detached) and its main checkout; None when git cannot read it.
pub fn place(path: &Path) -> Option<(String, PathBuf)> {
    let head = git(path, &["rev-parse", "--path-format=absolute", "--git-common-dir", "--symbolic-full-name", "HEAD"])?;
    let mut lines = head.lines();
    let common = lines.next().unwrap_or_default();
    let branch = lines.next().unwrap_or_default().strip_prefix("refs/heads/").unwrap_or_default().to_string();
    Some((branch, Path::new(common).parent().map_or_else(|| path.to_path_buf(), Path::to_path_buf)))
}

/// The worktree at `path`, looked at. A probe that fails or runs out of time keeps it, said as what cannot be told.
pub fn look(path: &Path, facts: &Facts) -> Look {
    let Some((branch, repo)) = place(path) else {
        let blockers = vec![Blocker::Unchecked("what it is: git cannot read it".into())];
        return Look { repo: path.to_path_buf(), blockers, ..Look::default() };
    };
    let reference = if branch.is_empty() { String::new() } else { format!("refs/heads/{branch}") };
    let mut out = Vec::new();

    // One status for the changes and the ignored files, untracked and submodules shown whatever the config says;
    // `matching` lists an ignored folder as one line without walking it.
    let status = git(path, &["--no-optional-locks", "status", "--porcelain", "--ignored=matching", "--untracked-files=normal", "--ignore-submodules=none"]);
    let (changes, ignored) = match &status {
        Some(status) => (
            status.lines().filter(|l| !l.is_empty() && !l.starts_with("!! ")).count(),
            status
                .lines()
                .filter_map(|l| l.strip_prefix("!! "))
                .filter(|f| !BUILDS.iter().any(|b| f.trim_end_matches('/').rsplit('/').next() == Some(*b)))
                .count(),
        ),
        None => (0, 0),
    };
    if status.is_none() {
        out.push(Blocker::Unchecked("whether it has uncommitted or ignored files".into()));
    }
    if changes > 0 {
        out.push(Blocker::Uncommitted(changes));
    }

    let tip = git(path, &["rev-parse", "HEAD"]).unwrap_or_default();
    // The code host's merge counts for this tip only: the merged commit itself, or one before it.
    let host = facts
        .merged_at
        .as_deref()
        .filter(|oid| !oid.is_empty() && !tip.is_empty())
        .filter(|oid| git(path, &["cat-file", "-e", &format!("{oid}^{{commit}}")]).is_some())
        .map(|oid| (oid.to_string(), *oid == tip || git(path, &["merge-base", "--is-ancestor", &tip, oid]).is_some()));
    match unpushed(path, &reference, host.as_ref().map(|(oid, _)| oid.as_str()).or(facts.pruned.as_deref())) {
        Some(0) => {}
        Some(ahead) => out.push(Blocker::Unpushed(ahead)),
        None => out.push(Blocker::Unchecked("whether its commits are pushed".into())),
    }
    if ignored > 0 {
        out.push(Blocker::Ignored(ignored));
    }
    match running(path, facts.processes.as_deref(), &facts.agents) {
        Inside::Running(what) => out.push(Blocker::Running(what)),
        Inside::Idle => {}
        Inside::Unknown => out.push(Blocker::Unchecked("what runs in it".into())),
    }
    match locked(&repo, path) {
        Some(None) => {}
        Some(Some(why)) => out.push(Blocker::Locked(why)),
        None => out.push(Blocker::Unchecked("whether it is locked".into())),
    }
    if path.join(".gitmodules").is_file() {
        out.push(Blocker::Submodules);
    }

    let base = default_branch(&repo).or_else(|| {
        facts
            .guess_base
            .then(|| ["main", "master"].into_iter().map(|b| format!("refs/heads/{b}")).find(|b| git(&repo, &["rev-parse", "--verify", "-q", b]).is_some()))?
    });
    let host_merged = host.as_ref().is_some_and(|(_, within)| *within);
    let merged = host_merged || base.as_deref().is_some_and(|b| merged(&repo, &branch, b));
    // Merged, or nothing of its own to merge or to lose, it is not held by its merge.
    let own = if merged || branch.is_empty() { Some(false) } else { own_commits(&repo, &branch, base.as_deref()) };
    match (&base, own) {
        (_, None) => out.push(Blocker::Unchecked("whether it merged".into())),
        (_, Some(false)) => {}
        (Some(b), Some(true)) => out.push(Blocker::NotMerged(b.rsplit('/').next().unwrap_or(b).to_string())),
        (None, Some(true)) => out.push(Blocker::UnknownBase),
    }
    Look { branch, repo, changes, merged, blockers: out }
}

/// Why the worktree at `path` stays; empty when it can go.
pub fn blockers(path: &Path, facts: &Facts) -> Vec<Blocker> {
    look(path, facts).blockers
}

/// Commits nothing else has: past its upstream while it has one, else past `pushed` (the commit the remote last had, as
/// the code host or a prune said it), else past every remote branch and every other local one; None when git could not
/// say.
fn unpushed(path: &Path, reference: &str, pushed: Option<&str>) -> Option<usize> {
    let count = |range: &[&str]| git(path, &[&["rev-list", "--count"], range].concat()).and_then(|n| n.parse().ok());
    let track = if reference.is_empty() { String::new() } else { git(path, &["for-each-ref", "--format=%(upstream)|%(upstream:track)", reference])? };
    match (track.split_once('|'), pushed) {
        (Some((upstream, track)), _) if !upstream.is_empty() && track != "[gone]" => count(&[&format!("{upstream}..HEAD")]),
        (_, Some(oid)) if git(path, &["cat-file", "-e", &format!("{oid}^{{commit}}")]).is_some() => count(&[&format!("{oid}..HEAD")]),
        _ if reference.is_empty() => count(&["HEAD", "--not", "--branches", "--remotes"]),
        // `--exclude` names a branch as `--branches` reads it, without refs/heads/.
        _ => count(&["HEAD", "--not", &format!("--exclude={}", reference.trim_start_matches("refs/heads/")), "--branches", "--remotes"]),
    }
}

/// Why `path` is locked: Some(None) when it is not, Some(Some(reason)) when it is (an empty reason when none was given),
/// None when git could not say.
fn locked(repo: &Path, path: &Path) -> Option<Option<String>> {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let list = git(repo, &["worktree", "list", "--porcelain"])?;
    let mut here = false;
    for line in list.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            here = Path::new(p) == real || Path::new(p) == path;
        } else if here && let Some(rest) = line.strip_prefix("locked") {
            return Some(Some(rest.trim().to_string()));
        }
    }
    Some(None)
}

/// The blockers as the view models carry them: `[{id, words}]`.
pub fn json(blockers: &[Blocker]) -> Value {
    Value::Array(blockers.iter().map(Blocker::json).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clone of a local bare remote, with `origin/HEAD` set, and a worktree on `branch` next to it.
    struct Repo {
        dir: tempfile::TempDir,
    }

    impl Repo {
        fn root(&self) -> PathBuf {
            std::fs::canonicalize(self.dir.path()).unwrap()
        }

        fn main(&self) -> PathBuf {
            self.root().join("api")
        }

        fn git(&self, dir: &Path, args: &[&str]) {
            let out = Command::new("git")
                .args(["-c", "user.name=me", "-c", "user.email=me@acme.example", "-c", "init.defaultBranch=main", "-C"])
                .arg(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }

        fn new() -> Repo {
            let repo = Repo { dir: tempfile::tempdir().unwrap() };
            let root = repo.root();
            repo.git(&root, &["init", "-q", "--bare", "remote.git"]);
            repo.git(&root, &["clone", "-q", "remote.git", "api"]);
            repo.git(&repo.main(), &["commit", "-q", "--allow-empty", "-m", "start"]);
            repo.git(&repo.main(), &["push", "-q", "-u", "origin", "main"]);
            repo.git(&repo.main(), &["remote", "set-head", "origin", "--auto"]);
            repo
        }

        fn worktree(&self, branch: &str) -> PathBuf {
            let path = self.root().join("wt").join(branch.replace('/', "-"));
            self.git(&self.main(), &["worktree", "add", "-q", "-b", branch, &path.to_string_lossy(), "main"]);
            path
        }

        /// A commit adding a file named after its message, so a squash has something to carry.
        fn commit(&self, path: &Path, message: &str) {
            let file = format!("{}.txt", message.replace(' ', "-"));
            std::fs::write(path.join(&file), message).unwrap();
            self.git(path, &["add", &file]);
            self.git(path, &["commit", "-q", "-m", message]);
        }
    }

    /// Facts with no program running anywhere.
    fn quiet() -> Facts {
        Facts { processes: Some(Vec::new()), ..Facts::default() }
    }

    fn host(oid: &str) -> Facts {
        Facts { merged_at: Some(oid.into()), ..quiet() }
    }

    fn tip(r: &Repo, path: &Path) -> String {
        git(path, &["rev-parse", "HEAD"]).unwrap_or_else(|| panic!("no tip in {}", r.root().display()))
    }

    #[test]
    fn a_branch_merged_and_pushed_with_nothing_in_it_can_go() {
        let r = Repo::new();
        let wt = r.worktree("me/eng-412");
        r.commit(&wt, "quota model");
        r.git(&wt, &["push", "-q", "-u", "origin", "me/eng-412"]);
        r.git(&r.main(), &["merge", "-q", "--ff-only", "me/eng-412"]);
        r.git(&r.main(), &["push", "-q", "origin", "main"]);
        let seen = look(&wt, &quiet());
        assert_eq!((seen.blockers, seen.merged), (vec![], true), "its commits reached origin/main");
        r.git(&r.main(), &["push", "-q", "origin", "--delete", "me/eng-412"]);
        r.git(&r.main(), &["fetch", "-q", "--prune"]);
        assert_eq!(blockers(&wt, &quiet()), [], "its upstream went with the merge");
    }

    #[test]
    fn a_squash_merge_counts_for_the_commit_it_merged_not_for_work_after_it() {
        let r = Repo::new();
        let wt = r.worktree("me/eng-413");
        r.commit(&wt, "banner");
        r.git(&wt, &["push", "-q", "-u", "origin", "me/eng-413"]);
        let merged_at = tip(&r, &wt);
        // Squashed on the host: main gets a new commit, the branch goes.
        r.git(&r.main(), &["merge", "-q", "--squash", "me/eng-413"]);
        r.git(&r.main(), &["commit", "-q", "-m", "banner (#7)"]);
        r.git(&r.main(), &["push", "-q", "origin", "main", ":me/eng-413"]);
        r.git(&r.main(), &["fetch", "-q", "--prune"]);
        assert_eq!(blockers(&wt, &host(&merged_at)), [], "the host merged this very commit");
        assert_eq!(said(&blockers(&wt, &quiet())), "1 commit not pushed, not merged into main", "without the host's word nothing says it merged");
        r.commit(&wt, "after the merge");
        assert_eq!(said(&blockers(&wt, &host(&merged_at))), "1 commit not pushed, not merged into main", "work after the merge stays");
        let pruned = Facts { pruned: Some(merged_at.clone()), ..quiet() };
        assert_eq!(blockers(&wt, &pruned)[0], Blocker::Unpushed(1), "a prune says what the remote had");
    }

    #[test]
    fn a_branch_gone_without_a_merge_is_not_merged() {
        let r = Repo::new();
        let wt = r.worktree("me/eng-398");
        r.commit(&wt, "csv");
        r.git(&wt, &["push", "-q", "-u", "origin", "me/eng-398"]);
        assert_eq!(said(&blockers(&wt, &quiet())), "not merged into main");
        // The pull request closed unmerged and its branch went.
        r.git(&r.main(), &["push", "-q", "origin", ":me/eng-398"]);
        r.git(&r.main(), &["fetch", "-q", "--prune"]);
        assert!(blockers(&wt, &quiet()).contains(&Blocker::NotMerged("main".into())), "a gone upstream alone is no merge");
    }

    #[test]
    fn each_blocker_is_found_and_said_the_same_way() {
        let r = Repo::new();
        let dirty = r.worktree("me/eng-413");
        r.git(&dirty, &["config", "status.showUntrackedFiles", "no"]);
        std::fs::write(dirty.join("notes.md"), "draft").unwrap();
        std::fs::write(dirty.join("todo.md"), "draft").unwrap();
        let found = blockers(&dirty, &quiet());
        assert_eq!(found, [Blocker::Uncommitted(2)], "untracked files count whatever the config hides");
        assert_eq!(said(&found), "2 uncommitted changes");

        let ahead = r.worktree("me/eng-414");
        r.commit(&ahead, "copy");
        r.git(&ahead, &["push", "-q", "-u", "origin", "me/eng-414"]);
        r.commit(&ahead, "more copy");
        assert_eq!(said(&blockers(&ahead, &host(&tip(&r, &ahead)))), "1 commit not pushed");
        let local = r.worktree("me/local");
        r.commit(&local, "a");
        r.commit(&local, "b");
        assert_eq!(said(&blockers(&local, &quiet())), "2 commits not pushed, not merged into main", "never pushed anywhere");

        let kept = r.worktree("me/eng-415");
        std::fs::write(kept.join(".gitignore"), ".env\nnode_modules/\n").unwrap();
        r.git(&kept, &["add", ".gitignore"]);
        r.git(&kept, &["commit", "-q", "-m", "ignore"]);
        r.git(&kept, &["push", "-q", "-u", "origin", "me/eng-415"]);
        std::fs::write(kept.join(".env"), "TOKEN=x").unwrap();
        std::fs::create_dir_all(kept.join("node_modules/x")).unwrap();
        std::fs::write(kept.join("node_modules/x/i.js"), "").unwrap();
        assert_eq!(said(&blockers(&kept, &host(&tip(&r, &kept)))), "1 ignored file other than builds", "a build folder is not one");

        let fresh = r.worktree("me/fresh");
        assert_eq!(blockers(&fresh, &quiet()), [], "nothing of its own to merge or lose");
        assert!(!merged(&r.main(), "me/fresh", "refs/remotes/origin/main"), "yet nothing merged, for the cleanup that runs alone");
        assert!(!has_own_commits(&r.main(), "", None), "no branch, nothing of its own");

        let busy = r.worktree("me/eng-420");
        let inside = |name: &str| Process { pid: 1, name: name.into(), cwd: busy.join("src") };
        let running = |names: &[&str]| Facts { processes: Some(names.iter().map(|n| inside(n)).collect()), ..Facts::default() };
        assert_eq!(said(&blockers(&busy, &running(&["zsh", "claude"]))), "Claude Code still runs in it");
        assert_eq!(said(&blockers(&busy, &running(&["zsh", "node"]))), "node still runs in it");
        assert_eq!(blockers(&busy, &running(&["zsh", "herdr-core", "-zsh"])), [], "a shell and herdr's own sit in the terminal its space closes");
        assert_eq!(said(&blockers(&busy, &Facts::default())), "cannot tell what runs in it");
        let agent = Facts { agents: vec![Working { name: "Codex".into(), cwd: busy.clone(), pane: "w9:p1".into(), space: "w9".into() }], ..quiet() };
        assert_eq!(said(&blockers(&busy, &agent)), "Codex still runs in it", "the hub's agents count without lsof");

        let locked = r.worktree("me/eng-422");
        r.git(&r.main(), &["worktree", "lock", "--reason", "on a USB disk", &locked.to_string_lossy()]);
        assert_eq!(said(&blockers(&locked, &quiet())), "locked: on a USB disk");
        let modules = r.worktree("me/eng-423");
        std::fs::write(modules.join(".gitmodules"), "[submodule \"x\"]\n\tpath = x\n\turl = ../x\n").unwrap();
        r.git(&modules, &["add", ".gitmodules"]);
        r.git(&modules, &["commit", "-q", "-m", "x"]);
        r.git(&modules, &["push", "-q", "-u", "origin", "me/eng-423"]);
        assert_eq!(said(&blockers(&modules, &host(&tip(&r, &modules)))), "it holds submodules");
    }

    #[test]
    fn without_a_reflog_own_commits_are_those_the_default_branch_lacks() {
        let r = Repo::new();
        r.git(&r.main(), &["config", "core.logAllRefUpdates", "false"]);
        let wt = r.worktree("me/eng-424");
        let reflog = git(&r.main(), &["reflog", "show", "refs/heads/me/eng-424", "--"]).unwrap_or_default();
        if !reflog.is_empty() {
            // This git logs new branches whatever the config: the reflog path is the one tested above.
            return;
        }
        assert!(!has_own_commits(&r.main(), "me/eng-424", Some("refs/remotes/origin/main")));
        r.commit(&wt, "x");
        assert!(has_own_commits(&r.main(), "me/eng-424", Some("refs/remotes/origin/main")));
    }

    #[test]
    fn a_repository_without_origin_head_cannot_say_what_merged() {
        let r = Repo::new();
        r.git(&r.main(), &["remote", "set-head", "origin", "--delete"]);
        changed();
        let wt = r.worktree("me/eng-421");
        r.commit(&wt, "x");
        r.git(&wt, &["push", "-q", "-u", "origin", "me/eng-421"]);
        let found = blockers(&wt, &quiet());
        assert_eq!(found, [Blocker::UnknownBase]);
        assert_eq!(said(&found), "its default branch is unknown (git remote set-head origin --auto)");
        assert_eq!(json(&found), json!([{"id": "unknown_base", "words": "its default branch is unknown (git remote set-head origin --auto)"}]));
        let guessed = Facts { guess_base: true, ..quiet() };
        assert_eq!(said(&blockers(&wt, &guessed)), "not merged into main", "the routines' run worktrees read main instead");
    }

    #[test]
    fn a_prune_says_the_commit_each_branch_had_and_a_fetch_is_said_again_within_the_minute() {
        let r = Repo::new();
        let wt = r.worktree("me/eng-425");
        r.commit(&wt, "x");
        r.git(&wt, &["push", "-q", "-u", "origin", "me/eng-425"]);
        let had = tip(&r, &wt);
        // Deleted on the remote from another clone, so only a fetch here can prune it.
        r.git(&r.root(), &["clone", "-q", "remote.git", "other"]);
        r.git(&r.root().join("other"), &["push", "-q", "origin", ":me/eng-425"]);
        let pruned = fetch(&r.main()).expect("the fetch ran");
        assert_eq!(pruned.get("me/eng-425"), Some(&had), "{pruned:?}");
        assert_eq!(fetch(&r.main()), Some(pruned), "fetched a moment ago: said again, not fetched");
        // A fetch that failed is said as failed for the minute after.
        let lost = Repo::new();
        lost.git(&lost.main(), &["remote", "set-url", "origin", &lost.root().join("gone.git").to_string_lossy()]);
        assert_eq!(fetch(&lost.main()), None);
        assert_eq!(fetch(&lost.main()), None, "never an empty answer in place of a failure");
    }

    #[test]
    fn a_probe_that_fails_keeps_the_worktree_and_says_what_it_could_not_tell() {
        let r = Repo::new();
        let wt = r.worktree("me/eng-426");
        std::fs::write(wt.join(".gitignore"), ".env\n").unwrap();
        r.git(&wt, &["add", ".gitignore"]);
        r.git(&wt, &["commit", "-q", "-m", "ignore"]);
        r.git(&wt, &["push", "-q", "-u", "origin", "me/eng-426"]);
        std::fs::write(wt.join(".env"), "TOKEN=x").unwrap();
        let merged_at = tip(&r, &wt);
        // An index git cannot read makes `status` fail, as a timeout would.
        let index = PathBuf::from(git(&wt, &["rev-parse", "--path-format=absolute", "--git-path", "index"]).unwrap());
        std::fs::write(&index, "not an index").unwrap();
        let found = blockers(&wt, &host(&merged_at));
        assert!(found.contains(&Blocker::Unchecked("whether it has uncommitted or ignored files".into())), "{found:?}");
        assert_eq!(said(&found[..1]), "cannot tell whether it has uncommitted or ignored files");
        let gone = r.root().join("not-a-worktree");
        std::fs::create_dir_all(&gone).unwrap();
        assert_eq!(said(&blockers(&gone, &quiet())), "cannot tell what it is: git cannot read it");
    }

    #[test]
    fn the_routines_template_names_and_spares_as_these_rules_do() {
        let template = include_str!("routine-cleanup.sh");
        for build in BUILDS {
            assert!(template.contains(&build.replace('.', "\\.")), "the template spares {build} as a build");
        }
        for idle in IDLE {
            assert!(
                template.contains(&format!("|{idle}|")) || template.contains(&format!("|{idle})")) || template.contains(&format!("({idle}|")),
                "{idle} is idle there too"
            );
        }
        for agent in crate::agent::builtin() {
            let case = template.lines().find(|l| l.trim_start().split(')').next().is_some_and(|p| p.split('|').any(|w| w == agent.id)));
            let case = case.unwrap_or_else(|| panic!("the template names no {}", agent.id));
            assert!(case.contains(&format!("echo \"{}\"", agent.name)), "{} is {} there too: {case}", agent.id, agent.name);
            for alias in &agent.aliases {
                assert!(case.split(')').next().unwrap().split('|').any(|w| w.trim() == alias), "{alias} reads as {} there too", agent.name);
            }
        }
    }

    /// The machine's real programs, read as a removal reads them: /proc on Linux, lsof on macOS.
    #[test]
    fn a_program_working_in_a_worktree_is_found_there_by_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap().join("src");
        std::fs::create_dir_all(&folder).unwrap();
        let mut child = Command::new("sleep").arg("30").current_dir(&folder).stdin(Stdio::null()).spawn().unwrap();
        // Right after it starts, Linux may name it for a moment after the thread that started it.
        let named = |found: &Option<Vec<Process>>| found.iter().flatten().any(|p| p.pid == child.id() && p.name == "sleep");
        let start = Instant::now();
        let mut found = processes();
        while !named(&found) && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
            found = processes();
        }
        let alive = child.try_wait().unwrap().is_none();
        let _ = child.kill();
        let _ = child.wait();
        assert!(alive, "sleep ran all along");
        let found = found.expect("this machine says what runs where");
        let seen: Vec<&Process> = found.iter().filter(|p| p.pid == child.id()).collect();
        assert_eq!(seen, [&Process { pid: child.id(), name: "sleep".into(), cwd: folder }]);
        assert_eq!(running(dir.path(), Some(&found), &[]), Inside::Running("sleep".into()));
    }

    #[test]
    fn lsof_reads_as_programs_and_their_folders() {
        let text = "p12\nccodex\nn/w/api/eng-4\np13\nczsh\nn/home/me\n";
        assert_eq!(
            parse_lsof(text),
            [Process { pid: 12, name: "codex".into(), cwd: "/w/api/eng-4".into() }, Process { pid: 13, name: "zsh".into(), cwd: "/home/me".into() }]
        );
    }
}
