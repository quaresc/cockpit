//! `cockpit palette`: the launcher herdr opens as a popup over any pane (prefix+space). It searches every app, the
//! week's events and past agent sessions as you type, and lists cockpit's own commands: the pinned spaces, a refresh of
//! the inbox, the doctor's report and today's brief. Over an agent's session, a found item can also go into that
//! session: typed at its prompt, asked about, or looked into by a fork of it in the actions space.

pub mod gui;
mod reference;
mod report;
mod then;

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use cockpit_core::hub::Found;
use cockpit_core::palette::{Entry, Outcome, Palette};
use cockpit_core::theme::{FG, ORANGE, PANEL, base, bold, dim, fg};
use cockpit_core::{actions, agent, files, keys, term};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

pub use reference::reference;
use report::Report;
pub use then::Then;

/// `cockpit palette`: the popup until a choice or esc, then the choice, its failure told in a notification.
pub fn run() -> std::io::Result<()> {
    let mut app = App::start(std::env::var("PALETTE_PANE").ok().filter(|p| !p.is_empty()));
    term::run(&mut app)?;
    if let Some(then) = app.then.take()
        && let Err(error) = then.run()
    {
        cockpit_core::notify::post("⚠️ Cockpit", &error, "", &[]);
    }
    Ok(())
}

/// The pane the popup opened over, as herdr reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Over {
    pub pane: String,
    /// The id of the agent running there (`claude`, `codex`): only then can a found item go into it.
    pub agent: Option<String>,
    /// The agent's session id, which a fork resumes.
    pub session: Option<String>,
    pub cwd: String,
    pub title: String,
}

impl Over {
    /// From `herdr pane get`'s pane.
    pub fn of(pane: &Value) -> Option<Over> {
        let id = pane["pane_id"].as_str().filter(|p| !p.is_empty())?;
        let agent = pane["agent"].as_str().and_then(agent::of_program).map(|a| a.id);
        let session = pane["agent_session"]["value"].as_str().filter(|_| agent.is_some() && pane["agent_session"]["kind"] == "id").map(str::to_string);
        let title = pane["terminal_title_stripped"].as_str().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(id).to_string();
        Some(Over { pane: id.into(), agent, session, cwd: pane["cwd"].as_str().unwrap_or_default().into(), title })
    }

    /// The session to fork, when the agent below has one and can fork it.
    pub fn forkable(&self) -> Option<(String, String)> {
        let agent = agent::get(self.agent.as_deref()?).filter(|a| !a.fork.is_empty())?;
        Some((agent.id, self.session.clone()?))
    }
}

/// The palette's own entries.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Space(String),
    RefreshInbox,
    Doctor,
    Brief,
    /// The folder of the pane below, in the person's editor.
    Editor(String),
    /// The agent's session below, forked to a tab of the actions space.
    Fork {
        agent: String,
        session: String,
        cwd: String,
        title: String,
    },
    /// A page the agent below works with, named after its service (GitHub, Linear, Argo…).
    Link {
        label: String,
        detail: String,
        url: String,
    },
    /// A view the agent below showed, opened in a pane beside it.
    Document {
        pane: String,
        name: String,
        title: String,
    },
}

impl Command {
    fn entry(&self, id: usize) -> Entry {
        let (text, help) = match self {
            Command::Space(name) => (format!("Go to the {name} space"), "a pinned space".to_string()),
            Command::RefreshInbox => ("Refresh the inbox".into(), "PRs, reviews, tickets and CI fetched again now".into()),
            Command::Doctor => ("Check cockpit".into(), "the doctor's report: every part of cockpit, ok, warn or fail".into()),
            Command::Brief => ("Today's brief".into(), "the morning brief: meetings, reviews, CI, tickets".into()),
            Command::Editor(cwd) => (format!("Open {} in your editor", folder_name(cwd)), cwd.clone()),
            Command::Fork { title, .. } => ("Fork this session to a new tab".into(), format!("{title}, its conversation carried over")),
            Command::Link { label, detail, url } => (format!("Open {label}"), if detail.is_empty() { url.clone() } else { detail.clone() }),
            Command::Document { title, .. } => (format!("Show {title}"), "a document this agent showed, in a pane beside it".into()),
        };
        Entry { text, help, featured: true, id }
    }
}

/// The ctrl keys that send a found item into the agent's session below, with their footer labels.
const SESSION_KEYS: [(char, &str); 3] = [('r', "insert"), ('a', "ask"), ('f', "fork")];

