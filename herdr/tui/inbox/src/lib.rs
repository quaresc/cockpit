//! inbox.py in Rust, for the config's `apps: "rust"`: the same caches as one tree, every action run by inbox.py
//! itself through bridge.py; filtered as you type, rows picked for batch actions, status changes shown as they land.

pub mod app;
mod changes;
mod details;
pub mod facts;
mod filter;
mod history;
mod icons;
pub mod inputs;
mod keys;
pub mod model;
mod panel;
mod requests;
pub mod row;
mod select;
pub mod sessions;
mod tree;
mod work;
