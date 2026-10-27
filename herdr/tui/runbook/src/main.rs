//! runbook.py in Rust: a tab per runbook with its progress, its steps as rows whose cells wrap, the selected step's
//! actions under them, and the steps that turn done spinning, flashing and fading one after the other.

mod acts;
mod anim;
mod app;
mod book;
mod handoff;
mod keys;
mod pinned;
mod rows;
mod view;

use std::time::Duration;

use crossterm::event::Event;
use ratatui::Frame;

impl cockpit_core::term::App for app::App {
    fn draw(&mut self, frame: &mut Frame) {
        app::App::draw(self, frame);
    }

    fn event(&mut self, event: Event) -> bool {
        app::App::event(self, event)
    }

    fn tick(&mut self) {
        app::App::tick(self);
    }

    /// Fast while something moves, every second otherwise, a little faster while a bridge call or refresh runs.
    fn timeout(&self) -> Duration {
        if self.is_animating() {
            Duration::from_millis(16)
        } else if self.is_busy() {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(1)
        }
    }
}

fn main() -> std::io::Result<()> {
    cockpit_core::term::run(&mut app::App::new())
}
