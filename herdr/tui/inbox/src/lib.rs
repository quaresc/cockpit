//! The inbox: the caches as one tree, every action run by the hub; filtered as you type, rows picked for batch actions,
//! status changes shown as they land.

pub mod app;
mod changes;
mod details;
pub mod facts;
mod filter;
pub mod gui;
mod history;
mod icons;
pub mod inputs;
mod keys;
pub mod model;
mod panel;
mod requests;
pub mod row;
pub mod select;
pub mod sessions;
pub mod start;
#[cfg(test)]
mod test_home;
pub mod tree;
mod work;
pub mod worktrees;

/// `cockpit start-work [source]`: the Start work form alone in the terminal until it closes.
pub fn start_work(source: &str) -> std::io::Result<()> {
    start::run(source)
}

/// `cockpit inbox`: the inbox in the terminal until it quits.
pub fn run() -> std::io::Result<()> {
    cockpit_core::term::run(&mut app::App::start())
}
