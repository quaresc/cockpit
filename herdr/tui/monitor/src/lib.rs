//! The monitor space: how production does after each release, from ~/.cache/cockpit/monitor.json as the hub pushes
//! it: every app's latest deploy, monitors and error and latency sparklines, the releases of the last day with their
//! verdict, and the actions to investigate, fix, file a bug or roll back.

pub mod app;
pub mod data;
mod keys;
pub mod pinned;
pub mod rows;
mod view;

use std::time::Duration;

use crossterm::event::Event;
use ratatui::Frame;

/// `cockpit monitor`: production in the terminal until it quits.
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

    fn tick(&mut self) -> bool {
        app::App::tick(self)
    }

    /// A little faster while an action is on its way or a toast shows, every second otherwise.
    fn timeout(&self) -> Duration {
        if self.is_busy() { Duration::from_millis(100) } else { Duration::from_secs(1) }
    }
}
