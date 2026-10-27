//! show.py in Rust: `cockpit-show <name>` shows ~/.cache/cockpit/show/<name>.json beside a Claude session, keeps
//! `<name>.pid` while it runs (the mod reads it to update the open pane) and writes answers like show.py.

mod app;
mod blocks;
mod diff;
mod legacy;
mod log;
mod markdown;
mod mermaid;
mod picture;
mod question;
mod raster;
mod row;
mod svg;
mod table;
mod value;

use std::path::PathBuf;

use ratatui_image::picker::Picker;
use ratatui_image::picker::cap_parser::QueryStdioOptions;

use cockpit_core::{files, term};

/// Removes the pid file however the app ends: quit, panic, or the pane closing under it.
struct Alive(PathBuf);

impl Drop for Alive {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn main() -> std::io::Result<()> {
    let name = std::env::args().nth(1).unwrap_or_else(|| "show".into());
    let dir = files::cache().join("show");
    std::fs::create_dir_all(&dir)?;
    let alive = Alive(dir.join(format!("{name}.pid")));
    std::fs::write(&alive.0, std::process::id().to_string())?;
    for signal in [signal_hook::consts::SIGHUP, signal_hook::consts::SIGTERM] {
        let pid = alive.0.clone();
        // SAFETY: the handler only unlinks a path built beforehand and exits without unwinding.
        let _ = unsafe {
            signal_hook::low_level::register(signal, move || {
                let _ = std::fs::remove_file(&pid);
                signal_hook::low_level::exit(0);
            })
        };
    }
    raster::warm_up();
    let mut picker = Picker::from_query_stdio_with_options(QueryStdioOptions { kitty_compression: true, ..Default::default() }).unwrap_or_else(|_| Picker::halfblocks());
    picker.set_background_color(Some(image::Rgba([16, 16, 16, 255])));
    let mut app = app::ShowApp::new(name, dir, picker);
    term::run(&mut app)
}
