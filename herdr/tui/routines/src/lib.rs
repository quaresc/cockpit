//! The routines space: the routines in ~/.config/cockpit/routines/ as the hub pushes them, when each runs next, its runs
//! and what the last one said; add one in a form, run, stop, pause, edit or delete it, and talk an agent's run over.

pub mod app;
pub mod data;
pub mod form;
pub mod gui;
mod keys;
pub mod pinned;
pub mod rows;
mod view;

use std::time::Duration;

use crossterm::event::Event;
use ratatui::Frame;

/// `cockpit routines`: the routines in the terminal until it quits.
pub fn run() -> std::io::Result<()> {
    let mut app = app::App::start();
    app.report();
    cockpit_core::term::run(&mut app)
}

impl cockpit_core::term::App for app::App {
    fn draw(&mut self, frame: &mut Frame) {
        app::App::draw(self, frame);
    }

    fn event(&mut self, event: Event) -> bool {
        app::App::event(self, event)
    }

    fn help(&self) -> Option<(String, Vec<cockpit_core::acts::Act>)> {
        let busy = self.dialog.is_some() || self.palette.is_open || self.form.is_some();
        (!busy).then(|| ("routines".to_string(), self.acts().into_iter().map(|(act, _)| act).collect()))
    }
    fn tick(&mut self) -> bool {
        app::App::tick(self)
    }

    /// A little faster while a routine runs, an action is on its way or a toast shows, every second otherwise.
    fn timeout(&self) -> Duration {
        if self.is_busy() { Duration::from_millis(100) } else { Duration::from_secs(1) }
    }
}
