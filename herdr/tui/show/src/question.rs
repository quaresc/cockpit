//! Questions whose answer goes back to the session: a choice (options, the highlighted one's preview, a free answer
//! typed in a field) and a confirm (Cancel / Confirm), drawn like Textual's OptionList, Input and Buttons.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use serde_json::{Value, json};

use cockpit_core::text::{fit, width};
use cockpit_core::theme::{BG, DIM, FG, GREEN, LINE, ORANGE, RED, SURFACE, bold, dim, fg};

use crate::markdown::markdown;
use crate::row::{Act, Row, Target};

/// Textual's `OptionList { max-height: 8 }`, borders included.
const OPTIONS_SHOWN: usize = 6;
const CURSOR_BLURRED: Color = Color::Rgb(70, 56, 44);

pub enum Key {
    Unused,
    Used,
    Answer(Value, String),
}

/// A row of a box with Textual's tall border: thick left edge, thin right one, the top and bottom as eighth blocks.
fn edge(cols: usize, top: bool, color: Color, target: Target) -> Row {
    let line = if top { "▔" } else { "▁" }.repeat(cols.saturating_sub(2));
    Row::text("▊", fg(color).bg(SURFACE)).add(line, fg(color).bg(SURFACE)).add("▎", fg(color).bg(SURFACE)).target(target)
}

fn inside(cols: usize, color: Color, body: Vec<(String, Style)>, fill: Style, act: Option<Act>, target: Target) -> Row {
    let room = cols.saturating_sub(2);
    let used: usize = body.iter().map(|(t, _)| width(t)).sum();
    let mut row = Row::text("▊", fg(color).bg(SURFACE));
    let start = row.line.width();
    for (text, style) in body {
        row = row.add(text, style);
    }
    row = row.add(" ".repeat(room.saturating_sub(used)), fill).add("▎", fg(color).bg(SURFACE));
    if let Some(act) = act {
        row.clicks.push((start, start + room, act));
    }
    row.target(target)
}

pub struct Choice {
    prompt: Option<String>,
    answered: Option<String>,
    options: Vec<(String, Value, String)>,
    highlighted: usize,
    top: usize,
    input: String,
    sent: Option<String>,
}

impl Choice {
    pub fn new(block: &Value) -> Self {
        let options = block["options"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|o| {
                let label = o["label"].as_str().unwrap_or_default().to_string();
                let value = if o.get("value").is_some() { o["value"].clone() } else { json!(label) };
                (label, value, o["preview"].as_str().unwrap_or_default().to_string())
            })
            .collect();
        Self {
            prompt: block["prompt"].as_str().filter(|p| !p.is_empty()).map(str::to_string),
            answered: block["answered"].as_str().filter(|a| !a.is_empty()).map(str::to_string),
            options,
            highlighted: 0,
            top: 0,
            input: String::new(),
            sent: None,
        }
    }

    pub fn targets(&self) -> Vec<Target> {
        if self.sent.is_some() || self.answered.is_some() { vec![] } else { vec![Target::Options, Target::Input] }
    }

    pub fn rows(&mut self, cols: usize, focus: Option<&Target>) -> Vec<Row> {
        if let Some(label) = &self.sent {
            return vec![Row::text(format!("✓ sent to the session: {label}"), fg(GREEN))];
        }
        let mut out: Vec<Row> = self.prompt.iter().map(|p| Row::text(p.clone(), bold(FG))).collect();
        if let Some(answered) = &self.answered {
            out.push(Row::text(format!("✓ answered: {answered}"), fg(GREEN)));
            return out;
        }
        let on_options = focus == Some(&Target::Options);
        let color = if on_options { ORANGE } else { LINE };
        self.top = self.top.min(self.highlighted).max((self.highlighted + 1).saturating_sub(OPTIONS_SHOWN));
        out.push(edge(cols, true, color, Target::Options));
        for (i, (label, _, _)) in self.options.iter().enumerate().skip(self.top).take(OPTIONS_SHOWN) {
            let style = match (i == self.highlighted, on_options) {
                (true, true) => Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD),
                (true, false) => Style::new().fg(FG).bg(CURSOR_BLURRED),
                _ => Style::new().fg(FG).bg(SURFACE),
            };
            let text = format!(" {}", fit(label, cols.saturating_sub(4)).trim_end());
            out.push(inside(cols, color, vec![(text, style)], style, Some(Act::Pick(i)), Target::Options));
        }
        out.push(edge(cols, false, color, Target::Options));
        let typing = focus == Some(&Target::Input);
        let color = if typing { ORANGE } else { LINE };
        let field = Style::new().fg(FG).bg(SURFACE);
        let mut body = vec![(" ".to_string(), field)];
        if self.input.is_empty() && !typing {
            body.push(("or type another answer, enter sends".into(), fg(DIM).bg(SURFACE)));
        } else {
            body.push((self.input.clone(), field));
            if typing {
                body.push(("▏".into(), fg(ORANGE).bg(SURFACE)));
            }
        }
        out.push(edge(cols, true, color, Target::Input));
        out.push(inside(cols, color, body, field, None, Target::Input));
        out.push(edge(cols, false, color, Target::Input));
        if let Some((_, _, preview)) = self.options.get(self.highlighted) {
            out.extend(markdown(preview, cols));
        }
        out
    }

    fn send(&mut self, value: Value, label: String) -> Key {
        self.sent = Some(label.clone());
        Key::Answer(value, label)
    }

    pub fn act(&mut self, act: &Act) -> Key {
        match act {
            Act::Pick(i) if self.sent.is_none() => {
                self.highlighted = *i;
                let (label, value, _) = self.options[*i].clone();
                self.send(value, label)
            }
            _ => Key::Unused,
        }
    }

    pub fn key(&mut self, target: &Target, key: KeyEvent) -> Key {
        match (target, key.code) {
            (Target::Options, KeyCode::Up) => self.highlighted = self.highlighted.saturating_sub(1),
            (Target::Options, KeyCode::Down) => self.highlighted = (self.highlighted + 1).min(self.options.len().saturating_sub(1)),
            (Target::Options, KeyCode::Home) => self.highlighted = 0,
            (Target::Options, KeyCode::End) => self.highlighted = self.options.len().saturating_sub(1),
            (Target::Options, KeyCode::Enter) if !self.options.is_empty() => return self.act(&Act::Pick(self.highlighted)),
            (Target::Input, KeyCode::Enter) => {
                let text = self.input.trim().to_string();
                return if text.is_empty() { Key::Used } else { self.send(json!(text), text.clone()) };
            }
            (Target::Input, KeyCode::Backspace) => {
                self.input.pop();
            }
            (Target::Input, KeyCode::Char(c)) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => self.input.push(c),
            _ => return Key::Unused,
        }
        Key::Used
    }
}

