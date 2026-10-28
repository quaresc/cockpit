//! `cockpit palette`: the launcher herdr opens as a popup over any pane (prefix+space). It searches every app, the
//! week's events and past Claude sessions as you type, and lists cockpit's own commands: the pinned spaces, a refresh of
//! the inbox, the doctor's report and today's brief. Over a Claude session, a found item can also go into that session:
//! typed at its prompt, asked about, or looked into by a fork of it in the actions space.

mod reference;
mod report;
mod then;

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use cockpit_core::hub::Found;
use cockpit_core::palette::{Entry, Outcome, Palette};
use cockpit_core::theme::{FG, ORANGE, PANEL, base, bold, dim, fg};
use cockpit_core::{actions, files, keys, term};
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
    /// Whether a Claude agent runs there: only then can a found item go into it.
    pub is_claude: bool,
    /// The Claude session's id, which a fork resumes.
    pub session: Option<String>,
    pub cwd: String,
    pub title: String,
}

impl Over {
    /// From `herdr pane get`'s pane.
    pub fn of(pane: &Value) -> Option<Over> {
        let id = pane["pane_id"].as_str().filter(|p| !p.is_empty())?;
        let is_claude = pane["agent"] == "claude";
        let session = pane["agent_session"]["value"].as_str().filter(|_| is_claude && pane["agent_session"]["kind"] == "id").map(str::to_string);
        let title = pane["terminal_title_stripped"].as_str().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(id).to_string();
        Some(Over { pane: id.into(), is_claude, session, cwd: pane["cwd"].as_str().unwrap_or_default().into(), title })
    }
}

/// The palette's own entries.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Space(String),
    RefreshInbox,
    Doctor,
    Brief,
}

impl Command {
    fn entry(&self, id: usize) -> Entry {
        let (text, help) = match self {
            Command::Space(name) => (format!("Go to the {name} space"), "a pinned space".to_string()),
            Command::RefreshInbox => ("Refresh the inbox".into(), "PRs, reviews, tickets and CI fetched again now".into()),
            Command::Doctor => ("Check cockpit".into(), "the doctor's report: every part of cockpit, ok, warn or fail".into()),
            Command::Brief => ("Today's brief".into(), "the morning brief: meetings, reviews, CI, tickets".into()),
        };
        Entry { text, help, featured: true, id }
    }
}

/// The ctrl keys that send a found item into the Claude session below, with their footer labels.
const SESSION_KEYS: [(char, &str); 3] = [('r', "insert"), ('a', "ask"), ('f', "fork")];

/// What a session key does with a found item, None when it cannot (no Claude there, or no session id to fork).
pub fn into_session(key: char, over: &Over, found: &Found, inbox: &Value, config: &Value) -> Option<Then> {
    if !over.is_claude {
        return None;
    }
    let text = reference(found, inbox, config);
    let pane = over.pane.clone();
    Some(match key {
        'r' => Then::Insert { pane, text: format!("{text} ") },
        'a' => Then::Ask { pane, text: format!("Look at {text}: tell me what it is, where it stands, and what it means for what we are doing here.") },
        'f' => Then::Fork {
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

pub struct App {
    palette: Palette,
    commands: Vec<Command>,
    over: Option<Over>,
    asking: Option<Receiver<Option<Over>>>,
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
                let _ = tx.send(actions::herdr(&["pane", "get", &pane]).and_then(|r| Over::of(&r["pane"])));
                term::wake();
            });
            rx
        });
        App::new(commands(&actions::pinned()), None, asking)
    }

    pub fn new(commands: Vec<Command>, over: Option<Over>, asking: Option<Receiver<Option<Over>>>) -> App {
        let mut palette = Palette::default();
        palette.app = "palette".into();
        palette.open(commands.iter().enumerate().map(|(n, c)| c.entry(n)).collect());
        App { palette, commands, over, asking, report: None, then: None }
    }

    fn reopen(&mut self) {
        self.report = None;
        self.palette.open(self.commands.iter().enumerate().map(|(n, c)| c.entry(n)).collect());
    }

    /// The Claude session below, when there is one.
    fn claude(&self) -> Option<&Over> {
        self.over.as_ref().filter(|o| o.is_claude)
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
        let (over, found) = (self.claude()?, self.palette.selected_found()?);
        let inbox = files::load(&files::cache().join("inbox.json")).unwrap_or(Value::Null);
        into_session(c, over, found, &inbox, &files::config())
    }

    fn choose(&mut self, id: usize) -> bool {
        match self.commands.get(id).cloned() {
            Some(Command::Space(name)) => self.then = Some(Then::Space(name)),
            Some(Command::RefreshInbox) => self.then = Some(Then::RefreshInbox),
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
            && let Ok(over) = rx.try_recv()
        {
            self.over = over;
            self.asking = None;
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
        let into = self.report.is_none() && self.claude().is_some() && self.palette.selected_found().is_some();
        let mut spans = vec![Span::raw(" ")];
        for (key, what) in keys {
            spans.extend([Span::styled(format!("{key} "), bold(ORANGE)), Span::styled(format!("{what}  "), fg(FG))]);
        }
        if into {
            let forkable = self.claude().is_some_and(|o| o.session.is_some());
            for (key, what) in SESSION_KEYS.into_iter().filter(|(k, _)| *k != 'f' || forkable) {
                spans.extend([Span::styled(format!("^{key} "), bold(ORANGE)), Span::styled(format!("{what}  "), fg(FG))]);
            }
        }
        let right = match self.claude() {
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