/// What a session key does with a found item, None when it cannot (no agent there, or no session it can fork).
pub fn into_session(key: char, over: &Over, found: &Found, inbox: &Value, config: &Value) -> Option<Then> {
    over.agent.as_ref()?;
    let text = reference(found, inbox, config);
    let pane = over.pane.clone();
    Some(match key {
        'r' => Then::Insert { pane, text: format!("{text} ") },
        'a' => Then::Ask { pane, text: format!("Look at {text}: tell me what it is, where it stands, and what it means for what we are doing here.") },
        'f' => Then::Fork {
            agent: over.forkable()?.0,
            session: over.session.clone()?,
            cwd: over.cwd.clone(),
            title: found.title.clone(),
            prompt: format!(
                "This is a fork of a session, opened from the cockpit palette to look into {text}. Look into it with what this \
                 session knows, sum up in a few lines what you find, then wait for my questions. Change nothing (no edits, \
                 commits, pushes or ticket updates) unless I ask."
            ),
        },
        _ => return None,
    })
}

fn folder_name(cwd: &str) -> &str {
    cwd.trim_end_matches('/').rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or(cwd)
}

/// The agent's links and documents, as the hub's agents view has them for the pane below: links first, in its order.
pub fn agent_commands(pane: &str, agents: &Value) -> Vec<Command> {
    let Some(row) = agents["agents"].as_array().into_iter().flatten().find(|r| r["terminal"] == pane) else { return Vec::new() };
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let links = row["links"].as_array().into_iter().flatten().map(|l| Command::Link {
        label: Some(s(&l["label"])).filter(|t| !t.is_empty()).unwrap_or_else(|| s(&l["name"])),
        detail: s(&l["detail"]),
        url: s(&l["url"]),
    });
    let documents = row["documents"].as_array().into_iter().flatten().map(|d| Command::Document {
        pane: pane.to_string(),
        name: s(&d["name"]),
        title: Some(s(&d["title"])).filter(|t| !t.is_empty()).unwrap_or_else(|| s(&d["name"])),
    });
    links.chain(documents).collect()
}

/// What the pane below adds: its folder in the editor, and a fork of its agent's session.
pub fn session_commands(over: &Over) -> Vec<Command> {
    let mut out = Vec::new();
    if let Some((agent, session)) = over.forkable() {
        out.push(Command::Fork { agent, session, cwd: over.cwd.clone(), title: over.title.clone() });
    }
    if !over.cwd.is_empty() {
        out.push(Command::Editor(over.cwd.clone()));
    }
    out
}

pub struct App {
    palette: Palette,
    commands: Vec<Command>,
    over: Option<Over>,
    asking: Option<Receiver<(Option<Over>, Vec<Command>)>>,
    report: Option<Report>,
    /// What to do once the popup has closed.
    pub then: Option<Then>,
}

