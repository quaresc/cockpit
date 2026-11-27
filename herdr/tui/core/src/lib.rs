//! What the Rust space apps share: the Vesper theme, the terminal loop, file watching, the hub client that runs actions
//! and searches across apps, the header that says how fresh each source is, what is left of getting started, what keeps a
//! worktree, system notifications, what changed since you last looked, the options and what of them was shown, the
//! routines' files, and the palette, dialogs, toasts, text field, shared keys, a row pointed at from outside, widgets,
//! animations and screen snapshots.

pub mod accounts;
pub mod actions;
pub mod acts;
pub mod agent;
pub mod anim;
pub mod bridge;
pub mod browser;
pub mod cleanup;
pub mod commands;
pub mod deliver;
pub mod dialog;
pub mod files;
pub mod focus;
pub mod fresh;
pub mod frontend;
pub mod header;
pub mod help;
pub mod hub;
pub mod input;
pub mod jsonedit;
pub mod keys;
pub mod models;
pub mod modes;
pub mod names;
pub mod notify;
pub mod options;
pub mod palette;
pub mod peer;
pub mod pointed;
pub mod resume;
pub mod routines;
pub mod run;
pub mod seen;
pub mod sites;
pub mod sources;
pub mod started;
pub mod switches;
pub mod term;
pub mod testing;
pub mod text;
pub mod theme;
pub mod toast;
pub mod transcripts;
pub mod usage;
pub mod widgets;

/// The apps that run in a pinned space of their own, by the name `cockpit <app>` runs them under.
pub const SPACE_APPS: [&str; 8] = ["inbox", "todo", "runbook", "meetings", "monitor", "routines", "review", "mail"];

/// The pinned space an app of `SPACE_APPS` runs in: its own name, but the to-do space for `todo`.
pub fn space_of(app: &str) -> &str {
    if app == "todo" { "to-do" } else { app }
}

/// `--version` prints the binary's name and the workspace version, then exits: doctor checks the binaries run with it.
pub fn answer_version(name: &str) {
    if std::env::args().skip(1).any(|a| a == "--version") {
        println!("{name} {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }
}
