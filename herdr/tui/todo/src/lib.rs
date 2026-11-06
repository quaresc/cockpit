//! The to-do space: the Markdown checkbox list in ~/.config/cockpit/todo.md, open items first and done ones folded,
//! added and edited in place, filtered, marked several at a time, and read again whenever another process writes it.

pub mod app;
pub mod doc;
pub mod gui;
mod keys;
pub mod links;
pub mod pinned;
mod view;

/// `cockpit todo`: the to-do list in the terminal until it quits.
pub fn run() -> std::io::Result<()> {
    cockpit_core::term::run(&mut app::App::start())
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

    /// Idle only: `?` must type into the filter, an edit or a dialog when one of them is open.
    fn help(&self) -> Option<(String, Vec<cockpit_core::acts::Act>)> {
        let busy = self.filtering || self.editing.is_some() || self.dialog.is_some() || self.palette.is_open;
        (!busy).then(|| ("to-do".to_string(), self.acts().into_iter().map(|(act, _)| act).collect()))
    }
    fn tick(&mut self) -> bool {
        app::App::tick(self)
    }

    /// Fast while a flash plays; otherwise the watcher's word is picked up within half a second.
    fn timeout(&self) -> Duration {
        if self.is_animating() { Duration::from_millis(16) } else { Duration::from_millis(500) }
    }
}
