//! inbox.py in Rust, for the config's `apps: "rust"`: the same caches as one tree, every action run by inbox.py
//! itself through bridge.py.

mod app;
mod facts;
mod icons;
mod model;
mod panel;
mod requests;
mod row;
mod sessions;
mod tree;
mod work;

fn main() -> std::io::Result<()> {
    cockpit_core::term::run(&mut app::App::new())
}
