//! todo.py in Rust: the Markdown checkbox list in ~/.config/cockpit/todo.md, open items first and done ones folded,
//! added and edited in place, filtered, marked several at a time, and read again whenever another process writes it.

pub mod app;
pub mod doc;
mod keys;
pub mod links;
pub mod pinned;
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

    fn tick(&mut self) -> bool {
        app::App::tick(self)
    }

    /// Fast while a flash plays; otherwise the watcher's word is picked up within half a second.
    fn timeout(&self) -> Duration {
        if self.is_animating() { Duration::from_millis(16) } else { Duration::from_millis(500) }
    }
}
