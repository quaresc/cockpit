//! The terminal loop every app runs: alternate screen, mouse and focus reports on (herdr sends a pane its focus going in
//! and out), a frame drawn after each event and after a tick that changed something (so an idle app costs nothing),
//! everything restored on the way out, a panic included.

use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::event::{self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event};
use crossterm::execute;
use ratatui::{DefaultTerminal, Frame};

pub trait App {
    fn draw(&mut self, frame: &mut Frame);
    /// A key, mouse, focus or resize event; true quits.
    fn event(&mut self, event: Event) -> bool;
    /// Called when no event came within `timeout`: files to read again, animations to step. True when the screen changed
    /// and needs drawing again.
    fn tick(&mut self) -> bool;
    /// How long to wait for an event before the next tick: short while something animates.
    fn timeout(&self) -> Duration;
}

static WOKEN: AtomicBool = AtomicBool::new(false);
static REDRAW: AtomicBool = AtomicBool::new(false);

/// Wakes the loop and draws the next frame whatever the app's tick says: an answer that arrived in the background.
pub fn redraw() {
    REDRAW.store(true, Ordering::Relaxed);
    wake();
}

/// Makes the loop tick now rather than at its timeout: a watched file changed, a background reply came.
pub fn wake() {
    WOKEN.store(true, Ordering::Relaxed);
}

/// Waits for an event up to `timeout`, in short slices so a `wake` is seen within 50 ms; true when an event is there.
fn wait(timeout: Duration) -> std::io::Result<bool> {
    let end = Instant::now() + timeout;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if event::poll(left.min(Duration::from_millis(50)))? {
            return Ok(true);
        }
        if WOKEN.swap(false, Ordering::Relaxed) || left.is_zero() {
            return Ok(false);
        }
    }
}

pub fn run(app: &mut impl App) -> std::io::Result<()> {
    let mut terminal: DefaultTerminal = ratatui::init();
    let _ = execute!(stdout(), EnableMouseCapture, EnableFocusChange);
    let result = (|| -> std::io::Result<()> {
        let mut redraw = true;
        loop {
            if redraw {
                terminal.draw(|frame| app.draw(frame))?;
            }
            redraw = if wait(app.timeout())? {
                if app.event(event::read()?) {
                    return Ok(());
                }
                true
            } else {
                app.tick() | REDRAW.swap(false, Ordering::Relaxed)
            };
        }
    })();
    let _ = execute!(stdout(), DisableMouseCapture, DisableFocusChange);
    ratatui::restore();
    result
}
