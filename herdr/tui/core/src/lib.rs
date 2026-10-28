//! What the Rust space apps share: the Vesper theme, the terminal loop, file watching, the hub client that runs actions
//! and searches across apps, and the palette, dialogs, toasts, text field, widgets, animations and screen snapshots.

pub mod acts;
pub mod anim;
pub mod bridge;
pub mod dialog;
pub mod files;
pub mod focus;
pub mod hub;
pub mod input;
pub mod palette;
pub mod term;
pub mod testing;
pub mod text;
pub mod theme;
pub mod toast;
pub mod widgets;

/// `--version` prints the binary's name and the workspace version, then exits: doctor checks the binaries run with it.
pub fn answer_version(name: &str) {
    if std::env::args().skip(1).any(|a| a == "--version") {
        println!("{name} {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }
}
