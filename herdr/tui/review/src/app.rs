//! The space's state and what changes it: whose worktree it reviews (the agents the hub lists, polled), who the
//! comments go to (an agent on that worktree, a driven session among them, or a new agent), each worktree's scopes read
//! in the background with the comments written on them, the walkthrough, and the calls to the hub's review actions
//! (`send`, `walkthrough`, `stop_walkthrough`) and to Start work, the same the Herdr app's panel makes.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use cockpit_core::bridge::Bridge;
use cockpit_core::dialog::Dialog;
use cockpit_core::palette::Palette;
use cockpit_core::toast::{Kind as Toast, Toasts};
use cockpit_show::diff::{Diff, Note};
use cockpit_show::review::{self, Read};
use cockpit_show::row::Row;
use ratatui::layout::Rect;
use serde_json::{Value, json};

use crate::subject::{self, Agent};
use crate::walk;

/// How often the changes are read again while the space has the focus; a pull request less often, since `gh` is slow
/// and rate limited, and a walkthrough being written more often.
const EVERY: Duration = Duration::from_secs(3);
const PR_EVERY: Duration = Duration::from_secs(20);
const WRITING_EVERY: Duration = Duration::from_secs(2);
/// How often the changes and the agents are read while the space is out of view.
const AWAY_EVERY: Duration = Duration::from_secs(30);
const AGENTS_EVERY: Duration = Duration::from_secs(3);
/// A pinned agent missing from this many polls in a row is gone; once is a hiccup.
const GONE_AFTER: u32 = 3;
/// A walkthrough asked for is not asked for again while it starts.
const WRITE_STARTS: Duration = Duration::from_secs(15);
/// The picker's options are cut to fit its box on one line each.
const PICKED: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Uncommitted,
    Branch,
    Pr,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::Uncommitted, Scope::Branch, Scope::Pr];

    /// The name the hub's review actions take.
    pub fn id(self) -> &'static str {
        match self {
            Scope::Uncommitted => "uncommitted",
            Scope::Branch => "branch",
            Scope::Pr => "pr",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scope::Uncommitted => "Uncommitted",
            Scope::Branch => "Branch",
            Scope::Pr => "PR",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Diff,
    Walk,
}

/// What a key, a palette entry or a click runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Comment,
    Edit,
    Delete,
    Undo,
    Clear,
    Send,
    SideBySide,
    Hunk(bool),
    FoldAll,
    Scope(Scope),
    /// Diff ↔ walkthrough.
    Walk,
    /// The worktree picker.
    Switch,
    /// The agent whose worktree is reviewed, by its handle.
    Pick(String),
    /// The recipient picker.
    ToPicker,
    /// Who the comments on this worktree go to.
    To(To),
    OpenPr,
    Refresh,
    /// Writes the walkthrough (again).
    Write,
    Stop,
    Model,
    Step(isize),
    Copy,
}

/// What the dialog over the space asks: which agent's worktree (each option's handle), who the comments go to, or
/// whether to leave comments unsent.
pub enum Ask {
    Pick(Vec<String>),
    To(Vec<To>),
    Leave,
}

/// Who a worktree's comments go to: one of its agents (as last seen), a new agent started on the review, or the agent
/// a review started, by its space's label.
#[derive(Clone, Debug, PartialEq)]
pub enum To {
    Agent(Agent),
    New,
    Started(String),
}

/// The recipient as it stands now: an agent still running in the worktree, one gone or working elsewhere since it was
/// picked, a new agent, or the one a review started that is not running yet.
#[derive(Clone, Debug, PartialEq)]
pub enum Recipient {
    Agent(Agent),
    Gone(Agent),
    New,
    Starting,
}

impl Recipient {
    /// What the header and the toasts call it.
    pub fn name(&self) -> String {
        match self {
            Recipient::Agent(a) => a.name.clone(),
            Recipient::Gone(a) => format!("{} (gone)", a.name),
            Recipient::New => "a new agent".into(),
            Recipient::Starting => "the review agent (starting)".into(),
        }
    }
}

/// How long a second S may follow the first that asked before a repo's first new agent.
const CONFIRM: Duration = Duration::from_secs(8);

