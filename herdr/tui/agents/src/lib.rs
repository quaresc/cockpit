//! cockpit's agents: every agent terminal of the Herdr app and of herdr, what each asks and has done, whose move it is,
//! and the time each ticket took. The hub serves it as the `agents` view; both halves act on it through its actions.

// An agents row is one `json!` object with more fields than serde_json's default macro depth expands.
#![recursion_limit = "256"]

pub mod chat;
pub mod cli;
pub mod gui;
pub mod ledger;
pub mod links;
pub mod load;
pub mod next;
pub mod place;
pub mod prompt;
pub mod recap;
pub mod session;
pub mod spaces;
pub mod state;
pub mod term;
