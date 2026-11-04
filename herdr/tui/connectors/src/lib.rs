//! The person's MCP servers called by cockpit itself, with the mappings setup writes: no agent in between.

pub mod calendar;
pub mod ci;
pub mod client;
pub mod credentials;
pub mod mapping;
pub mod monitor;
pub mod platform;
pub mod presets;
pub mod sources;
pub mod tickets;
pub mod tools;

pub use client::{Connection, Server, block_on, call_mapped, connect, login};
