//! The person's MCP servers called by cockpit itself, with the mappings setup writes: no agent in between.

pub mod calendar;
pub mod ci;
pub mod client;
pub mod code;
pub mod credentials;
pub mod mail;
pub mod mapping;
pub mod monitor;
pub mod observability;
pub mod platform;
pub mod presets;
pub mod reads;
pub mod sources;
pub mod tickets;
pub mod tools;

pub use client::{Connection, Server, block_on, call_mapped, call_mapped_text, connect, login, login_server, printed_token};
