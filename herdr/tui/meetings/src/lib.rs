//! The meetings space: the week from ~/.cache/cockpit/meetings.json grouped by day, the running meeting highlighted and
//! the next one about to start pulsing, briefs prepared and folded open, and the same week drawn as a picture.

pub mod app;
mod keys;
pub mod picture;
pub mod pinned;
pub mod timeline;
mod view;
pub mod week;

/// `cockpit meetings`: the week in the terminal until it quits.
pub fn run() -> std::io::Result<()> {
    picture::warm_up();
    // Asked before the screen is taken: the terminal's graphics protocol and font size, else half blocks.
    let options = QueryStdioOptions { kitty_compression: true, ..Default::default() };
    let mut picker = Picker::from_query_stdio_with_options(options).unwrap_or_else(|_| Picker::halfblocks());
    picker.set_background_color(Some(image::Rgba([16, 16, 16, 255])));
    let mut app = app::App::start(picker);
    app.report();
    cockpit_core::term::run(&mut app)
}

use std::time::Duration;

use crossterm::event::Event;
use ratatui::Frame;
use ratatui_image::picker::Picker;
use ratatui_image::picker::cap_parser::QueryStdioOptions;

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

    /// Fast while a pulse plays, a little faster while an action or a picture is on its way, every second otherwise.
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
