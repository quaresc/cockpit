//! The inbox: the caches as one tree, every action run by the hub; filtered as you type, rows picked for batch actions,
//! status changes shown as they land.

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
#[cfg(test)]
mod test_home;
mod tree;
mod work;
