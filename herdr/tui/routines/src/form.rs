//! The form that adds a routine, blank or filled from a template picked first: its name, when it runs (read back in
//! words as you type, with its next three runs) and in which time zone, the prompt or script, the folder, the precheck,
//! an installed agent or the shell, its model, whether each run gets a worktree, and a template's tools.

use cockpit_core::input::Input;
use cockpit_core::routines::{TEMPLATES, Template, When, Zone};
use cockpit_core::text::wrap;
use cockpit_core::theme::{BG, DIM, FG, GREEN, LINE, ORANGE, RED, SURFACE, bold, dim, fg};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph};

use crate::data::{Agents, Clock};

pub const NAME: usize = 0;
pub const WHEN: usize = 1;
pub const ZONE: usize = 2;
pub const PROMPT: usize = 3;
pub const FOLDER: usize = 4;
pub const PRECHECK: usize = 5;
pub const KIND: usize = 6;
pub const MODEL: usize = 7;
pub const WORKTREE: usize = 8;
/// The most models drawn as pills; a longer list is stepped through one at a time.
const PILLS: usize = 6;
const LABELS: [&str; 6] = ["name", "when", "tz", "prompt", "folder", "precheck"];

#[derive(Default)]
pub struct Form {
    pub fields: [Input; 6],
    /// The field typed in, KIND for the agent/shell switch, MODEL for the model's.
    pub at: usize,
    pub shell: bool,
    /// The default agent's name, and whether it is Claude Code (whose tools and modes a template gives).
    pub agent: String,
    pub claude: bool,
    /// The selected agent id, empty for the default, and the installed choices from the hub.
    pub agent_set: String,
    pub agents: Agents,
    /// The models the agent takes as (id, label), its default first; none when there is nothing to pick.
    pub choices: Vec<(String, String)>,
    /// The model picked, by id: empty for the agent's default.
    pub model: String,
    pub worktree: bool,
    /// What a template gives it beyond the fields: its tools, permission mode, and when it notifies.
    pub tools: String,
    pub mode: String,
    pub notify: String,
}

/// The templates offered before the form, a blank routine first: the one at `at` is picked with enter.
#[derive(Default)]
pub struct Picker {
    pub at: usize,
}

/// What a key did to the picker: a template picked (None for a blank routine), or the picker closed.
pub enum Pick {
    Stay,
    Cancel,
    Picked(Option<&'static Template>),
}

impl Picker {
    pub fn key(&mut self, key: KeyEvent) -> Pick {
        let picked = |at: usize| Pick::Picked(at.checked_sub(1).map(|n| &TEMPLATES[n]));
        match key.code {
            KeyCode::Esc => return Pick::Cancel,
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => self.at = self.at.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => self.at = (self.at + 1).min(TEMPLATES.len()),
            KeyCode::Enter => return picked(self.at),
            KeyCode::Char(c @ '1'..='9') if (c as usize - '1' as usize) <= TEMPLATES.len() => return picked(c as usize - '1' as usize),
            _ => {}
        }
        Pick::Stay
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let width = 84.min(area.width.saturating_sub(4));
        let inner = (width as usize).saturating_sub(6);
        let mut lines = vec![Line::styled("New routine: start from", bold(ORANGE)), Line::raw("")];
        let choices = std::iter::once(("Blank", "", "a routine of your own")).chain(TEMPLATES.iter().map(|t| (t.name, t.when, t.about)));
        for (n, (name, when, about)) in choices.enumerate() {
            let on = n == self.at;
            let mark = if on { "▸ " } else { "  " };
            let head = format!("{mark}{} {name:<24}", n + 1);
            let rest = if when.is_empty() { about.to_string() } else { format!("{when} · {about}") };
            let room = inner.saturating_sub(cockpit_core::text::width(&head));
            let style = if on { Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD) } else { fg(FG) };
            lines
                .push(Line::from(vec![Span::styled(head, style), Span::styled(format!(" {}", cockpit_core::text::fit(&rest, room.saturating_sub(1))), dim())]));
        }
        lines.push(Line::raw(""));
        for part in wrap("j k move · enter or 1–5 picks · esc cancels · nothing is saved until you add the routine", inner) {
            lines.push(Line::styled(part, fg(DIM)));
        }
        let height = (lines.len() as u16 + 4).min(area.height);
        let box_ = Rect { x: area.x + (area.width - width) / 2, y: area.y + area.height.saturating_sub(height) / 2, width, height };
        frame.render_widget(Clear, box_);
        let block = Block::new().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(fg(ORANGE)).style(Style::new().bg(SURFACE));
        frame.render_widget(Paragraph::new(lines).block(block.padding(Padding::new(2, 2, 1, 1))), box_);
    }
}

