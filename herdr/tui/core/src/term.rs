//! The terminal loop every app runs: alternate screen and mouse on, a frame drawn after each event or tick, everything
//! restored on the way out, a panic included.

use std::io::stdout;
use std::time::Duration;

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::execute;
use ratatui::{DefaultTerminal, Frame};

pub trait App {
    fn draw(&mut self, frame: &mut Frame);
    /// A key, mouse or resize event; true quits.
    fn event(&mut self, event: Event) -> bool;
    /// Called when no event came within `timeout`: files to read again, animations to step.
    fn tick(&mut self);
    /// How long to wait for an event before the next tick: short while something animates.
    fn timeout(&self) -> Duration;
}

pub fn run(app: &mut impl App) -> std::io::Result<()> {
    let mut terminal: DefaultTerminal = ratatui::init();
    let _ = execute!(stdout(), EnableMouseCapture);
    let result = (|| -> std::io::Result<()> {
        loop {
            terminal.draw(|frame| app.draw(frame))?;
            if event::poll(app.timeout())? {
                if app.event(event::read()?) {
                    return Ok(());
                }
            } else {
                app.tick();
            }
        }
    })();
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}
