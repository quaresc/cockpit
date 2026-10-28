//! Screens as text for snapshot tests: an app drawn on ratatui's test backend, one line per row, trailing spaces cut.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::term::App;

pub fn render(app: &mut impl App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
    terminal.draw(|frame| app.draw(frame)).expect("a frame");
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
