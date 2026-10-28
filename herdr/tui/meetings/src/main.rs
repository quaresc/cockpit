//! cockpit-meetings: the meetings space's app.

use ratatui_image::picker::Picker;
use ratatui_image::picker::cap_parser::QueryStdioOptions;

fn main() -> std::io::Result<()> {
    cockpit_core::answer_version("cockpit-meetings");
    cockpit_meetings::picture::warm_up();
    // Asked before the screen is taken: the terminal's graphics protocol and font size, else half blocks.
    let mut picker = Picker::from_query_stdio_with_options(QueryStdioOptions { kitty_compression: true, ..Default::default() }).unwrap_or_else(|_| Picker::halfblocks());
    picker.set_background_color(Some(image::Rgba([16, 16, 16, 255])));
    let mut app = cockpit_meetings::app::App::start(picker);
    app.report();
    cockpit_core::term::run(&mut app)
}
