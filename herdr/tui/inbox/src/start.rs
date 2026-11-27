//! Start work in the terminal: the form that turns a prompt, a ticket or a PR into a worktree, a space and an agent.
//! The hub plans it again as you type and the form shows the steps and what blocks them before Enter; then each step's
//! progress, with Retry when one fails, Esc to leave it going, and `c` to cancel, which asks before removing anything.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use cockpit_core::bridge::Bridge;
use cockpit_core::dialog::{Answer, Dialog};
use cockpit_core::input::{Edit, Input};
use cockpit_core::term;
use cockpit_core::text::wrap;
use cockpit_core::theme::{BG, DIM, FG, GREEN, LINE, ORANGE, RED, SURFACE, bold, dim, fg};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph};
use serde_json::{Map, Value, json};

const SOURCE: usize = 0;
const REPO: usize = 1;
const WORKTREE: usize = 2;
const BASE: usize = 3;
const BRANCH: usize = 4;
const AGENT: usize = 5;
const ACCOUNT: usize = 6;
const MODEL: usize = 7;
const EFFORT: usize = 8;
const FAST: usize = 9;
const MODE: usize = 10;
const FIELDS: usize = 11;
const LABELS: [&str; FIELDS] = ["Source", "Repo", "Worktree", "Base", "Branch", "Agent", "Account", "Model", "Effort", "Speed", "Mode"];
/// How long the form waits after the last key before asking the hub for a new plan.
const SETTLE: Duration = Duration::from_millis(250);
const SPIN: [&str; 4] = ["◐", "◓", "◑", "◒"];

/// Both ends of the channel the hub's plans come back on, each with the request it answers.
type Plans = (Sender<(u64, Value)>, Receiver<(u64, Value)>);

/// What a key did to the form.
pub enum Outcome {
    Stay,
    /// The form is done; the note says how it went.
    Close(Option<String>),
}

pub struct Form {
    /// False for fixtures: nothing is asked of the hub.
    live: bool,
    id: String,
    source: Input,
    base: Input,
    branch: Input,
    model: Input,
    /// What the person picked; None leaves it to the plan.
    repo: Option<String>,
    agent: Option<String>,
    /// The config folder of the account picked, of the agent's `accounts`.
    account: Option<String>,
    mode: Option<String>,
    /// The effort level picked; empty leaves it to the agent.
    effort: String,
    /// Fast mode picked: "on", "off", or empty to leave it to the agent.
    fast: String,
    here: bool,
    /// What the blockers' fixes and the options set: source, existing, dirty, trust, setup, confirm, path, forkSetup.
    extra: Map<String, Value>,
    /// The field in focus, then the options and fixes after them.
    at: usize,
    pub plan: Value,
    asked: u64,
    /// A plan was asked for and has not come back.
    pending: bool,
    changed: Option<Instant>,
    /// When a model catalog still being read is checked again.
    catalog_due: Option<Instant>,
    plans: Plans,
    /// The run once Enter started it, as the `start` view shows it.
    pub run: Option<Value>,
    runs: (Sender<Value>, Receiver<Value>),
    polled: Instant,
    bridge: Bridge,
    /// The act's own reply: its note, or its error.
    reply: Option<Result<String, String>>,
    /// Cancel asks first, since it removes what the start made.
    confirm: Option<Dialog>,
    help: bool,
    /// Lines scrolled past when the form is taller than the pane.
    scroll: usize,
    frame: usize,
}

