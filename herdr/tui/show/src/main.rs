//! show.py in Rust: `cockpit-show <name>` shows ~/.cache/cockpit/show/<name>.json beside a Claude session, keeps
//! `<name>.pid` while it runs (the mod reads it to update the open pane) and writes answers like show.py.

use std::path::PathBuf;

use ratatui_image::picker::Picker;
use ratatui_image::picker::cap_parser::QueryStdioOptions;

use cockpit_core::{files, term};
use cockpit_show::{ShowApp, release_freed_pages, warm_up};

/// mimalloc hands freed pages back to the system soon after: the system allocator kept the tens of megabytes a picture's
/// draw frees, so an idle pane held them.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Removes the pid file however the app ends: quit, panic, or the pane closing under it.
struct Alive(PathBuf);

impl Drop for Alive {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn main() -> std::io::Result<()> {
    cockpit_core::answer_version("cockpit-show");
    release_freed_pages();
    let name = std::env::args().nth(1).unwrap_or_else(|| "show".into());
    let dir = files::cache().join("show");
    std::fs::create_dir_all(&dir)?;
    let alive = Alive(dir.join(format!("{name}.pid")));
    std::fs::write(&alive.0, std::process::id().to_string())?;
    // The pane closing (HUP) or the mod hiding the view (TERM) ends the app here: the pid file goes and the terminal
    // gets its screen and mouse back, which a bare exit in the signal handler would leave in the app's modes.
    if let Ok(mut signals) = signal_hook::iterator::Signals::new([signal_hook::consts::SIGHUP, signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT]) {
        let pid = alive.0.clone();
        std::thread::spawn(move || {
            if signals.forever().next().is_some() {
                let _ = std::fs::remove_file(&pid);
                let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
                ratatui::restore();
                std::process::exit(0);
            }
        });
    }
    warm_up();
    let mut picker = Picker::from_query_stdio_with_options(QueryStdioOptions { kitty_compression: true, ..Default::default() }).unwrap_or_else(|_| Picker::halfblocks());
    picker.set_background_color(Some(image::Rgba([16, 16, 16, 255])));
    let mut app = ShowApp::new(name, dir, picker);
    term::run(&mut app)
}