pub struct Confirm {
    summary: String,
    consequence: Option<String>,
    label: String,
    sent: Option<(String, bool)>,
    /// The answer the mod wrote back into the spec, so a redrawn view does not ask again.
    answered: Option<String>,
}

impl Confirm {
    pub fn new(block: &Value) -> Self {
        Self {
            summary: block["summary"].as_str().unwrap_or_default().to_string(),
            consequence: block["consequence"].as_str().filter(|c| !c.is_empty()).map(str::to_string),
            label: block["confirm_label"].as_str().filter(|l| !l.is_empty()).unwrap_or("Confirm").to_string(),
            sent: None,
            answered: block["answered"].as_str().filter(|a| !a.is_empty()).map(str::to_string),
        }
    }

    pub fn targets(&self) -> Vec<Target> {
        if self.sent.is_some() || self.answered.is_some() { vec![] } else { vec![Target::Button(false), Target::Button(true)] }
    }

    pub fn rows(&self, cols: usize, focus: Option<&Target>) -> Vec<Row> {
        if let Some((label, yes)) = &self.sent {
            return vec![Row::text(format!("✓ answered: {label}"), if *yes { fg(GREEN) } else { dim() })];
        }
        let mut out = markdown(&self.summary, cols);
        out.extend(self.consequence.iter().flat_map(|c| crate::blocks::wrapped(c, cols, fg(RED))));
        if let Some(answered) = &self.answered {
            out.push(Row::text(format!("✓ answered: {answered}"), if answered == "cancelled" { dim() } else { fg(GREEN) }));
            return out;
        }
        let buttons = [(false, "Cancel".to_string(), LINE, Color::Rgb(72, 72, 78), Color::Rgb(24, 24, 26), FG), (true, self.label.clone(), RED, Color::Rgb(255, 176, 176), Color::Rgb(168, 76, 76), BG)];
        let mut rows = [Row::new(), Row::new(), Row::new()];
        for (yes, label, bg, light, dark, text) in buttons {
            let w = (width(&label) + 4).max(16);
            let pad = w - width(&label);
            let mut style = Style::new().fg(text).bg(bg).add_modifier(Modifier::BOLD);
            if focus == Some(&Target::Button(yes)) {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let middle = format!("{}{label}{}", " ".repeat(pad / 2), " ".repeat(pad - pad / 2));
            let [top, mid, bottom] = rows;
            rows = [
                top.act("▔".repeat(w), fg(light).bg(bg), Act::Answer(yes)).add("  ", Style::new()),
                mid.act(middle, style, Act::Answer(yes)).add("  ", Style::new()),
                bottom.act("▁".repeat(w), fg(dark).bg(bg), Act::Answer(yes)).add("  ", Style::new()),
            ];
        }
        out.extend(rows.into_iter().map(|r| r.target(Target::Button(false))));
        out
    }

    pub fn act(&mut self, act: &Act) -> Key {
        match act {
            Act::Answer(yes) if self.sent.is_none() && self.answered.is_none() => {
                let label = if *yes { self.label.clone() } else { "cancelled".into() };
                self.sent = Some((label.clone(), *yes));
                Key::Answer(json!(yes), label)
            }
            _ => Key::Unused,
        }
    }

    pub fn key(&mut self, target: &Target, key: KeyEvent) -> Key {
        match (target, key.code) {
            (Target::Button(yes), KeyCode::Enter | KeyCode::Char(' ')) => self.act(&Act::Answer(*yes)),
            _ => Key::Unused,
        }
    }
}
