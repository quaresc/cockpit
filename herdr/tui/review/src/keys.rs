//! Keys and clicks: the palette, the dialog (the worktree picker, leaving with comments unsent) and a comment being
//! typed first, then the space's own keys (the shared verbs on their keys), then the diff's (moving, folding,
//! commenting); the `?` overlay and the palette list the actions the state in view offers.

use cockpit_core::acts::Act;
use cockpit_core::dialog::Answer;
use cockpit_core::keys::{self, Move};
use cockpit_core::palette::{Entry, Outcome};
use cockpit_show::row::Act as RowAct;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};

use crate::app::{App, Ask, Cmd, Mode, Scope};
use crate::walk;

impl App {
    /// The actions the state in view offers, each on its key: what `?` lists and the palette offers first.
    pub fn acts(&self) -> Vec<(Act, Cmd)> {
        let mut acts = Vec::new();
        if self.subject().is_none() {
            acts.push((Act::new("refresh", 'R', "refresh"), Cmd::Refresh));
            return acts;
        }
        let here = self.here();
        let status = walk::status(&here.walkthrough);
        let has_files = here.error.is_none() && !here.diff.files().is_empty();
        let walking = self.mode == Mode::Walk && has_files;
        if walking && status != "done" {
            match status {
                "running" => acts.push((Act::new("stop", 's', "stop").main(), Cmd::Stop)),
                _ => {
                    acts.push((Act::new("run", 'r', "run").main(), Cmd::Write));
                    acts.push((Act::new("model", 'm', "model"), Cmd::Model));
                }
            }
        } else if has_files {
            let on_draft = here.diff.draft_here().is_some();
            acts.push((Act::new("comment", 'c', "comment").main(), Cmd::Comment));
            if on_draft {
                acts.push((Act::new("edit", 'e', "edit"), Cmd::Edit));
                acts.push((Act::new("delete", 'D', "delete"), Cmd::Delete));
            }
            if !here.diff.notes().is_empty() {
                acts.push((Act::new("send", 'S', "send"), Cmd::Send));
            }
            if here.diff.can_undo() {
                acts.push((Act::new("undo", 'u', "undo"), Cmd::Undo));
            }
            acts.push((Act::new("next hunk", 'n', "next"), Cmd::Hunk(true)));
            acts.push((Act::new("previous hunk", 'p', "previous"), Cmd::Hunk(false)));
            acts.push((Act::new(if here.diff.side_by_side() { "unified" } else { "side by side" }, 's', "split"), Cmd::SideBySide));
            acts.push((Act::new("fold all", 'z', "fold"), Cmd::FoldAll));
            if walking {
                acts.push((Act::new("next step", 'l', "next step"), Cmd::Step(1)));
                acts.push((Act::new("previous step", 'h', "previous step"), Cmd::Step(-1)));
                if here.walkthrough["fresh"] == false {
                    acts.push((Act::new("redo", 'r', "redo"), Cmd::Write));
                }
                acts.push((Act::new("copy", 'y', "copy"), Cmd::Copy));
            }
        }
        // Diff ↔ walkthrough is this space's view: back to the diff stays offered when the scope has nothing to show.
        if has_files || self.mode == Mode::Walk {
            acts.push((Act::new("switch view", 'w', "walk"), Cmd::Walk));
        }
        acts.push((Act::new("uncommitted", '1', "scope"), Cmd::Scope(Scope::Uncommitted)));
        acts.push((Act::new("branch", '2', "scope"), Cmd::Scope(Scope::Branch)));
        acts.push((Act::new("pull request", '3', "scope"), Cmd::Scope(Scope::Pr)));
        if here.about["pr"]["url"].is_string() {
            acts.push((Act::new("open", 'o', "open"), Cmd::OpenPr));
        }
        acts.push((Act::new("send to", 'T', "to"), Cmd::ToPicker));
        acts.push((Act::new("worktree", 'W', "switch"), Cmd::Switch));
        acts.push((Act::new("refresh", 'R', "refresh"), Cmd::Refresh));
        acts
    }

    /// The actions in view first, then the other agents' worktrees and what has no key.
    fn palette_entries(&mut self) -> Vec<Entry> {
        let mut cmds = Vec::new();
        let mut entries = Vec::new();
        for (act, cmd) in self.acts() {
            let key = act.key.map(|k| k.to_string()).unwrap_or_default();
            let help = match cmd {
                Cmd::Walk if self.mode == Mode::Walk => format!("{key} · back to the diff"),
                Cmd::Walk => format!("{key} · the walkthrough"),
                Cmd::Switch => format!("{key} · another agent's worktree"),
                Cmd::ToPicker => format!("{key} · an agent of this worktree, or a new one"),
                _ => key,
            };
            entries.push(Entry { text: act.label.clone(), help, featured: true, id: cmds.len() });
            cmds.push(cmd);
        }
        if self.here().diff.has_drafts() {
            entries.push(Entry { text: "clear the comments".into(), help: "not sent yet; u brings them back".into(), featured: false, id: cmds.len() });
            cmds.push(Cmd::Clear);
        }
        if walk::status(&self.here().walkthrough) == "done" {
            entries.push(Entry { text: "copy the walkthrough as Markdown".into(), help: "y".into(), featured: false, id: cmds.len() });
            cmds.push(Cmd::Copy);
        }
        for agent in self.agents.clone().unwrap_or_default() {
            entries.push(Entry {
                text: format!("review  {}", agent.label()),
                help: "its worktree, the comments to it".into(),
                featured: false,
                id: cmds.len(),
            });
            cmds.push(Cmd::Pick(agent.terminal));
        }
        self.cmds = cmds;
        entries
    }

