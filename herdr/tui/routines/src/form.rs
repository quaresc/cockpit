//! The form that adds a routine: its name, when it runs (read back in words as you type, with its next run), the prompt
//! or script, the folder it runs in, and whether Claude or the shell runs it.

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
const LABELS: [&str; 4] = ["name", "when", "prompt", "folder"];

#[derive(Default)]
pub struct Form {
    pub fields: [Input; 4],
    /// The field typed in, KIND for the claude/shell switch.
    pub at: usize,
    pub shell: bool,
}

/// What a key did to the form.
pub enum Outcome {
    Stay,
    Cancel,
    /// Name, when, prompt, folder and "claude" or "shell", as the hub's `add` takes them.
    Submit(Vec<String>),
}

impl Form {
    pub fn new() -> Self {
        let mut form = Form::default();
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
                if let Some(next) = when.next_after(clock.local(clock.now)) {
                    let ms = next.and_local_timezone(clock.zone.unwrap_or(*Local::now().offset())).earliest().map(|t| t.timestamp_millis() as u64);
                    if let Some(ms) = ms {
                        hint.push_str(&format!(" · next {}", clock.until(ms)));
                    }
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
            _ if shift_tab => self.at = (self.at + KIND) % (KIND + 1),
            KeyCode::Up => self.at = self.at.saturating_sub(1),
            KeyCode::Tab | KeyCode::Down => self.at = (self.at + 1) % (KIND + 1),
            KeyCode::Enter if self.at < FOLDER => self.at += 1,
            KeyCode::Enter => {
                if self.problem().is_none() {
                    let kind = if self.shell { "shell" } else { "claude" };
                    let mut values: Vec<String> = (0..4).map(|f| self.text(f).to_string()).collect();
                    values.push(kind.into());
                    return Outcome::Submit(values);
                }
            }
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right if self.at == KIND => self.shell = !self.shell,
            _ if self.at < KIND => {
                self.fields[self.at].key(key);
            }
            _ => {}
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
                    (FOLDER, true) => ("the repos folder".to_string(), dim()),
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
            if on { Span::styled(format!(" {text} "), Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD)) } else { Span::styled(format!(" {text} "), Style::new().fg(FG).bg(LINE)) }
        };
        let kind_label = Span::styled(format!("{:<label_w$}", "runs"), if self.at == KIND { fg(ORANGE) } else { dim() });
        lines.push(Line::from(vec![kind_label, pill(!self.shell, "Claude"), Span::raw(" "), pill(self.shell, "shell")]));
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
