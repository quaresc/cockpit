//! A row's actions, drawn as buttons with their keys, the main one in the accent colour.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{BG, FG, LINE, ORANGE};

#[derive(Clone, Debug, Default)]
pub struct Act {
    pub label: String,
    pub key: Option<char>,
    pub main: bool,
    /// The key the text app's `confirmed` checks: a dialog asks first, then the call goes out armed.
    pub confirm: Option<String>,
    /// Labels to pick from in a dialog before the action runs, the picked index passed to it.
    pub choices: Vec<String>,
    /// What the app runs: its own name for the action.
    pub id: String,
}

impl Act {
    pub fn new(label: &str, key: char, id: &str) -> Self {
        Self { label: label.into(), key: Some(key), id: id.into(), ..Default::default() }
    }

    pub fn main(mut self) -> Self {
        self.main = true;
        self
    }

    pub fn confirm(mut self, key: &str) -> Self {
        self.confirm = Some(key.into());
        self
    }

    pub fn choices(mut self, choices: &[&str]) -> Self {
        self.choices = choices.iter().map(|c| c.to_string()).collect();
        self
    }
}

/// The buttons as one line, each "label  key" on its own pill.
pub fn buttons(acts: &[Act]) -> Line<'static> {
    let mut spans = Vec::new();
    for act in acts {
        let style = if act.main { Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD) } else { Style::new().fg(FG).bg(LINE) };
        let key = act.key.map(|k| format!("  {k}")).unwrap_or_default();
        spans.push(Span::styled(format!(" {}{key} ", act.label), style));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

/// The action whose key this is.
pub fn by_key(acts: &[Act], key: char) -> Option<&Act> {
    acts.iter().find(|a| a.key == Some(key))
}