    pub fn event(&mut self, event: Event) -> bool {
        self.dirty = true;
        match event {
            Event::FocusGained | Event::FocusLost => {
                self.focus(event == Event::FocusGained);
                false
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => self.key(key),
            Event::Mouse(mouse) => {
                let modal = self.palette.is_open || self.dialog.is_some();
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) if !modal => {
                        self.pin();
                        self.click(mouse.column, mouse.row);
                    }
                    MouseEventKind::ScrollDown => (self.scroll, self.reveal) = (self.scroll + 3, false),
                    MouseEventKind::ScrollUp => (self.scroll, self.reveal) = (self.scroll.saturating_sub(3), false),
                    _ => {}
                }
                false
            }
            _ => false,
        }
    }

    /// A key; true quits. Leaving with comments not sent asks first; ctrl+c again while it asks leaves anyway.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && keys::is_quit(&key) {
            return matches!(self.dialog, Some((_, Ask::Leave))) || self.may_leave();
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
        if let Some((dialog, _)) = &mut self.dialog {
            let Some(answer) = dialog.key(key) else { return false };
            let (_, ask) = self.dialog.take().expect("a dialog is open");
            match (ask, answer) {
                (Ask::Pick(handles), Answer::Picked(i)) => {
                    if let Some(handle) = handles.get(i) {
                        self.run(Cmd::Pick(handle.clone()));
                    }
                }
                (Ask::To(targets), Answer::Picked(i)) => {
                    if let Some(to) = targets.get(i) {
                        self.run(Cmd::To(to.clone()));
                    }
                }
                (Ask::Leave, Answer::Yes) => return true,
                _ => {}
            }
            return false;
        }
        if self.diff().is_typing() {
            let _ = self.here_mut().diff.key(key);
            self.reveal = true;
            return false;
        }
        if ctrl {
            match key.code {
                KeyCode::Char('p') => {
                    let entries = self.palette_entries();
                    self.palette.open(entries);
                }
                KeyCode::Char('d') => self.page(1),
                KeyCode::Char('u') => self.page(-1),
                _ => {}
            }
            return false;
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        if keys::is_quit(&key) {
            return self.may_leave();
        }
        let acts = self.acts();
        if let KeyCode::Char(c) = key.code
            && let Some((_, cmd)) = acts.iter().find(|(a, _)| a.key == Some(c))
        {
            self.run(cmd.clone());
            return false;
        }
        let has_files = self.here().error.is_none() && !self.here().diff.files().is_empty();
        let walking = self.mode == Mode::Walk && has_files;
        let status = walk::status(&self.here().walkthrough);
        match key.code {
            KeyCode::Right if walking => self.run(Cmd::Step(1)),
            KeyCode::Left if walking => self.run(Cmd::Step(-1)),
            KeyCode::Char('S') => self.send(),
            KeyCode::Tab => self.set_scope(Scope::ALL[(self.scope.index() + 1) % 3]),
            KeyCode::BackTab => self.set_scope(Scope::ALL[(self.scope.index() + 2) % 3]),
            KeyCode::Enter if walking && matches!(status, "none" | "failed") => self.run(Cmd::Write),
            _ if self.mode == Mode::Walk && status != "done" => {}
            _ => match keys::movement(&key) {
                Some(Move::Pages(n)) => self.page(n),
                _ if !has_files => {}
                _ => {
                    self.pin();
                    let _ = self.here_mut().diff.key(key);
                    self.reveal = true;
                }
            },
        }
        false
    }

    /// Moves the cursor a page of lines down (positive) or up.
    fn page(&mut self, by: isize) {
        let lines = (self.height.saturating_sub(2)).max(1) as isize;
        self.here_mut().diff.step(by * lines);
        self.reveal = true;
    }

    fn click(&mut self, x: u16, y: u16) {
        if let Some(scope) = self.hits.tabs.iter().find(|(r, _)| r.contains((x, y).into())).map(|(_, s)| *s) {
            return self.set_scope(scope);
        }
        let Some((_, left, clicks)) = self.hits.rows.iter().find(|(row, _, _)| *row == y) else { return };
        let col = x.saturating_sub(*left) as usize;
        let Some(act) = clicks.iter().find(|(a, b, _)| col >= *a && col < *b).map(|(_, _, act)| act.clone()) else { return };
        match act {
            RowAct::Pick(step) => {
                let at = self.here().step as isize;
                self.run(Cmd::Step(step as isize - at));
            }
            RowAct::Open(url) => cockpit_core::files::open_url(&url),
            act => {
                self.here_mut().diff.act(&act);
                self.reveal = true;
            }
        }
    }
}