/// A call to the hub, as the app sent it: what tests read.
#[derive(Clone, Debug, PartialEq)]
pub struct Sent {
    pub app: String,
    pub action: String,
    pub args: Vec<Value>,
}

/// One scope's changes: the diff with its comments, where they come from, the walkthrough over them, when they were
/// last read, or why they could not be.
pub struct Changes {
    pub diff: Diff,
    pub about: Value,
    pub walkthrough: Value,
    pub error: Option<String>,
    pub read_at: Option<u64>,
    pub step: usize,
}

impl Changes {
    fn new() -> Self {
        Self { diff: Diff::new(&json!({"patch": ""})), about: Value::Null, walkthrough: json!({"status": "none"}), error: None, read_at: None, step: 0 }
    }

    fn three() -> [Changes; 3] {
        [Changes::new(), Changes::new(), Changes::new()]
    }
}

/// The spans of a row a click acts on: from, to and what it does.
pub type Clicks = Vec<(usize, usize, cockpit_show::row::Act)>;

/// Where the last frame drew what a click can land on.
#[derive(Default)]
pub struct Hits {
    pub body: Rect,
    pub tabs: Vec<(Rect, Scope)>,
    /// Each drawn row's screen line, its first cell and the spans a click acts on.
    pub rows: Vec<(u16, u16, Clicks)>,
}

/// What the agents' poll is told: the space came into view or left it, or R asked to look again from scratch.
enum Poll {
    Focus(bool),
    Again,
}

pub struct App {
    /// None until the hub first answers.
    pub agents: Option<Vec<Agent>>,
    pub hub_error: Option<String>,
    /// The agent the review stays on: picked with W, or the one in view once the person works on its changes.
    pub chosen: Option<Agent>,
    /// Polls in a row the chosen agent was missing from.
    missing: u32,
    /// The worktree whose changes are shown.
    pub repo: Option<String>,
    pub scope: Scope,
    pub mode: Mode,
    /// Each worktree's three scopes, kept while an agent works there or its comments wait.
    pub repos: HashMap<String, [Changes; 3]>,
    /// Who each worktree's comments go to, when the person chose.
    pub to: HashMap<String, To>,
    blank: [Changes; 3],
    pub model: usize,
    pub toasts: Toasts,
    pub palette: Palette,
    pub cmds: Vec<Cmd>,
    pub dialog: Option<(Dialog, Ask)>,
    pub scroll: usize,
    pub reveal: bool,
    pub hits: Hits,
    /// Every call sent, newest last.
    pub sent: Vec<Sent>,
    /// The rows of the body last drawn, for paging.
    pub height: usize,
    pub tick: u64,
    /// How a read's time shows ("10:42:07"): the local clock, fixed in tests.
    pub clock: fn(u64) -> String,
    /// The body's rows as last built, for their width; built again when something they show changed.
    pub(crate) rows: Option<(usize, Vec<Row>)>,
    pub(crate) dirty: bool,
    live: bool,
    bridge: Bridge,
    calls: usize,
    /// Sends waiting for their reply, by tag: the comments they carry, marked sent once the agent took them.
    sending: HashMap<String, Vec<Note>>,
    /// The new agents being started, by the send's tag: the worktree and the space's label.
    starting: HashMap<String, (String, String)>,
    /// The worktrees whose review started a new agent once already, and the one whose first was asked about.
    new_ok: HashSet<String>,
    armed_new: Option<(String, Instant)>,
    serial: u64,
    reads: Option<Receiver<Read>>,
    reading: Option<(String, Scope)>,
    next_read: Instant,
    /// When a walkthrough was last asked for, until a read shows it started.
    writing: Option<Instant>,
    agents_rx: Option<Receiver<Result<Vec<Agent>, String>>>,
    poll: Option<Sender<Poll>>,
    focused: bool,
    pinned: bool,
    reported: Option<String>,
    toasting: bool,
}

impl App {
    /// The space as it runs: the agents polled from the hub, the changes read from git.
    pub fn start() -> Self {
        let mut app = Self::new(crate::pinned::is_pinned());
        app.live = true;
        let (poll, agents) = poll_agents();
        (app.poll, app.agents_rx) = (Some(poll), Some(agents));
        app
    }

