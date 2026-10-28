//! Keys and clicks: the filter field, picking rows, the highlighted row's actions (or the picked rows' shared ones,
//! asked once for all), the palette, and the dialog's answers.

use cockpit_core::dialog::{Answer, Dialog};
use cockpit_core::hub;
use cockpit_core::input::{Edit, Input};
use cockpit_core::keys::{self, Move};
use cockpit_core::palette::{Entry, Outcome};
use cockpit_core::toast::Kind as Toast;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::app::{App, Asked, Command};
use crate::row::Call;
use crate::select;

impl App {
    /// A key; true quits.
    pub(crate) fn key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some((dialog, _)) = &mut self.dialog {
            if let Some(answer) = dialog.key(key) {
                self.answer(answer);
            }
            return false;
        }
        if self.palette.is_open {
            match self.palette.key(key) {
                Outcome::Chosen(id) => self.run_command(id),
                Outcome::Elsewhere(target) => hub::go(&target),
                Outcome::Open | Outcome::Closed => {}
            }
            return false;
        }
        if self.editing {
            return self.filter_key(key);
        }
        if keys::is_quit(&key) {
            return true;
        }
        match key.code {
            KeyCode::Char('p') if ctrl => self.open_palette(),
            // ctrl+o is not o: a chord must not open, mute or quit.
            KeyCode::Char(_) if ctrl || key.modifiers.contains(KeyModifiers::ALT) => {}
            KeyCode::Char('/') => self.start_filter(),
            KeyCode::Esc if !self.picked.is_empty() => self.picked.clear(),
            KeyCode::Esc if self.tree.filtering() => self.clear_filter(),
            KeyCode::Char('v') if self.pick() => {}
            KeyCode::Char('V') => {
                if let Some(node) = self.tree.current() {
                    self.picked.same_kind(&self.tree, node);
                }
            }
            KeyCode::Char(c) if self.act_key(c) => {}
            KeyCode::Char('w') => self.regroup(),
            KeyCode::Char('R') => self.refresh(),
            KeyCode::Char(' ') => {
                if let Some(node) = self.tree.current() {
                    self.tree.toggle(node);
                }
            }
            KeyCode::Enter => self.primary(),
            _ => self.move_key(key),
        }
        false
    }

    /// The shared moves in the tree, or the arrows and pages while typing a filter.
    fn move_key(&mut self, key: KeyEvent) {
        match keys::movement(&key) {
            Some(Move::By(n)) => self.tree.step(n),
            Some(Move::Pages(n)) => self.tree.step(n * self.tree.page()),
            Some(Move::Top) => self.tree.step(isize::MIN / 2),
            Some(Move::Bottom) => self.tree.step(isize::MAX / 2),
            Some(Move::Group(_)) | None => {}
        }
    }

    fn start_filter(&mut self) {
        self.filter = Input::new(&self.filter.text);
        self.editing = true;
    }

    /// Typing narrows the tree live; enter keeps the filter, esc clears it, the arrows still move.
    fn filter_key(&mut self, key: KeyEvent) -> bool {
        match self.filter.key(key) {
            Edit::Changed => self.tree.set_filter(&self.filter.text.clone()),
            Edit::Submit if self.filter.text.trim().is_empty() => self.clear_filter(),
            Edit::Submit => self.editing = false,
            Edit::Cancel => self.clear_filter(),
            Edit::Ignored if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) => return true,
            Edit::Ignored => {
                if !matches!(key.code, KeyCode::Char(_)) {
                    self.move_key(key);
                }
            }
        }
        false
    }

    /// Picks or drops the highlighted row; false when it cannot be picked.
    fn pick(&mut self) -> bool {
        self.tree.current().is_some_and(|node| self.picked.toggle(&self.tree, node))
    }

    /// Runs a row's action: a choice asks which, a change asks first, the rest runs at once.
    pub(crate) fn act(&mut self, node: usize, n: usize) {
        let row = &self.tree.nodes[node].row;
        let Some(act) = row.acts.get(n) else { return };
        if !act.picks.is_empty() {
            let dialog = Dialog::Pick { question: format!("{}: {}", act.act.label, row.name), options: act.act.choices.clone(), selected: 0 };
            self.dialog = Some((dialog, Asked::Picks(act.picks.clone())));
        } else if let (Some(key), Some(call)) = (&act.act.confirm, &act.call) {
            let dialog = Dialog::Confirm { question: format!("{}: {}?", act.act.label, row.name) };
            self.dialog = Some((dialog, Asked::Armed(call.clone(), key.clone())));
        } else if let Some(call) = act.call.clone() {
            self.call(call, None);
        }
    }

    /// The picked rows' shared action `n`: asked once for all of them, then run on each.
    fn batch(&mut self, n: usize) {
        let rows = self.picked.rows(&self.tree);
        let Some(shared) = select::shared(&self.tree, &rows).get(n).cloned() else { return };
        let same = |a: &crate::row::RowAct| a.act.label == shared.label && a.act.key == shared.key;
        let acts: Vec<&crate::row::RowAct> = rows.iter().filter_map(|&r| self.tree.nodes[r].row.acts.iter().find(|a| same(a))).collect();
        if !shared.choices.is_empty() {
            let question = format!("{}: {}", shared.label, select::describe(&self.tree, &rows));
            let picks = acts.iter().map(|a| a.picks.clone()).collect();
            self.dialog = Some((Dialog::Pick { question, options: shared.choices.clone(), selected: 0 }, Asked::BatchPicks(shared.label.clone(), picks)));
        } else {
            let calls = acts.iter().filter_map(|a| a.call.clone().map(|c| (c, a.act.confirm.clone()))).collect();
            let question = select::question(&self.tree, &rows, &shared.label);
            self.dialog = Some((Dialog::Confirm { question }, Asked::Batch(shared.label.clone(), calls)));
        }
    }

    fn answer(&mut self, answer: Answer) {
        let Some((_, asked)) = self.dialog.take() else { return };
        match (answer, asked) {
            (Answer::Yes, Asked::Armed(call, key)) => self.call(call, Some(key)),
            (Answer::Picked(n), Asked::Picks(calls)) => {
                if let Some(call) = calls.get(n) {
                    self.call(call.clone(), None);
                }
            }
            (Answer::Yes, Asked::Batch(label, calls)) => self.run_batch(&label, calls),
            (Answer::Picked(n), Asked::BatchPicks(label, picks)) => {
                let calls = picks.into_iter().filter_map(|p| p.get(n).cloned().map(|c| (c, None))).collect();
                self.run_batch(&label, calls);
            }
            _ => {}
        }
    }

    fn run_batch(&mut self, label: &str, calls: Vec<(Call, Option<String>)>) {
        let count = calls.len();
        for (call, armed) in calls {
            self.call(call, armed);
        }
        self.toasts.push(format!("{label}: {count} sent"), Toast::Note);
        self.picked.clear();
    }

    /// Enter: the row's main action, else what a click on it does, else its fold.
    fn primary(&mut self) {
        let Some(node) = self.tree.current() else { return };
        let row = &self.tree.nodes[node].row;
        if let Some(n) = row.acts.iter().position(|a| a.act.main) {
            self.act(node, n);
            return;
        }
        match row.primary.clone() {
            Some(call) => self.call(call, None),
            None => self.tree.toggle(node),
        }
    }

    fn regroup(&mut self) {
        self.by_status = !self.by_status;
        self.rebuild();
    }

    fn refresh(&mut self) {
        self.call(crate::row::call("request_refresh", vec![]), None);
    }

    /// Every row's actions and a "go to" for each, then refresh, regroup and filter; the main actions show before typing.
    fn open_palette(&mut self) {
        let mut entries = Vec::new();
        self.commands.clear();
        let mut push = |commands: &mut Vec<Command>, text: String, help: String, featured: bool, command: Command| {
            entries.push(Entry { text, help, featured, id: commands.len() });
            commands.push(command);
        };
        for node in &self.tree.nodes {
            let row = &node.row;
            for (n, act) in row.acts.iter().enumerate() {
                push(&mut self.commands, format!("{}  {}", act.act.label, row.name), row.section.clone(), act.act.main, Command::Act(node.path.clone(), n));
            }
            if !row.name.is_empty() {
                push(&mut self.commands, format!("go to  {}", row.name), row.section.clone(), false, Command::GoTo(node.path.clone()));
            }
        }
        push(&mut self.commands, "refresh the inbox".into(), "asks the mod for fresh data".into(), true, Command::Refresh);
        let regroup = format!("group work by {}", if self.by_status { "feature" } else { "status" });
        push(&mut self.commands, regroup, "w".into(), true, Command::Regroup);
        push(&mut self.commands, "filter rows".into(), "/".into(), true, Command::Filter);
        self.palette.open(entries);
    }

    fn run_command(&mut self, id: usize) {
        match self.commands.get(id) {
            Some(Command::Act(path, n)) => {
                let n = *n;
                if let Some(node) = self.tree.node(&path.clone()) {
                    self.act(node, n);
                }
            }
            Some(Command::GoTo(path)) => {
                if let Some(node) = self.tree.node(&path.clone()) {
                    if self.tree.filtering() {
                        self.clear_filter();
                    }
                    self.tree.go_to(node);
                }
            }
            Some(Command::Refresh) => self.refresh(),
            Some(Command::Regroup) => self.regroup(),
            Some(Command::Filter) => self.start_filter(),
            None => {}
        }
    }

    /// The action bound to this key run: the picked rows' shared one, else the highlighted row's; false when none is.
    fn act_key(&mut self, c: char) -> bool {
        if !self.picked.is_empty() {
            let rows = self.picked.rows(&self.tree);
            let Some(n) = select::shared(&self.tree, &rows).iter().position(|a| a.key == Some(c)) else { return false };
            self.batch(n);
            return true;
        }
        let Some(node) = self.tree.current() else { return false };
        let Some(n) = self.tree.nodes[node].row.acts.iter().position(|a| a.act.key == Some(c)) else { return false };
        self.act(node, n);
        true
    }

    /// A button pressed: the picked rows' shared action, else the highlighted row's.
    fn press(&mut self, n: usize) {
        if !self.picked.is_empty() {
            self.batch(n);
        } else if let Some(node) = self.tree.current() {
            self.act(node, n);
        }
    }

    pub(crate) fn mouse(&mut self, mouse: MouseEvent) {
        if self.dialog.is_some() || self.palette.is_open {
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollDown => self.tree.scroll(3),
            MouseEventKind::ScrollUp => self.tree.scroll(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(&(_, n)) = self.buttons.iter().find(|(r, _)| r.contains((mouse.column, mouse.row).into())) {
                    self.press(n);
                    return;
                }
                if let Some((node, on_glyph)) = self.tree.at(mouse.column, mouse.row) {
                    self.tree.select(node);
                    let row = &self.tree.nodes[node].row;
                    if on_glyph || (row.primary.is_none() && !self.tree.nodes[node].kids.is_empty()) {
                        self.tree.toggle(node);
                    }
                }
            }
            _ => {}
        }
    }
}
