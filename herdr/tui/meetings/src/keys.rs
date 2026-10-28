//! Keys and clicks: rows (or the week view's blocks), the selected meeting's actions, and the palette over all of them.

use cockpit_core::acts::Act;
use cockpit_core::keys::{self, Move};
use cockpit_core::palette::{Entry, Outcome};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Position;

use crate::app::{App, Cmd};
use crate::timeline::Layout;
use crate::week::{Meeting, day_label};

impl App {
    /// A meeting's actions in order: its brief, join, open, then prepare or redo.
    pub fn acts_for(&self, m: &Meeting) -> Vec<(Act, Cmd)> {
        let id = m.id.clone();
        let has_brief = self.briefs.contains_key(&id);
        let mut acts = Vec::new();
        if has_brief {
            let label = if self.open.contains(&id) { "hide brief" } else { "brief" };
            acts.push((Act::new(label, 'b', "brief"), Cmd::Brief(id.clone())));
        }
        if m.join.is_some() {
            acts.push((Act::new("join", 'm', "join").main(), Cmd::Join(id.clone())));
        }
        let mut open = Act::new("open", 'o', "open");
        open.main = m.join.is_none();
        acts.push((open, Cmd::Open(id.clone())));
        if has_brief {
            acts.push((Act::new("redo", 'r', "redo"), Cmd::Prepare(id)));
        } else if m.briefable && !m.is_past(&self.clock) && self.brief_enabled {
            acts.push((Act::new("prepare", 'p', "prepare"), Cmd::Prepare(id)));
        }
        acts
    }

    pub fn acts(&self) -> Vec<(Act, Cmd)> {
        self.selected_meeting().map(|m| self.acts_for(m)).unwrap_or_default()
    }

    /// The view's toggle, then every meeting's actions and a jump to each; the upcoming calls first.
    fn palette_entries(&mut self) -> Vec<Entry> {
        let mut cmds = Vec::new();
        let mut entries = Vec::new();
        let label = if self.week_view { "list view" } else { "week view" };
        entries.push(Entry { text: label.into(), help: "w".into(), featured: true, id: 0 });
        cmds.push(Cmd::Week);
        let today = self.clock.today();
        for m in self.shown() {
            let local = self.clock.local(m.start);
            let help = format!("{} {} · {}", day_label(local.date(), &self.clock), m.span_text(&self.clock), m.duration_text());
            let upcoming_today = local.date() == today && !m.is_past(&self.clock);
            for (act, cmd) in self.acts_for(m) {
                let featured = act.id == "join" && upcoming_today;
                entries.push(Entry { text: format!("{}  {}", act.label, m.title), help: help.clone(), featured, id: cmds.len() });
                cmds.push(cmd);
            }
            entries.push(Entry { text: format!("go to  {}", m.title), help, featured: false, id: cmds.len() });
            cmds.push(Cmd::GoTo(m.id.clone()));
        }
        self.cmds = cmds;
        entries
    }

    pub fn event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Mouse(mouse) => {
                let at = Position::new(mouse.column, mouse.row);
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) if !self.palette.is_open => self.click(at),
                    MouseEventKind::ScrollDown if self.hits.list.contains(at) => self.scroll_by(3),
                    MouseEventKind::ScrollUp if self.hits.list.contains(at) => self.scroll_by(-3),
                    _ => {}
                }
                false
            }
            _ => false,
        }
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && keys::is_quit(&key) {
            return true;
        }
        if self.palette.is_open {
            match self.palette.key(key) {
                Outcome::Chosen(id) => {
                    if let Some(cmd) = self.cmds.get(id).cloned() {
                        self.run(cmd);
                    }
                }
                Outcome::Elsewhere(target) => cockpit_core::hub::go(&target),
                _ => {}
            }
            return false;
        }
        if ctrl && key.code == KeyCode::Char('p') {
            let entries = self.palette_entries();
            self.palette.open(entries);
            return false;
        }
        if ctrl || key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        if keys::is_quit(&key) {
            return true;
        }
        if let Some(to) = keys::movement(&key) {
            match to {
                Move::By(n) => self.move_sel(n),
                Move::Pages(n) => self.move_sel(10 * n),
                Move::Top => self.select_edge(false),
                Move::Bottom => self.select_edge(true),
                Move::Group(_) => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Char('w') => self.run(Cmd::Week),
            KeyCode::Esc if self.week_view => self.week_view = false,
            KeyCode::Enter => {
                let acts = self.acts();
                if let Some((_, cmd)) = acts.iter().find(|(a, _)| a.main).or(acts.first()) {
                    self.run(cmd.clone());
                }
            }
            KeyCode::Char(c) => {
                if let Some((_, cmd)) = self.acts().into_iter().find(|(a, _)| a.key == Some(c)) {
                    self.run(cmd);
                }
            }
            _ => {}
        }
        false
    }

    fn click(&mut self, at: Position) {
        if let Some((_, cmd)) = self.hits.buttons.iter().find(|(r, _)| r.contains(at)) {
            let cmd = cmd.clone();
            self.run(cmd);
        } else if self.week_view && self.hits.picture.contains(at) {
            let (cw, ch) = self.picture.cell();
            let area = self.hits.picture;
            let (x, y) = ((at.x - area.x) as f64 * cw + cw / 2.0, (at.y - area.y) as f64 * ch + ch / 2.0);
            if let Some(week) = &self.week {
                let layout = Layout::new(week, &self.clock, area.width as f64 * cw, area.height as f64 * ch, (cw, ch));
                if let Some(id) = layout.at(x, y) {
                    self.selected = Some(id.to_string());
                }
            }
        } else if let Some((_, id)) = self.hits.titles.iter().find(|(r, _)| r.contains(at)) {
            let id = id.clone();
            self.selected = Some(id.clone());
            self.run(Cmd::Open(id));
        } else if let Some((_, id)) = self.hits.rows.iter().find(|(r, _)| r.contains(at)) {
            self.selected = Some(id.clone());
        }
    }

    fn scroll_by(&mut self, lines: isize) {
        self.scroll = (self.scroll as isize + lines).max(0) as usize;
        self.follow = false;
    }
}