    /// The space with no agents yet and nothing read: what tests feed.
    pub fn new(pinned: bool) -> Self {
        let mut palette = Palette::default();
        palette.app = "review".into();
        Self {
            agents: None,
            hub_error: None,
            chosen: None,
            missing: 0,
            repo: None,
            scope: Scope::Uncommitted,
            mode: Mode::Diff,
            repos: HashMap::new(),
            to: HashMap::new(),
            blank: Changes::three(),
            model: 0,
            toasts: Toasts::default(),
            palette,
            cmds: Vec::new(),
            dialog: None,
            scroll: 0,
            reveal: false,
            hits: Hits::default(),
            sent: Vec::new(),
            height: 20,
            tick: 0,
            clock: local_time,
            rows: None,
            dirty: true,
            live: false,
            bridge: Bridge::default(),
            calls: 0,
            sending: HashMap::new(),
            starting: HashMap::new(),
            new_ok: HashSet::new(),
            armed_new: None,
            serial: 0,
            reads: None,
            reading: None,
            next_read: Instant::now(),
            writing: None,
            agents_rx: None,
            poll: None,
            focused: true,
            pinned,
            reported: None,
            toasting: false,
        }
    }

    pub fn here(&self) -> &Changes {
        let scopes = self.repo.as_ref().and_then(|r| self.repos.get(r)).unwrap_or(&self.blank);
        &scopes[self.scope.index()]
    }

    pub fn here_mut(&mut self) -> &mut Changes {
        let scope = self.scope.index();
        match self.repo.as_ref().and_then(|r| self.repos.get_mut(r)) {
            Some(scopes) => &mut scopes[scope],
            None => &mut self.blank[scope],
        }
    }

    /// The diff of the scope in view.
    pub fn diff(&self) -> &Diff {
        &self.here().diff
    }

    /// The agent the comments go to: the one the review stays on (as last seen while a poll misses it), else the most
    /// recently active.
    pub fn subject(&self) -> Option<&Agent> {
        let agents = self.agents.as_ref()?;
        match &self.chosen {
            Some(chosen) => Some(agents.iter().find(|a| a.terminal == chosen.terminal).unwrap_or(chosen)),
            None => agents.iter().find(|a| a.tui),
        }
    }

    /// The agents working in the reviewed worktree, the most recently active first.
    pub fn on_worktree(&self) -> Vec<&Agent> {
        self.agents.iter().flatten().filter(|a| Some(&a.repo) == self.repo.as_ref()).collect()
    }

    /// Who the comments go to: the one chosen for this worktree (said gone when it stopped), else the agent the review
    /// follows, else one of the worktree's agents, else a new agent.
    pub fn recipient(&self) -> Option<Recipient> {
        self.recipient_of(self.repo.as_ref()?)
    }

    /// Who the comments on `repo` go to, by the same rules.
    pub fn recipient_of(&self, repo: &str) -> Option<Recipient> {
        let repo = &repo.to_string();
        // A choice counts while it still works in this worktree, by git's name for its folder.
        let here = |found: &dyn Fn(&Agent) -> bool| self.agents.iter().flatten().find(|a| &a.repo == repo && found(a)).cloned();
        match self.to.get(repo) {
            Some(To::New) => return Some(Recipient::New),
            Some(To::Agent(a)) => return Some(here(&|b| b.terminal == a.terminal).map_or_else(|| Recipient::Gone(a.clone()), Recipient::Agent)),
            Some(To::Started(label)) => return Some(here(&|b| b.space == *label).map_or(Recipient::Starting, Recipient::Agent)),
            None => {}
        }
        let followed = self.subject().filter(|a| &a.repo == repo).cloned();
        Some(followed.or_else(|| self.on_worktree().first().map(|a| (*a).clone())).map_or(Recipient::New, Recipient::Agent))
    }

    /// The comments waiting to be sent anywhere: in any scope of any worktree, their lines in the diff or not.
    pub fn unsent(&self) -> usize {
        self.repos.values().flatten().map(|c| c.diff.notes().len() + c.diff.orphans().len()).sum()
    }

    /// The agents as the hub's view lists them, each folder named by its repository (tests give it).
    pub fn set_agents(&mut self, view: &Value, toplevel: &dyn Fn(&str) -> Option<String>) {
        self.take_agents(Ok(subject::agents(view, toplevel)));
    }