/// What a key did to the form.
pub enum Outcome {
    Stay,
    Cancel,
    /// Name, when, prompt, folder, "" for the default agent or "shell", the model, "true" for a worktree per run, how
    /// it notifies (empty: always), the precheck, the time zone, a template's tools and mode, as the hub's `add` takes them.
    Submit(Vec<String>),
}

impl Form {
    /// A new form for the default agent: its name, and whether it is Claude Code.
    pub fn new((agent, claude): (String, bool)) -> Self {
        let mut form = Form { agent, claude, ..Form::default() };
        form.fields[WHEN] = Input::new("weekdays 09:00");
        // Its models come from the hub's agents (`refresh_agents`), as the app's form takes them.
        form.with_models(Vec::new())
    }

    /// The models the agent lists, preserving a model already selected while discovery refreshes.
    pub fn with_models(mut self, models: Vec<String>) -> Self {
        let mut ids = models;
        if !self.model.is_empty() && !ids.contains(&self.model) {
            ids.push(self.model.clone());
        }
        if !ids.iter().any(String::is_empty) {
            ids.insert(0, String::new());
        }
        let label = |id: &str| if id.is_empty() { "default" } else { id }.to_string();
        self.choices = if ids.len() > 1 { ids.iter().map(|id| (id.clone(), label(id))).collect() } else { Vec::new() };
        if !self.choices.iter().any(|(id, _)| *id == self.model) {
            self.model.clear();
        }
        self
    }

    /// Updates the installed agents and models without discarding typed fields or the model already selected.
    pub fn refresh_agents(&mut self, agents: &Agents) {
        self.agents = agents.clone();
        let id = if self.agent_set.is_empty() { &agents.default } else { &self.agent_set };
        self.claude = id.is_empty() || id == "claude";
        self.agent = agents
            .installed
            .iter()
            .find(|a| a["id"] == id.as_str())
            .and_then(|a| a["name"].as_str())
            .unwrap_or(if self.claude { "Claude Code" } else { id })
            .to_string();
        let model = self.model.clone();
        let models = agents.models_for(&self.agent_set);
        let choices = std::mem::take(self).with_models(models);
        *self = choices;
        self.model = model;
    }

    fn kinds(&self) -> Vec<(String, String)> {
        let (default, _) = self.agents.default_agent();
        let mut kinds = vec![(String::new(), if self.agents.default.is_empty() { self.agent.clone() } else { default })];
        kinds.extend(self.agents.installed.iter().filter_map(|a| {
            let id = a["id"].as_str()?;
            (id != self.agents.default).then(|| (id.to_string(), a["name"].as_str().unwrap_or(id).to_string()))
        }));
        if !self.agent_set.is_empty() && !kinds.iter().any(|(id, _)| id == &self.agent_set) {
            kinds.push((self.agent_set.clone(), self.agent.clone()));
        }
        kinds.push(("shell".into(), "shell".into()));
        kinds
    }

    fn step_kind(&mut self, by: isize) {
        let kinds = self.kinds();
        let selected = if self.shell { "shell" } else { &self.agent_set };
        let at = kinds.iter().position(|(id, _)| id == selected).unwrap_or(0) as isize;
        let id = kinds[(at + by).rem_euclid(kinds.len() as isize) as usize].0.clone();
        self.shell = id == "shell";
        if !self.shell {
            self.agent_set = id;
            self.model.clear();
            let agents = self.agents.clone();
            self.refresh_agents(&agents);
            if !self.claude {
                self.tools.clear();
                self.mode.clear();
            }
        }
    }

    fn step_model(&mut self, by: isize) {
        let n = self.choices.len() as isize;
        if n == 0 {
            return;
        }
        let at = self.choices.iter().position(|(id, _)| *id == self.model).unwrap_or(0) as isize;
        self.model = self.choices[(at + by).rem_euclid(n) as usize].0.clone();
    }

    /// A new form filled from a template: nothing more is saved than what the form then adds.
    pub fn from_template(agent: (String, bool), t: &Template) -> Self {
        let mut form = Form::new(agent);
        for (field, text) in [(NAME, t.name), (WHEN, t.when), (PROMPT, t.body), (PRECHECK, t.precheck)] {
            form.fields[field] = Input::new(text);
        }
        form.shell = t.shell;
        // Fitted to the agent already (`Template::for_agent`); the hub's list of its models comes with `refresh_agents`.
        form.model = t.model.to_string();
        form.worktree = t.worktree;
        (form.tools, form.mode, form.notify) = (t.tools.into(), t.mode.into(), t.notify.into());
        form
    }

    pub fn text(&self, field: usize) -> &str {
        self.fields[field].text.trim()
    }

