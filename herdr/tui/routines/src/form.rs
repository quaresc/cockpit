//! The form that adds a routine: its name, when it runs (read back in words as you type, with its next three runs), the
//! prompt or script, the folder it runs in, whether the default agent or the shell runs it, Claude Code's model when it
//! is the agent, and whether each run gets a worktree of its own.

use chrono::Local;
use cockpit_core::input::Input;
use cockpit_core::routines::When;
use cockpit_core::text::wrap;
use cockpit_core::theme::{BG, DIM, FG, GREEN, LINE, ORANGE, RED, SURFACE, bold, dim, fg};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph};

use crate::data::Clock;

pub const NAME: usize = 0;
pub const WHEN: usize = 1;
pub const PROMPT: usize = 2;
pub const FOLDER: usize = 3;
pub const KIND: usize = 4;
pub const MODEL: usize = 5;
pub const WORKTREE: usize = 6;
/// The model switch's choices for Claude Code, as the hub's `add` takes them: its default first.
const MODELS: [(&str, &str); 5] = [("", "default"), ("lightest", "lightest"), ("haiku", "haiku"), ("sonnet", "sonnet"), ("opus", "opus")];
const LABELS: [&str; 4] = ["name", "when", "prompt", "folder"];

#[derive(Default)]
pub struct Form {
    pub fields: [Input; 4],
    /// The field typed in, KIND for the agent/shell switch, MODEL for the model's.
    pub at: usize,
    pub shell: bool,
    /// The default agent's name, and whether it is Claude Code, the only one whose models the form offers.
    pub agent: String,
    pub claude: bool,
    /// Which of MODELS.
    pub model: usize,
    pub worktree: bool,
}

/// What a key did to the form.
pub enum Outcome {
    Stay,
    Cancel,
    /// Name, when, prompt, folder, "" for the default agent or "shell", the model and "true" for a worktree per run, as
    /// the hub's `add` takes them.
    Submit(Vec<String>),
}

impl Form {
    /// A new form for the default agent: its name, and whether it is Claude Code.
    pub fn new((agent, claude): (String, bool)) -> Self {
        let mut form = Form { agent, claude, ..Form::default() };
        form.fields[WHEN] = Input::new("weekdays 09:00");
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
        self.text(PROMPT).is_empty().then(|| if self.shell { "the script is empty".into() } else { "the prompt is empty".into() })
    }

    /// The `when` field read back: what it means and its next run, or why it does not parse.
    pub fn when_hint(&self, clock: &Clock) -> (String, bool) {
        match When::parse(self.text(WHEN)) {
            Ok(when) => {
                let mut hint = when.describe(self.text(WHEN));
                let zone = clock.zone.unwrap_or(*Local::now().offset());
                let next: Vec<String> = std::iter::successors(when.next_after(clock.local(clock.now)), |at| when.next_after(*at))
                    .take(3)
                    .filter_map(|at| at.and_local_timezone(zone).earliest().map(|t| clock.until(t.timestamp_millis() as u64)))
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
            KeyCode::Enter if self.at < FOLDER => self.at += 1,
            KeyCode::Enter => {
                if self.problem().is_none() {
                    let kind = if self.shell { "shell" } else { "" };
                    let mut values: Vec<String> = (0..4).map(|f| self.text(f).to_string()).collect();
                    values.push(kind.into());
                    values.push(if self.shell || !self.claude { String::new() } else { MODELS[self.model].0.to_string() });
                    values.push(if self.worktree { "true".into() } else { String::new() });
                    return Outcome::Submit(values);
                }
            }
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right if self.at == KIND => self.shell = !self.shell,
            KeyCode::Left if self.at == MODEL => self.model = (self.model + MODELS.len() - 1) % MODELS.len(),
            KeyCode::Char(' ') | KeyCode::Right if self.at == MODEL => self.model = (self.model + 1) % MODELS.len(),
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right if self.at == WORKTREE => self.worktree = !self.worktree,
            _ if self.at < KIND => {
                self.fields[self.at].key(key);
            }
            _ => {}
        }
        // A script has no model to pick, nor an agent other than Claude Code here.
        if (self.shell || !self.claude) && self.at == MODEL {
            self.at = if shift_tab || key.code == KeyCode::Up { KIND } else { WORKTREE };
        }
        Outcome::Stay
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, clock: &Clock) {
        let width = 76.min(area.width.saturating_sub(4));
        let inner = (width as usize).saturating_sub(6);
        let label_w = 8;
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
                    _ => (value.clone(), fg(FG)),
                };
                Line::from(vec![Span::styled(prompt, dim()), Span::styled(cut_line(&text, inner.saturating_sub(label_w)), style)])
            };
            lines.push(line);
            if n == WHEN {
                let (hint, ok) = self.when_hint(clock);
                for part in wrap(&hint, inner.saturating_sub(label_w + 2)) {
                    lines.push(Line::from(vec![Span::raw(" ".repeat(label_w)), Span::styled(format!("→ {part}"), fg(if ok { GREEN } else { RED }))]));
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
        let agent = if self.agent.is_empty() { "agent" } else { &self.agent };
        lines.push(Line::from(vec![kind_label, pill(!self.shell, agent), Span::raw(" "), pill(self.shell, "shell")]));
        if !self.shell && self.claude {
            let mut spans = vec![Span::styled(format!("{:<label_w$}", "model"), if self.at == MODEL { fg(ORANGE) } else { dim() })];
            for (n, (_, label)) in MODELS.iter().enumerate() {
                spans.push(pill(n == self.model, label));
                spans.push(Span::raw(" "));
            }
            lines.push(Line::from(spans));
            if MODELS[self.model].0 == "lightest" {
                lines.push(Line::styled(format!("{:label_w$}haiku with nothing loaded but the tools it lists", ""), dim()));
            }
        }
        let worktree_label = Span::styled(format!("{:<label_w$}", "worktree"), if self.at == WORKTREE { fg(ORANGE) } else { dim() });
        lines.push(Line::from(vec![worktree_label, pill(!self.worktree, "no"), Span::raw(" "), pill(self.worktree, "one per run")]));
        lines.push(Line::raw(""));
        let help = match self.problem() {
            Some(problem) => Line::styled(problem, fg(RED)),
            None => Line::styled("tab moves · enter from the folder on adds it · esc cancels", fg(DIM)),
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