    /// What the agents poll found: the agents, or why the hub did not answer. The review stays on its agent; one
    /// missing for a few polls in a row is let go.
    pub fn take_agents(&mut self, found: Result<Vec<Agent>, String>) {
        let before = self.subject().map(|a| a.terminal.clone());
        match found {
            Ok(agents) => {
                if let Some(chosen) = self.chosen.clone() {
                    match agents.iter().find(|a| a.terminal == chosen.terminal) {
                        Some(now) => (self.chosen, self.missing) = (Some(now.clone()), 0),
                        None => {
                            self.missing += 1;
                            if self.missing >= GONE_AFTER {
                                self.toasts.push(format!("{} is gone: back to the most recently active agent", chosen.name), Toast::Note);
                                (self.chosen, self.missing) = (None, 0);
                            }
                        }
                    }
                }
                // A worktree no agent works in any more is let go, unless comments wait there.
                let live: HashSet<String> = agents.iter().map(|a| a.repo.clone()).chain(self.chosen.iter().map(|a| a.repo.clone())).collect();
                self.repos.retain(|repo, scopes| live.contains(repo) || scopes.iter().any(|c| c.diff.has_drafts()));
                (self.agents, self.hub_error) = (Some(agents), None);
            }
            Err(error) => self.hub_error = Some(error),
        }
        self.follow_subject(before);
        self.dirty = true;
    }

    /// The review on the subject's worktree, its scopes as they were left; a subject that changed on its own says so.
    fn follow_subject(&mut self, before: Option<String>) {
        let Some((terminal, repo, name)) = self.subject().map(|a| (a.terminal.clone(), a.repo.clone(), a.name.clone())) else {
            self.repo = None;
            return;
        };
        if before.as_ref().is_some_and(|b| *b != terminal) {
            let worktree = repo.rsplit('/').next().unwrap_or(&repo).to_string();
            self.toasts.push(format!("Now reviewing {name} · {worktree}"), Toast::Note);
        }
        if self.repo.as_deref() == Some(repo.as_str()) {
            return;
        }
        self.repos.entry(repo.clone()).or_insert_with(Changes::three);
        self.repo = Some(repo);
        (self.scroll, self.next_read, self.reading) = (0, Instant::now(), None);
        self.apply_step();
    }

    /// The review stays on the agent in view: the person works on its changes.
    pub fn pin(&mut self) {
        if self.chosen.is_none() {
            self.chosen = self.subject().cloned();
        }
    }

    /// A read came back: its worktree's scope takes it, whichever worktree is in view now.
    pub fn arrive(&mut self, read: Read) {
        let Some(scope) = Scope::ALL.into_iter().find(|s| s.id() == read.scope) else { return };
        let Some(scopes) = self.repos.get_mut(&read.repo) else { return };
        let changes = &mut scopes[scope.index()];
        changes.read_at = Some(cockpit_core::seen::now_ms());
        changes.about = read.about;
        match read.error {
            Some(error) => changes.error = Some(error),
            None => {
                changes.error = None;
                if walk::status(&read.walkthrough) != "none" {
                    self.writing = None;
                }
                changes.walkthrough = read.walkthrough;
                changes.diff.arrived(read.hash, read.files, None);
            }
        }
        self.apply_step();
        self.dirty = true;
    }

    /// The step in view shows its hunks alone in walkthrough mode; the diff shows every hunk otherwise.
    pub fn apply_step(&mut self) {
        let walking = self.mode == Mode::Walk;
        let here = self.here_mut();
        let steps = walk::steps(&here.walkthrough);
        here.step = here.step.min(steps.len().saturating_sub(1));
        let only = steps.get(here.step).filter(|_| walking).map(walk::Step::only);
        here.diff.set_only(only);
    }

    /// How often the changes are read again now.
    fn every(&self) -> Duration {
        let writing = walk::status(&self.here().walkthrough) == "running";
        match (self.focused, self.scope, writing) {
            (false, _, _) => AWAY_EVERY,
            (_, _, true) => WRITING_EVERY,
            (_, Scope::Pr, _) => PR_EVERY,
            _ => EVERY,
        }
    }