    /// Why it cannot be added yet, if it cannot.
    pub fn problem(&self) -> Option<String> {
        if let Err(err) = When::parse(self.text(WHEN)) {
            return Some(err);
        }
        if let Err(err) = Zone::parse(self.text(ZONE)) {
            return Some(err);
        }
        self.text(PROMPT).is_empty().then(|| if self.shell { "the script is empty".into() } else { "the prompt is empty".into() })
    }

    /// The `when` field read back on the time zone's clock: what it means and its next runs on yours, or why either does
    /// not parse.
    pub fn when_hint(&self, clock: &Clock) -> (String, bool) {
        let zone = match clock.of(self.text(ZONE)) {
            Ok(zone) => zone,
            Err(err) => return (err, false),
        };
        match When::parse(self.text(WHEN)) {
            Ok(when) => {
                let mut hint = when.describe(self.text(WHEN));
                if matches!(when, When::Cron(_)) && !zone.name().is_empty() {
                    hint.push_str(&format!(" in {}", zone.name()));
                }
                let next: Vec<String> = std::iter::successors(when.next_after(zone.local(clock.now)), |at| when.next_after(*at))
                    .take(3)
                    .filter_map(|at| zone.epoch_ms(at).map(|ms| clock.until(ms)))
                    .collect();
                if !next.is_empty() {
                    hint.push_str(&format!(" · next {}", next.join(", ")));
                }
                (hint, true)
            }
            Err(err) => (err, false),
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let shift_tab = key.code == KeyCode::BackTab || (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT));
        match key.code {
            KeyCode::Esc => return Outcome::Cancel,
            _ if shift_tab => self.at = (self.at + WORKTREE) % (WORKTREE + 1),
            KeyCode::Up => self.at = self.at.saturating_sub(1),
            KeyCode::Tab | KeyCode::Down => self.at = (self.at + 1) % (WORKTREE + 1),
            KeyCode::Enter if self.at < PRECHECK => self.at += 1,
            KeyCode::Enter => {
                if self.problem().is_none() {
                    let kind = if self.shell { "shell" } else { &self.agent_set };
                    let mut values: Vec<String> = [NAME, WHEN, PROMPT, FOLDER].iter().map(|f| self.text(*f).to_string()).collect();
                    values.push(kind.into());
                    values.push(if self.shell || self.choices.is_empty() { String::new() } else { self.model.clone() });
                    values.push(if self.worktree { "true".into() } else { String::new() });
                    values.push(self.notify.clone());
                    values.extend([self.text(PRECHECK).to_string(), self.text(ZONE).to_string(), self.tools.clone(), self.mode.clone()]);
                    return Outcome::Submit(values);
                }
            }
            KeyCode::Left if self.at == KIND => self.step_kind(-1),
            KeyCode::Char(' ') | KeyCode::Right if self.at == KIND => self.step_kind(1),
            KeyCode::Left if self.at == MODEL => self.step_model(-1),
            KeyCode::Char(' ') | KeyCode::Right if self.at == MODEL => self.step_model(1),
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right if self.at == WORKTREE => self.worktree = !self.worktree,
            _ if self.at < KIND => {
                self.fields[self.at].key(key);
            }
            _ => {}
        }
        // A script has no model to pick, nor an agent that lists none.
        if (self.shell || self.choices.is_empty()) && self.at == MODEL {
            self.at = if shift_tab || key.code == KeyCode::Up { KIND } else { WORKTREE };
        }
        Outcome::Stay
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, clock: &Clock) {
        let width = 76.min(area.width.saturating_sub(4));
        let inner = (width as usize).saturating_sub(6);
        let label_w = 10;
        let mut lines = vec![Line::styled("New routine", bold(ORANGE)), Line::raw("")];
        for (n, label) in LABELS.iter().enumerate() {
            let label = if n == PROMPT && self.shell { "script" } else { label };
            let prompt = format!("{label:<label_w$}");
            let line = if n == self.at {
                self.fields[n].line(&prompt)
            } else {
                let value = &self.fields[n].text;
                let (text, style) = match (n, value.is_empty()) {
                    (NAME, true) => ("from the prompt".to_string(), dim()),
                    (FOLDER, true) if self.shell => ("the repos folder".to_string(), dim()),
                    (FOLDER, true) => ("a folder of its own".to_string(), dim()),
                    (ZONE, true) => ("the machine's".to_string(), dim()),
                    (PRECHECK, true) => ("none: it always runs".to_string(), dim()),
                    _ => (value.clone(), fg(FG)),
                };
                Line::from(vec![Span::styled(prompt, dim()), Span::styled(cut_line(&text, inner.saturating_sub(label_w)), style)])
            };
            lines.push(line);
            if n == ZONE {
                let (hint, ok) = self.when_hint(clock);
                for (n, part) in wrap(&hint, inner.saturating_sub(label_w + 2)).into_iter().enumerate() {
                    let lead = if n == 0 { "→ " } else { "  " };
                    lines.push(Line::from(vec![Span::raw(" ".repeat(label_w)), Span::styled(format!("{lead}{part}"), fg(if ok { GREEN } else { RED }))]));
                }
            }
            if n == PRECHECK && self.at == PRECHECK {
                let hint = "a shell command run first: exit 0 runs it, 1–125 skip it quietly; other exits, and over 60 s, fail and notify";
                for part in wrap(hint, inner.saturating_sub(label_w)) {
                    lines.push(Line::styled(format!("{:label_w$}{part}", ""), dim()));
                }
            }
        }
        let pill = |on: bool, text: &str| {
            if on {
                Span::styled(format!(" {text} "), Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD))
            } else {
                Span::styled(format!(" {text} "), Style::new().fg(FG).bg(LINE))
            }
        };
        let kind_label = Span::styled(format!("{:<label_w$}", "runs"), if self.at == KIND { fg(ORANGE) } else { dim() });
        let kinds = self.kinds();
        let selected = if self.shell { "shell" } else { &self.agent_set };
        let mut spans = vec![kind_label];
        if kinds.len() <= 3 {
            for (n, (id, label)) in kinds.iter().enumerate() {
                if n > 0 {
                    spans.push(Span::raw(" "));
                }
                spans.push(pill(id == selected, label));
            }
        } else {
            let at = kinds.iter().position(|(id, _)| id == selected).unwrap_or(0);
            spans.push(Span::styled(format!("‹ {} ›  {} of {}", cut_line(&kinds[at].1, inner.saturating_sub(label_w + 14)), at + 1, kinds.len()), fg(FG)));
        }
        lines.push(Line::from(spans));
        if !self.shell && !self.choices.is_empty() {
            let mut spans = vec![Span::styled(format!("{:<label_w$}", "model"), if self.at == MODEL { fg(ORANGE) } else { dim() })];
            if self.choices.len() <= PILLS {
                for (id, label) in &self.choices {
                    spans.push(pill(*id == self.model, label));
                    spans.push(Span::raw(" "));
                }
            } else {
                // One at a time: ‹ name › and where it is in the list.
                let at = self.choices.iter().position(|(id, _)| *id == self.model).unwrap_or(0);
                let arrows = if self.at == MODEL { fg(ORANGE) } else { fg(LINE) };
                let name = cut_line(&self.choices[at].1, inner.saturating_sub(label_w + 14));
                spans.extend([Span::styled("‹ ", arrows), Span::styled(name, fg(FG)), Span::styled(" ›", arrows)]);
                spans.push(Span::styled(format!("  {} of {}", at + 1, self.choices.len()), dim()));
            }
            lines.push(Line::from(spans));
            if self.model == "lightest" {
                lines.push(Line::styled(format!("{:label_w$}haiku with nothing loaded but the tools it lists", ""), dim()));
            }
        }
        let worktree_label = Span::styled(format!("{:<label_w$}", "worktree"), if self.at == WORKTREE { fg(ORANGE) } else { dim() });
        lines.push(Line::from(vec![worktree_label, pill(!self.worktree, "no"), Span::raw(" "), pill(self.worktree, "one per run")]));
        let notify = if self.notify == "always" { "" } else { self.notify.as_str() };
        let given = [("tools", self.tools.as_str()), ("mode", self.mode.as_str()), ("notifies", notify)];
        for (label, value) in given.into_iter().filter(|(label, v)| !v.is_empty() && !(self.shell && *label != "notifies")) {
            for (n, part) in wrap(value, inner.saturating_sub(label_w)).into_iter().enumerate() {
                let label = if n == 0 { label } else { "" };
                lines.push(Line::from(vec![Span::styled(format!("{label:<label_w$}"), dim()), Span::styled(part, fg(FG))]));
            }
        }
        lines.push(Line::raw(""));
        let help = match self.problem() {
            Some(problem) => Line::styled(problem, fg(RED)),
            None => Line::styled("tab moves · enter from the precheck on adds it · esc cancels", fg(DIM)),
        };
        lines.push(help);
        let height = (lines.len() as u16 + 4).min(area.height);
        let box_ = Rect { x: area.x + (area.width - width) / 2, y: area.y + area.height.saturating_sub(height) / 2, width, height };
        frame.render_widget(Clear, box_);
        let block = Block::new().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(fg(ORANGE)).style(Style::new().bg(SURFACE));
        frame.render_widget(Paragraph::new(lines).block(block.padding(Padding::new(2, 2, 1, 1))), box_);
    }
}

/// The text's end when it is longer than the cells: what was typed last stays in view.
fn cut_line(text: &str, cells: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= cells || cells < 2 {
        return text.to_string();
    }
    format!("…{}", chars[chars.len() - (cells - 1)..].iter().collect::<String>())
}
