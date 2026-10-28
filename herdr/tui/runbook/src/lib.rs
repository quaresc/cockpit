//! The runbook space: a tab per runbook with its progress, its steps as rows whose cells wrap, the selected step's
//! actions under them or at its end, and the steps that turn done spinning, flashing and fading one after the other.
//! Steps can be filtered as you type, picked for a batch, and edited or added straight into the runbook file.

pub mod actions;
pub mod acts;
pub mod anim;
pub mod app;
pub mod book;
pub mod edit;
pub mod filter;
pub mod handoff;
pub mod keys;
pub mod pinned;
pub mod rows;
pub mod steps;
pub mod table;
pub mod view;

/// `cockpit runbook`: the runbooks in the terminal until it quits.
pub fn run() -> std::io::Result<()> {
    cockpit_core::term::run(&mut app::App::new())
}

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

    fn tick(&mut self) -> bool {
        app::App::tick(self)
    }

    /// Fast while something moves, a little faster while a bridge call or refresh runs, every second otherwise.
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