    /// Reads the scope's changes in the background when a read is due; none while a comment is typed, so its line
    /// stays where it is.
    fn read_if_due(&mut self) {
        let due = self.live && self.reading.is_none() && Instant::now() >= self.next_read && !self.diff().is_typing();
        let Some(repo) = self.repo.clone().filter(|_| due) else { return };
        let (scope, since) = (self.scope, self.here().diff.shown());
        let (tx, rx) = channel();
        self.reads = Some(rx);
        self.reading = Some((repo.clone(), scope));
        std::thread::spawn(move || {
            let _ = tx.send(review::read(&repo, scope.id(), since));
            cockpit_core::term::wake();
        });
    }

    fn take_reads(&mut self) -> bool {
        let Some(read) = self.reads.as_ref().and_then(|rx| rx.try_recv().ok()) else { return false };
        self.reading = None;
        // A worktree or scope switched to while another was read is read at once.
        let current = read.scope == self.scope.id() && self.repo.as_deref() == Some(read.repo.as_str());
        self.arrive(read);
        self.next_read = Instant::now() + if current { self.every() } else { Duration::ZERO };
        true
    }

    /// Reads the changes again now.
    pub fn refresh(&mut self) {
        self.next_read = Instant::now();
    }

    pub fn is_busy(&self) -> bool {
        self.calls > 0 || self.toasting || (self.mode == Mode::Walk && walk::status(&self.here().walkthrough) == "running")
    }

    /// The space came into view or left it: reads and polls slow down while it is away.
    pub fn focus(&mut self, on: bool) {
        self.focused = on;
        if let Some(poll) = &self.poll {
            let _ = poll.send(Poll::Focus(on));
        }
        if on {
            self.refresh();
        }
    }

    pub fn tick(&mut self) -> bool {
        let mut changed = self.take_replies();
        if let Some(found) = self.agents_rx.as_ref().and_then(|rx| rx.try_iter().last()) {
            let before = (self.agents.clone(), self.hub_error.clone());
            self.take_agents(found);
            changed |= before != (self.agents.clone(), self.hub_error.clone());
        }
        changed |= self.take_reads();
        self.read_if_due();
        self.report();
        if self.mode == Mode::Walk && walk::status(&self.here().walkthrough) == "running" {
            self.tick += 1;
            (changed, self.dirty) = (true, true);
        }
        let toasting = self.toasts.any();
        changed |= toasting || self.toasting;
        self.toasting = toasting;
        changed
    }