impl Form {
    fn blank(source: &str, live: bool) -> Form {
        Form {
            live,
            id: format!("t{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)),
            source: Input::new(source),
            base: Input::default(),
            branch: Input::default(),
            model: Input::default(),
            repo: None,
            agent: None,
            account: None,
            mode: None,
            effort: String::new(),
            fast: String::new(),
            here: false,
            extra: Map::new(),
            at: SOURCE,
            plan: Value::Null,
            asked: 0,
            pending: false,
            changed: Some(Instant::now() - SETTLE),
            catalog_due: None,
            plans: channel(),
            run: None,
            runs: channel(),
            polled: Instant::now() - Duration::from_secs(1),
            bridge: Bridge::default(),
            reply: None,
            confirm: None,
            help: false,
            scroll: 0,
            frame: 0,
        }
    }

    /// A form with the source given (a ticket's key, a PR's URL, or nothing), asking the hub for its plan.
    pub fn new(source: &str, live: bool) -> Form {
        let mut form = Form::blank(source, live);
        form.pump();
        form
    }

    /// The form on a start that runs or failed already, as the inbox's row opens it: its steps, Retry and Cancel.
    pub fn attach(id: &str, live: bool) -> Form {
        let mut form = Form::blank("", live);
        form.id = id.to_string();
        form.changed = None;
        form.run = Some(json!({"state": "running", "steps": []}));
        form.pump();
        form
    }

    /// The form on a plan and a run given, for tests: nothing is fetched.
    pub fn fixture(source: &str, plan: Value, run: Option<Value>) -> Form {
        let mut form = Form::blank(source, false);
        form.plan = plan;
        form.changed = None;
        form.run = run;
        form
    }

    /// The request as the hub's `start_work` takes it.
    pub fn request(&self) -> Value {
        let mut req = json!({
            "id": self.id, "text": self.source.text.trim(), "worktree": if self.here { "here" } else { "new" },
            "base": self.base.text.trim(), "branch": self.branch.text.trim(), "model": self.model.text.trim(),
            "repo": self.repo.clone().unwrap_or_default(), "agent": self.agent.clone().unwrap_or_default(), "mode": self.mode.clone().unwrap_or_default(),
            "effort": self.effort, "account": self.account.clone().unwrap_or_default(),
        });
        // Fast mode goes only once picked.
        match self.fast.as_str() {
            "on" => req["fast"] = json!(true),
            "off" => req["fast"] = json!(false),
            _ => {}
        }
        for (k, v) in &self.extra {
            req[k] = v.clone();
        }
        req
    }

    fn started(&self) -> bool {
        self.run.is_some() || self.reply.is_some()
    }

    fn state(&self) -> &str {
        self.run.as_ref().and_then(|r| r["state"].as_str()).unwrap_or(if self.started() { "running" } else { "" })
    }

    /// The plan's options, drawn under the source; a source already open is brought into view and offers none.
    fn options(&self) -> Vec<Value> {
        if self.plan["place"] == "focus" { Vec::new() } else { self.plan["options"].as_array().cloned().unwrap_or_default() }
    }

    /// The options, then the blockers' fixes, each as (label, what it sets): in the order drawn, which 1–9 pick, as the
    /// Herdr app's dialog does.
    fn choices(&self) -> Vec<(String, Value)> {
        let fixes = self.plan["blockers"].as_array().into_iter().flatten().flat_map(|b| b["fixes"].as_array().cloned().unwrap_or_default());
        self.options().into_iter().chain(fixes).map(|f| (f["label"].as_str().unwrap_or_default().to_string(), f["set"].clone())).collect()
    }

    /// The agent the form starts: the one picked, else the plan's.
    fn agent_choice(&self) -> Value {
        let id = self.agent.clone().or_else(|| self.plan["agent"].as_str().map(str::to_string)).unwrap_or_default();
        self.plan["choices"]["agents"].as_array().into_iter().flatten().find(|a| a["id"] == id.as_str()).cloned().unwrap_or(Value::Null)
    }

    /// The accounts the agent can start on, when this machine has several: `{dir, name, default}`, the default first.
    fn accounts(&self) -> Vec<Value> {
        self.agent_choice()["accounts"].as_array().cloned().unwrap_or_default()
    }

    /// Whether the field shows: base and branch for a new worktree of anything but a PR, the account on a machine with
    /// several, the model, effort level, fast mode and mode for an agent that takes them at start, and the source alone
    /// when the start runs already.
    fn shown(&self, field: usize) -> bool {
        if self.plan["place"] == "focus" {
            return field == SOURCE;
        }
        match field {
            BASE | BRANCH => !self.here && self.plan["kind"] != "pr",
            ACCOUNT => self.accounts().len() > 1,
            MODEL => self.agent_choice()["models"] != false,
            EFFORT => self.offered(EFFORT).len() > 1,
            FAST => self.offered(FAST).len() > 1,
            MODE => self.agent_choice()["modes"].as_array().is_some_and(|m| !m.is_empty()),
            _ => true,
        }
    }

    /// The model is typed for an agent that lists none.
    fn is_text(&self, at: usize) -> bool {
        matches!(at, SOURCE | BASE | BRANCH) || (at == MODEL && self.offered(MODEL).is_empty())
    }

    /// What the agent takes at start for a model, an effort level or fast mode, as (value, label), its own default ("")
    /// first; empty where it takes none. The model picked counts: its own levels and fast mode, where its catalog says.
    fn offered(&self, field: usize) -> Vec<(String, String)> {
        let picks = self.agent_choice()["picks"].clone();
        let start = |k: &str| picks["start"][k] == true;
        let models: Vec<Value> = if start("model") { picks["models"].as_array().cloned().unwrap_or_default() } else { vec![] };
        let model = self.model.text.trim();
        let offer = models.iter().find(|m| m["id"] == model);
        let pair = |v: &str, l: &str| (v.to_string(), l.to_string());
        let list: Vec<(String, String)> = match field {
            MODEL => models.iter().filter_map(|m| Some(pair(m["id"].as_str()?, m["label"].as_str().or(m["id"].as_str())?))).collect(),
            EFFORT if start("effort") => offer
                .and_then(|o| o["efforts"].as_array())
                .or(picks["efforts"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|l| l.as_str().map(|l| pair(l, &effort_name(l))))
                .collect(),
            FAST if start("fast") && offer.is_none_or(|o| o["fast"] != false) => vec![pair("on", "fast"), pair("off", "standard")],
            _ => vec![],
        };
        if list.is_empty() {
            return list;
        }
        std::iter::once(pair("", "its default")).chain(list).collect()
    }

    /// A model, an effort level or fast mode the agent or the model picked now does not take goes back to its default.
    fn keep_picks(&mut self) {
        let has = |list: Vec<(String, String)>, v: &str| v.is_empty() || list.iter().any(|(x, _)| x == v);
        if !self.offered(MODEL).is_empty() && !has(self.offered(MODEL), self.model.text.trim()) {
            self.model = Input::default();
        }
        if !has(self.offered(EFFORT), &self.effort) {
            self.effort.clear();
        }
        if !has(self.offered(FAST), &self.fast) {
            self.fast.clear();
        }
    }

    fn stops(&self) -> Vec<usize> {
        (0..FIELDS).filter(|f| self.shown(*f)).chain((0..self.choices().len()).map(|n| FIELDS + n)).collect()
    }

    fn step_focus(&mut self, by: isize) {
        let stops = self.stops();
        let here = stops.iter().position(|s| *s == self.at).unwrap_or(0) as isize;
        let n = stops.len() as isize;
        self.at = stops[((here + by) % n + n) as usize % n as usize];
    }

    /// Something the plan reads changed: the hub is asked again once typing settles.
    fn edited(&mut self) {
        self.changed = Some(Instant::now());
    }

    /// The source or the repo changed: the fixes picked for the last plan are dropped with it.
    fn reset(&mut self) {
        self.extra.clear();
        self.edited();
    }

    fn cycle(&mut self, by: isize) {
        let choices = self.plan["choices"].clone();
        let pick = |list: Vec<String>, now: &str| -> Option<String> {
            if list.is_empty() {
                return None;
            }
            let at = list.iter().position(|x| x == now).map_or(0, |i| (i as isize + by).rem_euclid(list.len() as isize) as usize);
            Some(list[at].clone())
        };
        let strings = |v: &Value, key: &str| -> Vec<String> {
            v.as_array().into_iter().flatten().filter_map(|x| x[key].as_str().or(x.as_str()).map(str::to_string)).collect()
        };
        match self.at {
            REPO => {
                let now = self.repo.clone().unwrap_or_else(|| self.plan["repo"].as_str().unwrap_or_default().to_string());
                self.repo = pick(strings(&choices["repos"], ""), &now);
                self.reset();
            }
            WORKTREE => {
                self.here = !self.here;
                self.extra.remove("dirty");
                self.edited();
            }
            AGENT => {
                let now = self.agent.clone().unwrap_or_else(|| self.plan["agent"].as_str().unwrap_or_default().to_string());
                self.agent = pick(strings(&choices["agents"], "id"), &now);
                // Another agent starts from its own defaults.
                (self.model, self.effort, self.fast) = (Input::default(), String::new(), String::new());
                self.edited();
            }
            ACCOUNT => {
                let now = self.account.clone().unwrap_or_else(|| self.plan["account"].as_str().unwrap_or_default().to_string());
                self.account = pick(strings(&Value::Array(self.accounts()), "dir"), &now);
                self.edited();
            }
            MODEL | EFFORT | FAST => {
                let values: Vec<String> = self.offered(self.at).into_iter().map(|(v, _)| v).collect();
                let now = match self.at {
                    MODEL => self.model.text.trim().to_string(),
                    EFFORT => self.effort.clone(),
                    _ => self.fast.clone(),
                };
                let next = pick(values, &now).unwrap_or_default();
                match self.at {
                    MODEL => self.model = Input::new(&next),
                    EFFORT => self.effort = next,
                    _ => self.fast = next,
                }
                self.keep_picks();
                self.edited();
            }
            MODE => {
                let now = self.mode.clone().unwrap_or_else(|| self.plan["mode"].as_str().unwrap_or("default").to_string());
                self.mode = pick(strings(&self.agent_choice()["modes"], "id"), &now);
                self.edited();
            }
            _ => {}
        }
    }

    fn apply(&mut self, set: &Value) {
        for (k, v) in set.as_object().into_iter().flatten() {
            match k.as_str() {
                "branch" => self.branch = Input::new(v.as_str().unwrap_or_default()),
                "base" => self.base = Input::new(v.as_str().unwrap_or_default()),
                "repo" => self.repo = v.as_str().map(str::to_string),
                "worktree" => self.here = v == "here",
                "account" => self.account = v.as_str().filter(|a| !a.is_empty()).map(str::to_string),
                _ => {
                    self.extra.insert(k.clone(), v.clone());
                }
            }
        }
        self.at = SOURCE;
        self.edited();
    }

    /// Whether Enter may start: a plan that answers the form as it is now, with nothing blocking.
    fn ready(&self) -> bool {
        self.plan["ok"] == true && self.changed.is_none() && !self.pending
    }

    fn begin(&mut self) {
        self.catalog_due = None;
        self.run = Some(json!({"state": "running", "steps": self.plan["steps"], "label": self.plan["label"]}));
        if self.live {
            self.bridge.call("work", "start", "work", vec![self.request()], None);
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if let Some(dialog) = &mut self.confirm {
            match dialog.key(key) {
                Some(Answer::Yes) => {
                    self.confirm = None;
                    if self.live {
                        self.bridge.call("cancel", "start", "cancel", vec![json!(self.id)], None);
                    }
                    return Outcome::Close(Some("cancelling: what the start made is removed".into()));
                }
                Some(_) => self.confirm = None,
                None => {}
            }
            return Outcome::Stay;
        }
        if self.started() {
            return self.key_running(key);
        }
        let shift_tab = key.code == KeyCode::BackTab || (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT));
        let text = self.is_text(self.at);
        match key.code {
            KeyCode::Esc => return Outcome::Close(None),
            _ if shift_tab => self.step_focus(-1),
            KeyCode::Up => self.step_focus(-1),
            KeyCode::Tab | KeyCode::Down => self.step_focus(1),
            KeyCode::PageDown => self.scroll += 5,
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(5),
            KeyCode::Enter if self.at >= FIELDS => {
                if let Some((_, set)) = self.choices().get(self.at - FIELDS).cloned() {
                    self.apply(&set);
                }
            }
            KeyCode::Enter if self.ready() => self.begin(),
            KeyCode::Enter => {
                let blocked = self.plan["ok"] == false && self.changed.is_none() && !self.pending;
                let choices = self.choices().len();
                if blocked && choices > 0 {
                    // To the first fix, else the first option.
                    let options = self.options().len();
                    self.at = FIELDS + if choices > options { options } else { 0 };
                }
            }
            KeyCode::Char(c @ '1'..='9') if !text => {
                if let Some((_, set)) = self.choices().get(c as usize - '1' as usize).cloned() {
                    self.apply(&set);
                }
            }
            KeyCode::Char('?') if !text => self.help = !self.help,
            KeyCode::Left if !text => self.cycle(-1),
            KeyCode::Right | KeyCode::Char(' ') if !text => self.cycle(1),
            _ if text => {
                let input = match self.at {
                    SOURCE => &mut self.source,
                    BASE => &mut self.base,
                    BRANCH => &mut self.branch,
                    _ => &mut self.model,
                };
                if let Edit::Changed = input.key(key) {
                    if self.at == SOURCE {
                        self.reset();
                    } else {
                        self.edited();
                    }
                }
            }
            _ => {}
        }
        Outcome::Stay
    }

    fn key_running(&mut self, key: KeyEvent) -> Outcome {
        let state = self.state().to_string();
        let removable = self.run.as_ref().is_some_and(|r| r["cancel"] == true);
        match (key.code, state.as_str()) {
            (KeyCode::Char('c'), "running" | "failed") if removable => {
                let question = "Remove what this start made? Its space closes, and the worktree and branch it made go.".to_string();
                self.confirm = Some(Dialog::Confirm { question });
            }
            (KeyCode::Char('r'), "failed") => {
                self.reply = None;
                if self.live {
                    self.bridge.call("retry", "start", "retry", vec![json!(self.id)], None);
                }
                if let Some(run) = self.run.as_mut() {
                    run["state"] = json!("running");
                    run["error"] = Value::Null;
                }
            }
            (KeyCode::PageDown, _) => self.scroll += 5,
            (KeyCode::PageUp, _) => self.scroll = self.scroll.saturating_sub(5),
            (KeyCode::Esc | KeyCode::Char('h'), _) | (KeyCode::Enter, "done" | "cancelled") => {
                let note = match (&self.reply, state.as_str()) {
                    (Some(Ok(note)), _) if !note.is_empty() => Some(note.clone()),
                    (_, "running") => Some("the start goes on: the inbox shows it until it ends".into()),
                    (_, "failed") => Some("kept as it is: the inbox shows the start, to retry or cancel".into()),
                    _ => None,
                };
                return Outcome::Close(note);
            }
            _ => {}
        }
        Outcome::Stay
    }

    /// The hub's answers, whichever came in: a new plan, the run's progress, the act's reply. True when any did.
    pub fn pump(&mut self) -> bool {
        let mut moved = false;
        if !self.live {
            return false;
        }
        while let Ok((n, plan)) = self.plans.1.try_recv() {
            if n == self.asked {
                self.plan = plan;
                self.catalog_due = self.plan["choices"]["agents"]
                    .as_array()
                    .is_some_and(|agents| agents.iter().any(|a| a["picks"]["loading"] == true))
                    .then(|| Instant::now() + Duration::from_millis(500));
                self.pending = false;
                if !self.stops().contains(&self.at) {
                    self.at = SOURCE;
                }
                moved = true;
            }
        }
        self.refresh_catalog(Instant::now());
        if self.changed.is_some_and(|at| at.elapsed() >= SETTLE) && !self.started() {
            self.changed = None;
            self.pending = true;
            self.asked += 1;
            let (n, tx, request) = (self.asked, self.plans.0.clone(), self.request());
            std::thread::spawn(move || {
                let plan = cockpit_core::hub::fetch("start_work", json!({"request": request})).unwrap_or_else(|e| json!({"error": e}));
                let _ = tx.send((n, plan));
                term::wake();
            });
        }
        while let Ok(reply) = self.bridge.replies.try_recv() {
            if reply.tag == "work" || reply.tag == "retry" {
                self.feed_reply(match reply.error {
                    Some(error) => Err(error),
                    None => Ok(reply.note),
                });
            }
            moved = true;
        }
        while let Ok(run) = self.runs.1.try_recv() {
            self.feed_run(run);
            moved = true;
        }
        let over = matches!(self.state(), "done" | "cancelled") && self.reply.is_some();
        if self.started() && !over && self.polled.elapsed() >= Duration::from_millis(300) {
            self.polled = Instant::now();
            let (tx, id) = (self.runs.0.clone(), self.id.clone());
            std::thread::spawn(move || {
                let asked = json!({"op": "view", "name": "start", "args": {"id": id}});
                let run = cockpit_core::hub::request(&asked, Duration::from_secs(2)).map(|v| v["value"]["run"].clone()).unwrap_or(Value::Null);
                let _ = tx.send(run);
                term::wake();
            });
            self.frame += 1;
            moved = true;
        }
        moved
    }

    /// A catalog still being read schedules another plan without changing what the person picked.
    fn refresh_catalog(&mut self, now: Instant) {
        if self.changed.is_none() && !self.pending && !self.started() && self.catalog_due.is_some_and(|due| now >= due) {
            self.catalog_due = None;
            self.changed = Some(now - SETTLE);
        }
    }

    /// The act's reply to Start or Retry: its note, or its error.
    pub fn feed_reply(&mut self, reply: Result<String, String>) {
        self.reply = Some(reply);
    }

    /// The run as the hub's `start` view has it; none at all after an error means the start failed before it began.
    pub fn feed_run(&mut self, run: Value) {
        match (run.is_null(), &self.reply) {
            (false, _) => self.run = Some(run),
            (true, Some(Err(error))) => {
                let steps = self.run.as_ref().map(|r| r["steps"].clone()).unwrap_or(Value::Null);
                self.run = Some(json!({"state": "failed", "steps": steps, "error": {"step": "start", "text": error}, "retry": false, "cancel": false}));
            }
            _ => {}
        }
    }

    /// Whether it waits on the hub, for the app to tick faster.
    pub fn busy(&self) -> bool {
        self.changed.is_some() || self.catalog_due.is_some() || self.pending || (self.started() && !matches!(self.state(), "done" | "failed" | "cancelled"))
    }

    /// The run finished well and its note was given: the form can go.
    pub fn finished(&self) -> Option<String> {
        match (&self.reply, self.state()) {
            (Some(Ok(note)), "done") => Some(note.clone()),
            _ => None,
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Drawing

    fn value_of(&self, field: usize) -> (String, Style) {
        let plan = &self.plan;
        let (text, given) = match field {
            REPO => {
                (self.repo.clone().or_else(|| plan["repo"].as_str().filter(|r| !r.is_empty()).map(str::to_string)).unwrap_or_else(|| "pick one".into()), true)
            }
            WORKTREE => ((if self.here { "here, in the checkout" } else { "a new worktree" }).to_string(), true),
            AGENT => {
                let found = self.agent_choice();
                let name = found["name"].as_str().or(plan["agentName"].as_str()).unwrap_or_default().to_string();
                (if found["installed"] == false { format!("{name} (not installed)") } else { name }, true)
            }
            ACCOUNT => {
                let dir = self.account.clone().or_else(|| plan["account"].as_str().map(str::to_string)).unwrap_or_default();
                let list = self.accounts();
                let found = list.iter().find(|a| a["dir"] == dir.as_str()).or_else(|| list.iter().find(|a| a["default"] == true));
                let name = found.and_then(|a| a["name"].as_str()).unwrap_or_default();
                let default = found.is_some_and(|a| a["default"] == true);
                (if default { format!("{name} (default)") } else { name.to_string() }, self.account.is_some())
            }
            MODE => {
                let id = self.mode.clone().or_else(|| plan["mode"].as_str().filter(|m| !m.is_empty()).map(str::to_string)).unwrap_or_else(|| "default".into());
                let agent = self.agent_choice();
                let label =
                    agent["modes"].as_array().into_iter().flatten().find(|m| m["id"] == id.as_str()).and_then(|m| m["label"].as_str()).unwrap_or("Asks first");
                (label.to_string(), true)
            }
            BASE if self.base.text.is_empty() => (plan["base"].as_str().filter(|b| !b.is_empty()).unwrap_or("its default").to_string(), false),
            BRANCH if self.branch.text.is_empty() => (plan["branch"].as_str().unwrap_or_default().to_string(), false),
            MODEL if self.model.text.is_empty() => ("its default".into(), false),
            MODEL if !self.offered(MODEL).is_empty() => {
                let id = self.model.text.trim();
                (self.offered(MODEL).into_iter().find(|(v, _)| v == id).map(|(_, l)| l).unwrap_or_else(|| id.to_string()), true)
            }
            EFFORT if self.effort.is_empty() => ("its default".into(), false),
            EFFORT => (effort_name(&self.effort), true),
            FAST if self.fast.is_empty() => ("its default".into(), false),
            FAST => ((if self.fast == "on" { "fast" } else { "standard" }).to_string(), true),
            SOURCE if self.source.text.is_empty() => ("a prompt, a ticket key or a PR's URL".into(), false),
            SOURCE => (self.source.text.clone(), true),
            BASE => (self.base.text.clone(), true),
            BRANCH => (self.branch.text.clone(), true),
            _ => (self.model.text.clone(), true),
        };
        (text, if given { fg(FG) } else { dim() })
    }

    fn field_line(&self, field: usize, label_w: usize, inner: usize) -> Line<'static> {
        let focused = self.at == field && !self.started();
        let label = format!("{:<label_w$}", LABELS[field]);
        if focused && self.is_text(field) {
            let input = match field {
                SOURCE => &self.source,
                BASE => &self.base,
                BRANCH => &self.branch,
                _ => &self.model,
            };
            return input.line(&label);
        }
        let (text, style) = self.value_of(field);
        let room = inner.saturating_sub(label_w + 4);
        let text = cockpit_core::text::fit(&text, room).trim_end().to_string();
        let label_style = if focused { fg(ORANGE) } else { dim() };
        if self.is_text(field) {
            return Line::from(vec![Span::styled(label, label_style), Span::styled(text, style)]);
        }
        let arrows = if focused { fg(ORANGE) } else { fg(LINE) };
        Line::from(vec![Span::styled(label, label_style), Span::styled("‹ ", arrows), Span::styled(text, style), Span::styled(" ›", arrows)])
    }

    /// A row of pills for these choices, the one in focus lit, each with the digit that picks it; `first` is the stop of
    /// the first.
    fn pills(&self, choices: &[Value], first: usize, indent: usize) -> Line<'static> {
        let mut spans = vec![Span::raw(" ".repeat(indent))];
        for (n, f) in choices.iter().enumerate() {
            let on = self.at == first + n;
            let style = if on { Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD) } else { Style::new().fg(FG).bg(LINE) };
            let digit = first - FIELDS + n + 1;
            let digit = if digit <= 9 { format!("{digit} ") } else { String::new() };
            spans.push(Span::styled(format!(" {} ", f["label"].as_str().unwrap_or_default()), style));
            spans.push(Span::styled(digit, if on { style } else { Style::new().fg(DIM).bg(LINE) }));
            spans.push(Span::raw(" "));
        }
        Line::from(spans)
    }

    /// One plan or progress line per step, a blocker's fixes under it, then the notes.
    fn plan_lines(&self, inner: usize) -> Vec<Line<'static>> {
        let home = cockpit_core::files::home().to_string_lossy().into_owned();
        let short = |t: &str| if home.len() > 1 { t.replace(&home, "~") } else { t.to_string() };
        let mut out = Vec::new();
        let push = |mark: Span<'static>, text: String, style: Style, out: &mut Vec<Line<'static>>| {
            for (n, part) in wrap(&text, inner.saturating_sub(2)).into_iter().enumerate() {
                let lead = if n == 0 { mark.clone() } else { Span::raw("  ") };
                out.push(Line::from(vec![lead, Span::styled(part, style)]));
            }
        };
        if let Some(error) = self.plan["error"].as_str().filter(|_| self.run.is_none()) {
            push(Span::styled("✗ ", fg(RED)), format!("The hub did not answer: {error}"), fg(RED), &mut out);
            return out;
        }
        let progress = self.run.as_ref().and_then(|r| r["steps"].as_array().cloned());
        let steps = progress.clone().or_else(|| self.plan["steps"].as_array().cloned()).unwrap_or_default();
        for s in &steps {
            let text = short(s["text"].as_str().unwrap_or_default());
            let (mark, style) = match (progress.is_some(), s["state"].as_str().unwrap_or("todo")) {
                (false, _) => (Span::styled("· ", dim()), fg(FG)),
                (true, "done") => (Span::styled("✓ ", fg(GREEN)), fg(FG)),
                (true, "running") => (Span::styled(format!("{} ", SPIN[self.frame % SPIN.len()]), fg(ORANGE)), fg(FG)),
                (true, "failed") => (Span::styled("✗ ", fg(RED)), fg(RED)),
                (true, "skipped") => (Span::styled("– ", dim()), dim()),
                (true, _) => (Span::styled("· ", dim()), dim()),
            };
            let note = s["note"].as_str().filter(|n| !n.is_empty()).map(|n| format!(": {}", short(n))).unwrap_or_default();
            push(mark, format!("{text}{note}"), style, &mut out);
        }
        if progress.is_some() {
            if let Some(error) = self.run.as_ref().and_then(|r| r["error"]["text"].as_str()) {
                push(Span::styled("✗ ", fg(RED)), short(error), fg(RED), &mut out);
            } else if let Some(Err(error)) = &self.reply {
                push(Span::styled("✗ ", fg(RED)), short(error), fg(RED), &mut out);
            }
            return out;
        }
        let mut stop = FIELDS + self.options().len();
        for b in self.plan["blockers"].as_array().into_iter().flatten() {
            let fixes = b["fixes"].as_array().cloned().unwrap_or_default();
            if b["step"] == "empty" {
                push(Span::styled("i ", fg(DIM)), short(b["text"].as_str().unwrap_or_default()), dim(), &mut out);
            } else {
                let step = b["step"].as_str().unwrap_or_default();
                push(Span::styled("✗ ", fg(RED)), format!("{step}  {}", short(b["text"].as_str().unwrap_or_default())), fg(RED), &mut out);
            }
            if !fixes.is_empty() {
                out.push(self.pills(&fixes, stop, 2));
                stop += fixes.len();
            }
        }
        for n in self.plan["notes"].as_array().into_iter().flatten() {
            push(Span::styled("i ", fg(DIM)), short(n.as_str().unwrap_or_default()), dim(), &mut out);
        }
        out
    }

