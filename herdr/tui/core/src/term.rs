//! The terminal loop every app runs: alternate screen and mouse on, a frame drawn after each event and after a tick that
//! changed something (so an idle app costs nothing), everything restored on the way out, a panic included.

use std::io::stdout;
use std::time::Duration;

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::execute;
use ratatui::{DefaultTerminal, Frame};

pub trait App {
    fn draw(&mut self, frame: &mut Frame);
    /// A key, mouse or resize event; true quits.
    fn event(&mut self, event: Event) -> bool;
    /// Called when no event came within `timeout`: files to read again, animations to step. True when the screen changed
    /// and needs drawing again.
    fn tick(&mut self) -> bool;
    /// How long to wait for an event before the next tick: short while something animates.
    fn timeout(&self) -> Duration;
}

pub fn run(app: &mut impl App) -> std::io::Result<()> {
    let mut terminal: DefaultTerminal = ratatui::init();
    let _ = execute!(stdout(), EnableMouseCapture);
    let result = (|| -> std::io::Result<()> {
        let mut redraw = true;
        loop {
            if redraw {
                terminal.draw(|frame| app.draw(frame))?;
            }
            redraw = if event::poll(app.timeout())? {
                if app.event(event::read()?) {
                    return Ok(());
                }
                true
            } else {
                app.tick()
            };
        }
    })();
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}
