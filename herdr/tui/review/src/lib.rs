//! The review space: the changes in an agent's worktree in one of three scopes (uncommitted, the branch against its
//! base, its pull request), commented line by line and sent to that agent as one message, or read as a walkthrough.

pub mod app;
mod keys;
pub mod pinned;
pub mod subject;
mod view;
pub mod walk;

use std::time::Duration;

use crossterm::event::Event;
use ratatui::Frame;

/// `cockpit review`: the review space in the terminal until it quits.
pub fn run() -> std::io::Result<()> {
    let mut app = app::App::start();
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
        let busy = self.dialog.is_some() || self.palette.is_open || self.diff().is_typing();
        (!busy).then(|| ("review".to_string(), self.acts().into_iter().map(|(act, _)| act).collect()))
    }

    fn tick(&mut self) -> bool {
        app::App::tick(self)
    }

    /// Quick while a call is on its way, a toast shows or a walkthrough being written spins; twice a second otherwise.
    fn timeout(&self) -> Duration {
        if self.is_busy() { Duration::from_millis(100) } else { Duration::from_millis(500) }
    }
}