    fn footer(&self) -> Line<'static> {
        let key = |k: &str| Span::styled(k.to_string(), fg(ORANGE));
        let word = |w: &str| Span::styled(w.to_string(), dim());
        let removable = self.run.as_ref().is_some_and(|r| r["cancel"] == true);
        match self.state() {
            "running" => {
                let mut spans = vec![word("starting…  "), key("esc"), word(" leave it going")];
                if removable {
                    spans.extend([word("  "), key("c"), word(" cancel")]);
                }
                Line::from(spans)
            }
            "failed" => {
                let mut spans = vec![Span::styled("▸ ", fg(ORANGE)), key("r"), word(" retry   "), key("esc"), word(" keep it")];
                if removable {
                    spans.extend([word("   "), key("c"), word(" cancel, removing what it made")]);
                }
                Line::from(spans)
            }
            "done" | "cancelled" => Line::from(vec![key("⏎"), word(" close")]),
            _ if self.plan.is_null() || self.changed.is_some() || self.pending => Line::from(vec![word("checking…   "), key("esc"), word(" cancel")]),
            _ if self.plan["ok"] == true && self.plan["place"] == "focus" => {
                Line::from(vec![Span::styled("▸ Bring it into view ", bold(ORANGE)), key("⏎"), word("   "), key("esc"), word(" cancel")])
            }
            _ if self.plan["ok"] == true => Line::from(vec![
                Span::styled("▸ Start ", bold(ORANGE)),
                key("⏎"),
                word("   "),
                key("tab"),
                word(" next  "),
                key("←→"),
                word(" change  "),
                key("esc"),
                word(" cancel"),
            ]),
            _ if self.choices().is_empty() => Line::from(vec![word("can't start yet: see what is marked ✗   "), key("esc"), word(" cancel")]),
            _ => Line::from(vec![word("can't start yet  "), key("⏎"), word(" to a fix  "), key("esc"), word(" cancel")]),
        }
    }

    /// The form's lines, and the line the focus is on, to keep it in view.
    fn lines(&self, inner: usize, label_w: usize) -> (Vec<Line<'static>>, usize) {
        let named =
            self.plan["title"].as_str().filter(|t| !t.is_empty()).or_else(|| self.run.as_ref().and_then(|r| r["title"].as_str()).filter(|t| !t.is_empty()));
        let title = match named.filter(|_| self.started()) {
            Some(t) => format!("Start work · {t}"),
            None => "Start work".into(),
        };
        let mut lines = vec![Line::styled(cockpit_core::text::fit(&title, inner).trim_end().to_string(), bold(ORANGE)), Line::raw("")];
        let mut focus = 0;
        let options = if self.started() { Vec::new() } else { self.options() };
        if !(self.started() && self.plan.is_null()) {
            for field in (0..FIELDS).filter(|f| self.shown(*f)) {
                if self.at == field {
                    focus = lines.len();
                }
                lines.push(self.field_line(field, label_w, inner));
                if field != SOURCE {
                    continue;
                }
                let kind = match self.plan["kind"].as_str().unwrap_or_default() {
                    "pr" => "PR",
                    other => other,
                };
                let resolved = self.plan["title"].as_str().filter(|t| !t.is_empty() && !self.source.text.trim().is_empty());
                if let Some(t) = resolved.filter(|_| kind != "prompt") {
                    let text = cockpit_core::text::fit(&format!("→ {kind} · {t}"), inner.saturating_sub(label_w)).trim_end().to_string();
                    lines.push(Line::from(vec![Span::raw(" ".repeat(label_w)), Span::styled(text, fg(GREEN))]));
                }
                if !options.is_empty() {
                    if (FIELDS..FIELDS + options.len()).contains(&self.at) {
                        focus = lines.len();
                    }
                    lines.push(self.pills(&options, FIELDS, label_w));
                }
            }
            lines.push(Line::raw(""));
        }
        let plan_start = lines.len();
        lines.extend(self.plan_lines(inner));
        if self.at >= FIELDS + options.len() {
            focus = (plan_start..lines.len()).find(|n| lines[*n].spans.iter().any(|s| s.style.bg == Some(ORANGE))).unwrap_or(plan_start);
        }
        if self.help {
            lines.push(Line::raw(""));
            for (k, what) in [
                ("tab ↓ ↑", "move between fields, options and fixes"),
                ("← → space", "change a choice"),
                ("⏎", "start, or apply the option or fix in focus"),
                ("1–9", "apply the option or fix of that number, away from a text field"),
                ("pgup pgdn", "scroll"),
                ("esc", "leave; a start already going goes on"),
                ("r  c", "retry a failed step, cancel a start (asked first)"),
            ] {
                lines.push(Line::from(vec![Span::styled(format!("{k:<12}"), fg(FG)), Span::styled(what, dim())]));
            }
        }
        (lines, focus)
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let width = 78.min(area.width.saturating_sub(2)).max(20.min(area.width));
        let inner = (width as usize).saturating_sub(6);
        let (lines, focus) = self.lines(inner, 10);
        let height = (lines.len() as u16 + 6).min(area.height);
        let box_ = Rect { x: area.x + (area.width - width) / 2, y: area.y + area.height.saturating_sub(height) / 2, width, height };
        frame.render_widget(Clear, box_);
        let block = Block::new().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(fg(ORANGE)).style(Style::new().bg(SURFACE));
        let body = block.padding(Padding::new(2, 2, 1, 1));
        let inside = body.inner(box_);
        frame.render_widget(body, box_);
        let room = inside.height.saturating_sub(2) as usize;
        // Taller than the pane: the focus stays in view, and PgUp/PgDn move past the rest.
        let most = lines.len().saturating_sub(room);
        let mut scroll = self.scroll.min(most);
        if focus >= scroll + room {
            scroll = focus + 1 - room;
        } else if focus < scroll && !self.started() {
            scroll = focus;
        }
        let text_area = Rect { height: room as u16, ..inside };
        let total = lines.len();
        frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), text_area);
        if inside.height >= 1 {
            let foot = Rect { y: inside.y + inside.height - 1, height: 1, ..inside };
            let mut footer = self.footer();
            if most > 0 {
                footer.spans.push(Span::styled(format!("   ↕ {}/{total}", scroll + room.min(total)), dim()));
            }
            frame.render_widget(Paragraph::new(footer), foot);
        }
        if let Some(dialog) = &self.confirm {
            dialog.render(frame, area);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The popup on its own

/// `cockpit start-work [source]`: the form alone, for the global chord; it quits when the form closes.
pub struct Popup {
    pub form: Form,
}

impl term::App for Popup {
    fn draw(&mut self, frame: &mut Frame) {
        frame.render_widget(Block::new().style(cockpit_core::theme::base()), frame.area());
        self.form.render(frame, frame.area());
    }

    fn event(&mut self, event: Event) -> bool {
        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
        {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return true;
            }
            if let Outcome::Close(_) = self.form.key(key) {
                return true;
            }
        }
        self.form.pump();
        false
    }

    fn tick(&mut self) -> bool {
        self.form.pump() || self.form.busy()
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(if self.form.busy() { 100 } else { 1000 })
    }
}

/// Runs the popup until the form closes.
pub fn run(source: &str) -> std::io::Result<()> {
    term::run(&mut Popup { form: Form::new(source, true) })
}

/// An effort level as the form says it: "xhigh" is "extra high".
fn effort_name(level: &str) -> String {
    match level {
        "xhigh" => "extra high".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_catalog_refreshes_without_an_edit() {
        let mut form = Form::blank("ENG-412", false);
        form.changed = None;
        let now = Instant::now();
        form.catalog_due = Some(now + Duration::from_millis(500));
        form.refresh_catalog(now);
        assert!(form.changed.is_none(), "the form waits between discovery checks");
        form.refresh_catalog(now + Duration::from_secs(1));
        assert!(form.changed.is_some() && form.catalog_due.is_none(), "one more plan is scheduled without changing the prompt");
        assert_eq!(form.request()["text"], "ENG-412");
        form.changed = None;
        form.refresh_catalog(now + Duration::from_secs(2));
        assert!(form.changed.is_none(), "no more checks once completion clears the due time");
        form.catalog_due = Some(now);
        form.begin();
        assert!(form.catalog_due.is_none(), "starting work stops model-discovery polling");
    }
}