impl App {
    /// Draws at once: the pane below is asked of herdr in the background, and the hub only once something is typed.
    pub fn start(pane: Option<String>) -> App {
        // A hub that is not running yet starts now, while the first letters are typed, rather than on the first search.
        std::thread::spawn(|| cockpit_core::hub::request(&serde_json::json!({"op": "ping"}), Duration::from_secs(3)));
        let asking = pane.map(|pane| {
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let over = actions::herdr(&["pane", "get", &pane]).and_then(|r| Over::of(&r["pane"]));
                let request = serde_json::json!({"op": "view", "name": "agents", "args": {}});
                let agents = cockpit_core::hub::request(&request, Duration::from_secs(3)).map(|r| r["value"].clone()).unwrap_or_default();
                let _ = tx.send((over, agent_commands(&pane, &agents)));
                term::wake();
            });
            rx
        });
        App::new(commands(&actions::pinned()), None, asking)
    }

    pub fn new(mut commands: Vec<Command>, over: Option<Over>, asking: Option<Receiver<(Option<Over>, Vec<Command>)>>) -> App {
        commands.extend(over.iter().flat_map(session_commands));
        let mut palette = Palette::default();
        palette.app = "palette".into();
        palette.open(commands.iter().enumerate().map(|(n, c)| c.entry(n)).collect());
        App { palette, commands, over, asking, report: None, then: None }
    }

    fn reopen(&mut self) {
        self.report = None;
        self.palette.open(self.commands.iter().enumerate().map(|(n, c)| c.entry(n)).collect());
    }

    /// The agent's session below, when there is one.
    fn below(&self) -> Option<&Over> {
        self.over.as_ref().filter(|o| o.agent.is_some())
    }

    pub fn event(&mut self, event: Event) -> bool {
        let Event::Key(key) = event else { return false };
        if key.kind == KeyEventKind::Release {
            return false;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('c' | 'q')) {
            return true;
        }
        if self.report.is_some() {
            return self.report_key(key);
        }
        if let (true, KeyCode::Char(c)) = (ctrl, key.code)
            && let Some(then) = self.session_key(c)
        {
            self.then = Some(then);
            return true;
        }
        match self.palette.key(key) {
            Outcome::Open => false,
            Outcome::Closed => true,
            Outcome::Elsewhere(target) => {
                self.then = Some(Then::Go(target));
                true
            }
            Outcome::Chosen(id) => self.choose(id),
        }
    }

    fn session_key(&self, c: char) -> Option<Then> {
        SESSION_KEYS.iter().find(|(k, _)| *k == c)?;
        let (over, found) = (self.below()?, self.palette.selected_found()?);
        let inbox = files::load(&files::cache().join("inbox.json")).unwrap_or(Value::Null);
        into_session(c, over, found, &inbox, &files::config())
    }

    fn choose(&mut self, id: usize) -> bool {
        match self.commands.get(id).cloned() {
            Some(Command::Space(name)) => self.then = Some(Then::Space(name)),
            Some(Command::RefreshInbox) => self.then = Some(Then::RefreshInbox),
            Some(Command::Editor(cwd)) => self.then = Some(Then::Editor(cwd)),
            Some(Command::Fork { agent, session, cwd, title }) => self.then = Some(Then::Fork { agent, session, cwd, title, prompt: String::new() }),
            Some(Command::Link { url, .. }) => self.then = Some(Then::Open(url)),
            Some(Command::Document { pane, name, .. }) => self.then = Some(Then::Document { pane, name }),
            Some(Command::Doctor) => {
                self.report = Some(Report::doctor());
                return false;
            }
            Some(Command::Brief) => {
                let brief = files::load(&files::cache().join("brief.json")).unwrap_or(Value::Null);
                self.report = Some(Report::brief(&brief, &chrono::Local::now().format("%Y-%m-%d").to_string()));
                return false;
            }
            None => {}
        }
        true
    }

    /// Esc goes back to the search, q closes, Enter opens the line's link.
    fn report_key(&mut self, key: KeyEvent) -> bool {
        let Some(report) = &mut self.report else { return false };
        match key.code {
            KeyCode::Esc => self.reopen(),
            KeyCode::Char('q') => return true,
            KeyCode::Enter => {
                if let Some(url) = report.selected_url() {
                    self.then = Some(Then::Open(url));
                    return true;
                }
            }
            _ => {
                if let Some(movement) = keys::movement(&key) {
                    report.go(movement);
                }
            }
        }
        false
    }

    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.asking
            && let Ok((over, agent)) = rx.try_recv()
        {
            // The agent's own links and documents come first: over an agent, they are what the palette is opened for.
            self.commands.splice(0..0, agent);
            self.commands.extend(over.iter().flat_map(session_commands));
            self.over = over;
            self.asking = None;
            if self.report.is_none() {
                self.palette.replace(self.commands.iter().enumerate().map(|(n, c)| c.entry(n)).collect());
            }
            changed = true;
        }
        if let Some(report) = &mut self.report {
            changed |= report.pump();
        }
        changed
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::new().style(base()), area);
        let [main, footer] = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
        match &mut self.report {
            Some(report) => report.draw(frame, main),
            None => self.palette.render_in(frame, main),
        }
        self.footer(frame, footer);
    }

    /// The keys that work now, and at the right the session a found item would go into.
    fn footer(&self, frame: &mut Frame, area: Rect) {
        let keys: Vec<(&str, &str)> = match &self.report {
            Some(report) if report.selected_url().is_some() => vec![("enter", "open"), ("esc", "back"), ("q", "close")],
            Some(_) => vec![("esc", "back"), ("q", "close")],
            None => vec![("enter", "go"), ("esc", "close")],
        };
        let into = self.report.is_none() && self.below().is_some() && self.palette.selected_found().is_some();
        let mut spans = vec![Span::raw(" ")];
        for (key, what) in keys {
            spans.extend([Span::styled(format!("{key} "), bold(ORANGE)), Span::styled(format!("{what}  "), fg(FG))]);
        }
        if into {
            let forkable = self.below().is_some_and(|o| o.forkable().is_some());
            for (key, what) in SESSION_KEYS.into_iter().filter(|(k, _)| *k != 'f' || forkable) {
                spans.extend([Span::styled(format!("^{key} "), bold(ORANGE)), Span::styled(format!("{what}  "), fg(FG))]);
            }
        }
        let right = match self.below() {
            Some(over) => vec![Span::styled("over ", dim()), Span::styled(format!("{} ", over.title), fg(ORANGE))],
            None => Vec::new(),
        };
        let used: usize = spans.iter().chain(&right).map(|s| s.content.width()).sum();
        if used <= area.width as usize {
            spans.push(Span::raw(" ".repeat(area.width as usize - used)));
            spans.extend(right);
        }
        frame.render_widget(Paragraph::new(Line::from(spans)).style(Style::new().bg(PANEL)), area);
    }
}

/// The pinned spaces to jump to, then the commands.
pub fn commands(pinned: &[String]) -> Vec<Command> {
    let spaces = pinned.iter().filter(|n| !n.is_empty()).map(|n| Command::Space(n.clone()));
    spaces.chain([Command::RefreshInbox, Command::Doctor, Command::Brief]).collect()
}

impl term::App for App {
    fn draw(&mut self, frame: &mut Frame) {
        App::draw(self, frame);
    }

    fn event(&mut self, event: Event) -> bool {
        App::event(self, event)
    }

    fn tick(&mut self) -> bool {
        App::tick(self)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(1)
    }
}