    /// The sidebar row's line, sent when it changed.
    pub fn report(&mut self) {
        if !self.pinned {
            return;
        }
        let text = crate::pinned::sidebar_text(self);
        if self.reported.as_ref() != Some(&text) {
            crate::pinned::report(text.clone());
            self.reported = Some(text);
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Calls

    fn call(&mut self, tag: &str, action: &str, args: Vec<Value>) {
        self.call_app("review", tag, action, args);
    }

    fn call_app(&mut self, app: &str, tag: &str, action: &str, args: Vec<Value>) {
        self.sent.push(Sent { app: app.into(), action: action.into(), args: args.clone() });
        if self.live {
            self.calls += 1;
            self.bridge.call(tag, app, action, args, None);
        }
    }

    pub fn take_replies(&mut self) -> bool {
        let mut any = false;
        while let Ok(reply) = self.bridge.replies.try_recv() {
            self.calls = self.calls.saturating_sub(1);
            self.replied(&reply.tag, &reply.note, reply.error);
            any = true;
        }
        any
    }

    /// What a call's reply does: a send that went through marks its comments sent; an error shows.
    pub fn replied(&mut self, tag: &str, note: &str, error: Option<String>) {
        let notes = self.sending.remove(tag);
        match error {
            Some(error) => {
                // The hub names the action before its error: the person reads the error alone.
                self.starting.remove(tag);
                let error = error.strip_prefix("send: ").or_else(|| error.strip_prefix("work: ")).unwrap_or(&error).to_string();
                self.toasts.push(error, Toast::Bad);
                if tag == "walkthrough" {
                    self.writing = None;
                }
            }
            None => {
                if let Some(notes) = notes {
                    for changes in self.repos.values_mut().flatten() {
                        changes.diff.mark_sent(&notes);
                    }
                }
                // A new agent is picked once: the next review of the worktree goes to the agent this one started.
                if let Some((repo, label)) = self.starting.remove(tag) {
                    self.to.insert(repo, To::Started(label));
                }
                self.toasts.push(note.to_string(), Toast::Good);
            }
        }
        if !tag.starts_with("send") {
            self.refresh();
        }
        self.dirty = true;
    }

    /// Sends the scope's comments as one message: to an agent of the worktree through `deliver` (which refuses while it
    /// waits on a question), or to a new agent Start work opens on the worktree with the review as its prompt. One review
    /// goes at a time.
    pub fn send(&mut self) {
        let Some(recipient) = self.recipient() else {
            return self.toasts.push("No worktree to review: W picks an agent's", Toast::Bad);
        };
        if !self.sending.is_empty() {
            return self.toasts.push(format!("The last review is still on its way to {}", recipient.name()), Toast::Note);
        }
        let notes: Vec<Note> = self.diff().notes().into_iter().cloned().collect();
        if notes.is_empty() {
            let why = if self.diff().orphans().is_empty() {
                "No comment to send: c comments the line under the cursor"
            } else {
                "No comment to send: the lines yours were on are no longer in the diff"
            };
            return self.toasts.push(why, Toast::Note);
        }
        let repo = self.repo.clone().unwrap_or_default();
        if recipient == Recipient::New && !self.confirm_new(&repo) {
            return;
        }
        let comments: Vec<Value> = notes.iter().map(Note::value).collect();
        let mut review = json!({"scope": self.scope.id(), "repo": repo, "comments": comments});
        self.serial += 1;
        let tag = format!("send-{}", self.serial);
        match recipient {
            Recipient::Gone(agent) => self.toasts.push(format!("{} is no longer running here: T picks who gets the review", agent.name), Toast::Bad),
            Recipient::Starting => self.toasts.push("The review agent is not running yet: T picks who gets the review", Toast::Note),
            // The last poll's word, at once; `deliver` checks again as it sends.
            Recipient::Agent(agent) if agent.waiting => self.toasts.push(cockpit_core::deliver::waiting_note(&agent.name), Toast::Bad),
            Recipient::Agent(agent) => {
                review["to"] = json!(agent.name);
                self.sending.insert(tag.clone(), notes);
                self.pin();
                self.call(&tag, "send", vec![json!(agent.terminal), review]);
            }
            Recipient::New => {
                let Ok(prompt) = review::message(&review) else { return };
                // A space of its own, and Start work's own choices of repo, agent and mode left as they were.
                let label = format!("review-{}", cockpit_core::seen::now_ms());
                let request = json!({
                    "id": label, "source": "prompt", "text": prompt, "repo": repo, "worktree": "here", "dirty": true,
                    "label": label, "remember": false,
                });
                self.sending.insert(tag.clone(), notes);
                self.starting.insert(tag.clone(), (repo.clone(), label));
                self.pin();
                self.call_app("start", &tag, "work", vec![request]);
            }
        }
    }

    /// The first new agent a worktree's review starts asks first: a second S soon after starts it.
    fn confirm_new(&mut self, repo: &str) -> bool {
        if self.new_ok.contains(repo) {
            return true;
        }
        if self.armed_new.as_ref().is_some_and(|(r, at)| r == repo && at.elapsed() < CONFIRM) {
            self.armed_new = None;
            self.new_ok.insert(repo.to_string());
            return true;
        }
        self.armed_new = Some((repo.to_string(), Instant::now()));
        let name = repo.rsplit('/').next().unwrap_or(repo);
        self.toasts.push(format!("This starts a new agent in {name} on your comments: S again starts it"), Toast::Note);
        false
    }

    /// Runs a command.
    pub fn run(&mut self, cmd: Cmd) {
        self.dirty = true;
        let repo = self.repo.clone().unwrap_or_default();
        match cmd {
            Cmd::Send => self.send(),
            Cmd::Scope(scope) => self.set_scope(scope),
            Cmd::Walk => {
                self.mode = if self.mode == Mode::Walk { Mode::Diff } else { Mode::Walk };
                self.apply_step();
                self.scroll = 0;
            }
            Cmd::Switch => self.open_picker(),
            Cmd::ToPicker => self.open_recipients(),
            Cmd::To(to) => {
                let name = match &to {
                    To::Agent(a) => a.name.clone(),
                    To::New => "a new agent, started on the review".into(),
                    To::Started(_) => "the review agent".into(),
                };
                self.toasts.push(format!("comments on {} go to {name}", repo.rsplit('/').next().unwrap_or(&repo)), Toast::Note);
                self.to.insert(repo, to);
            }
            Cmd::Pick(terminal) => {
                if let Some(agent) = self.agents.iter().flatten().find(|a| a.terminal == terminal).cloned() {
                    self.toasts.push(format!("comments go to {}", agent.name), Toast::Note);
                    self.to.insert(agent.repo.clone(), To::Agent(agent.clone()));
                    (self.chosen, self.missing) = (Some(agent), 0);
                }
                // The person picked it: no "now reviewing" on top of it.
                self.follow_subject(None);
            }
            Cmd::Refresh => {
                review::forget(&repo);
                if let Some(poll) = &self.poll {
                    let _ = poll.send(Poll::Again);
                }
                self.refresh();
            }
            Cmd::Write => {
                if self.writing.is_some_and(|at| at.elapsed() < WRITE_STARTS) {
                    return;
                }
                self.writing = Some(Instant::now());
                let model = walk::MODELS[self.model].0;
                self.call("walkthrough", "walkthrough", vec![json!(repo), json!(self.scope.id()), json!(model)]);
            }
            Cmd::Stop => self.call("stop", "stop_walkthrough", vec![json!(repo), json!(self.scope.id())]),
            Cmd::Model => self.model = (self.model + 1) % walk::MODELS.len(),
            Cmd::Step(by) => {
                let count = walk::steps(&self.here().walkthrough).len();
                let here = self.here_mut();
                here.step = here.step.saturating_add_signed(by).min(count.saturating_sub(1));
                self.apply_step();
                self.here_mut().diff.to_first_line();
                (self.scroll, self.reveal) = (0, false);
            }
            Cmd::Copy => match walk::markdown(&self.here().walkthrough) {
                Some(text) if cockpit_core::files::copy(&text) => self.toasts.push("Copied the walkthrough as Markdown", Toast::Good),
                Some(_) => self.toasts.push("No clipboard tool answered (pbcopy, wl-copy or xclip)", Toast::Bad),
                None => {}
            },
            Cmd::OpenPr => {
                if let Some(url) = self.here().about["pr"]["url"].as_str().map(str::to_string) {
                    cockpit_core::files::open_url(&url);
                    self.toasts.push(format!("opened {url}"), Toast::Note);
                }
            }
            Cmd::Clear => {
                let n = self.here_mut().diff.clear();
                self.toasts.push(format!("{n} comment{} cleared · u brings them back", if n == 1 { "" } else { "s" }), Toast::Note);
            }
            Cmd::Undo => {
                let n = self.here_mut().diff.undo();
                if n > 0 {
                    self.toasts.push(format!("{n} comment{} back", if n == 1 { "" } else { "s" }), Toast::Note);
                }
            }
            Cmd::Comment | Cmd::Edit | Cmd::Delete | Cmd::SideBySide | Cmd::Hunk(_) | Cmd::FoldAll => self.on_diff(cmd),
        }
    }

    fn on_diff(&mut self, cmd: Cmd) {
        self.pin();
        let diff = &mut self.here_mut().diff;
        match cmd {
            Cmd::Comment => diff.comment_here(),
            Cmd::Edit => _ = diff.edit_here(),
            Cmd::Delete => {
                if diff.delete_here() {
                    self.toasts.push("comment deleted · u brings it back", Toast::Note);
                }
            }
            Cmd::SideBySide => diff.toggle_side_by_side(),
            Cmd::Hunk(forward) => _ = diff.hunk(forward),
            Cmd::FoldAll => diff.fold_all(),
            _ => {}
        }
        self.reveal = true;
    }

    pub fn set_scope(&mut self, scope: Scope) {
        if scope == self.scope {
            return;
        }
        (self.scope, self.scroll, self.reveal, self.dirty) = (scope, 0, true, true);
        self.apply_step();
        self.refresh();
    }

    /// Opens the picker on every agent working in a worktree.
    pub fn open_picker(&mut self) {
        // The space follows the TUI's own agents; the desktop's still get its reviews.
        let agents: Vec<Agent> = self.agents.clone().unwrap_or_default().into_iter().filter(|a| a.tui).collect();
        if agents.is_empty() {
            return self.toasts.push("No agent in a herdr pane works in a git worktree yet", Toast::Note);
        }
        let current = self.subject().map(|a| a.terminal.clone());
        let selected = agents.iter().position(|a| Some(&a.terminal) == current.as_ref()).unwrap_or(0);
        let options = agents.iter().map(|a| cockpit_core::text::fit(&a.label(), PICKED).trim_end().to_string()).collect();
        let question = "Whose worktree to review? Comments go to that agent.".to_string();
        self.dialog = Some((Dialog::Pick { question, options, selected }, Ask::Pick(agents.into_iter().map(|a| a.terminal).collect())));
    }

    /// Opens the recipient picker: the worktree's agents, driven sessions among them, then a new agent.
    pub fn open_recipients(&mut self) {
        let agents: Vec<Agent> = self.on_worktree().into_iter().cloned().collect();
        let mut options: Vec<String> = agents.iter().map(|a| cockpit_core::text::fit(&a.recipient(), PICKED).trim_end().to_string()).collect();
        options.push("A new agent, started on the review".into());
        let mut targets: Vec<To> = agents.into_iter().map(To::Agent).collect();
        targets.push(To::New);
        let selected = match self.recipient() {
            Some(Recipient::Agent(a)) => targets.iter().position(|t| matches!(t, To::Agent(b) if b.terminal == a.terminal)).unwrap_or(0),
            Some(Recipient::New | Recipient::Starting) => targets.len() - 1,
            _ => 0,
        };
        let question = "Who gets the comments on this worktree?".to_string();
        self.dialog = Some((Dialog::Pick { question, options, selected }, Ask::To(targets)));
    }

    /// Asks before leaving with comments not sent; true when there are none and the space can go.
    pub fn may_leave(&mut self) -> bool {
        let n = self.unsent();
        if n == 0 {
            return true;
        }
        let question = format!("Leave with {n} comment{} not sent? They are lost.", if n == 1 { "" } else { "s" });
        self.dialog = Some((Dialog::Confirm { question }, Ask::Leave));
        false
    }
}

fn local_time(ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms as i64).single().map(|t| t.format("%H:%M:%S").to_string()).unwrap_or_default()
}

/// The agents working in a git worktree, from the hub's agents view on a thread of their own: every few seconds while
/// the space is in view, every half minute while it is not, at once when told. Each folder's repository is asked of
/// git once, again after R, and forgotten when no agent works in it.
fn poll_agents() -> (Sender<Poll>, Receiver<Result<Vec<Agent>, String>>) {
    let (tell, told) = channel();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let toplevels: std::cell::RefCell<HashMap<String, Option<String>>> = Default::default();
        let toplevel = |cwd: &str| toplevels.borrow_mut().entry(cwd.to_string()).or_insert_with(|| review::toplevel(cwd)).clone();
        let mut every = AGENTS_EVERY;
        loop {
            // Every agent, both halves' terminals and the sessions cockpit drives: any of them can take the review.
            let request = json!({"op": "view", "name": "agents", "args": {}});
            let found = cockpit_core::hub::request(&request, Duration::from_secs(5)).map(|answer| {
                let listed: HashSet<&str> = answer["value"]["agents"].as_array().into_iter().flatten().filter_map(|a| a["cwd"].as_str()).collect();
                toplevels.borrow_mut().retain(|cwd, _| listed.contains(cwd.as_str()));
                subject::agents(&answer["value"], &toplevel)
            });
            if tx.send(found).is_err() {
                return;
            }
            cockpit_core::term::wake();
            match told.recv_timeout(every) {
                Ok(Poll::Focus(on)) => every = if on { AGENTS_EVERY } else { AWAY_EVERY },
                Ok(Poll::Again) => toplevels.borrow_mut().clear(),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    (tell, rx)
}
