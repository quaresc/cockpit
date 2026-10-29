//! The list's rows: the routines, running and failed ones first, each unfolded onto its past runs; each row's actions,
//! and the keys its marker stands for.

use cockpit_core::acts::Act;

use crate::app::{App, Cmd};
use crate::data::{Doc, Routine, State};

/// What a row stands for: a routine by its slug, or one of its runs by its id.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Routine(String),
    Run(String, String),
}

impl Item {
    pub fn slug(&self) -> &str {
        match self {
            Item::Routine(slug) | Item::Run(slug, _) => slug,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub key: String,
    pub item: Item,
    pub depth: usize,
    /// The guide drawn before a run's row: "├ " or "└ ".
    pub guide: &'static str,
    /// A routine with runs, and whether they show.
    pub fold: Option<bool>,
}

/// The routines in the order they show: running or waiting, then failed or broken, then by name.
pub fn ordered(doc: &Doc) -> Vec<&Routine> {
    let rank = |r: &Routine| match doc.state(r) {
        State::Running | State::Waiting => 0,
        State::Failed | State::Broken | State::Over => 1,
        State::Paused => 3,
        _ => 2,
    };
    let mut all: Vec<&Routine> = doc.routines.iter().collect();
    all.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    all
}

impl App {
    pub fn rows(&self) -> Vec<Row> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let mut rows = Vec::new();
        for r in ordered(doc) {
            let runs = doc.runs(&r.slug);
            let open = self.opened.contains(&r.slug);
            rows.push(Row { key: r.slug.clone(), item: Item::Routine(r.slug.clone()), depth: 0, guide: "", fold: (!runs.is_empty()).then_some(open) });
            if open {
                let last = runs.len().saturating_sub(1);
                for (n, run) in runs.iter().enumerate() {
                    let item = Item::Run(r.slug.clone(), run.id.clone());
                    rows.push(Row { key: format!("{}/run/{}", r.slug, run.id), item, depth: 1, guide: if n == last { "└ " } else { "├ " }, fold: None });
                }
            }
        }
        rows
    }

    /// The keys a row's marker stands for: a routine's own, which its runs' events carry.
    pub fn marker_keys(&self, row: &Row) -> Vec<String> {
        vec![row.item.slug().to_string()]
    }

    pub fn is_marked(&self, row: &Row) -> bool {
        row.depth == 0 && self.marker_keys(row).iter().any(|key| self.looked.is_marked(key))
    }

    pub fn selected_row(&self) -> Option<Row> {
        let key = self.selected.as_ref()?;
        self.rows().into_iter().find(|r| &r.key == key)
    }

    pub fn move_sel(&mut self, by: isize) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|k| rows.iter().position(|r| &r.key == k)).unwrap_or(0) as isize;
        self.selected = Some(rows[(at + by).clamp(0, rows.len() as isize - 1) as usize].key.clone());
        self.follow = true;
    }

    pub fn select_edge(&mut self, last: bool) {
        let rows = self.rows();
        let edge = if last { rows.last() } else { rows.first() };
        self.selected = edge.map(|r| r.key.clone());
        self.follow = true;
    }

    /// The previous or next routine's row.
    pub fn move_group(&mut self, by: isize) {
        let rows = self.rows();
        let at = self.selected.as_ref().and_then(|k| rows.iter().position(|r| &r.key == k)).unwrap_or(0);
        let heads: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.depth == 0).map(|(i, _)| i).collect();
        let next = if by > 0 { heads.iter().find(|&&i| i > at) } else { heads.iter().rev().find(|&&i| i < at) };
        if let Some(&i) = next {
            self.selected = Some(rows[i].key.clone());
            self.follow = true;
        }
    }

    /// The row's actions in order, the main one marked; empty for a row whose data is gone.
    pub fn acts_for(&self, item: &Item) -> Vec<(Act, Cmd)> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let Some(r) = doc.routine(item.slug()) else { return Vec::new() };
        let slug = r.slug.clone();
        let mut acts = Vec::new();
        match item {
            Item::Routine(_) => {
                let state = doc.state(r);
                if matches!(state, State::Running | State::Waiting) {
                    acts.push((Act::new("stop", 's', "stop").main(), Cmd::Stop(slug.clone())));
                } else if state != State::Broken {
                    acts.push((Act::new("run", 'r', "run").main(), Cmd::Run(slug.clone())));
                }
                acts.push((Act::new("edit", 'e', "edit"), Cmd::Edit(slug.clone())));
                if let Some(last) = doc.last(&slug) {
                    acts.push((Act::new("open", 'o', "open"), Cmd::Output(slug.clone(), None)));
                    if last.session().is_some() {
                        acts.push((Act::new("talk", 't', "talk"), Cmd::Talk(slug.clone(), None)));
                    }
                }
                acts.push((Act::new(if r.paused { "resume" } else { "pause" }, 'p', "pause"), Cmd::Pause(slug.clone(), !r.paused)));
                if r.run != "shell" {
                    let models = cockpit_core::routines::MODELS;
                    let at = models.iter().position(|m| m.eq_ignore_ascii_case(&r.model)).unwrap_or(0);
                    let next = models[(at + 1) % models.len()];
                    let label = format!("model {}", if r.model.is_empty() { "default" } else { &r.model });
                    acts.push((Act { label, key: Some('m'), id: "model".into(), ..Default::default() }, Cmd::Model(slug.clone(), next.into())));
                }
                acts.push((Act::new("delete", 'D', "delete").confirm(&format!("delete-routine-{slug}")), Cmd::Delete(slug)));
            }
            Item::Run(_, id) => {
                let Some(run) = doc.run(&slug, id) else { return Vec::new() };
                acts.push((Act::new("open", 'o', "open").main(), Cmd::Output(slug.clone(), Some(id.clone()))));
                if run.session().is_some() {
                    acts.push((Act::new("talk", 't', "talk"), Cmd::Talk(slug.clone(), Some(id.clone()))));
                }
                acts.push((Act::new("run", 'r', "run"), Cmd::Run(slug)));
            }
        }
        acts
    }

    pub fn acts(&self) -> Vec<(Act, Cmd)> {
        self.selected_row().map(|r| self.acts_for(&r.item)).unwrap_or_default()
    }
}
