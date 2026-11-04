//! The mail space: the messages that need me (from a person, or marked important) above the updates (bots,
//! notifications, newsletters), the selected one read whole in a preview, opened in the side browser or the mail client,
//! answered by an agent, archived or marked read.

pub mod app;
pub mod gui;
pub mod html;
mod keys;
pub mod model;
pub mod pinned;
pub mod thread;
mod view;

use std::time::Duration;

use crossterm::event::Event;
use ratatui::Frame;

/// `cockpit mail`: the mail in the terminal until it quits.
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

    fn tick(&mut self) -> bool {
        app::App::tick(self)
    }

    /// A little faster while an action or a thread is on its way, every second otherwise.
    fn timeout(&self) -> Duration {
        if self.is_busy() { Duration::from_millis(200) } else { Duration::from_secs(1) }
    }
}
