//! The pinned actions space, a tab per one-off Claude session (see cockpit_core::actions), opened among the spaces the
//! action's config pins.

use crate::config::Config;

pub use cockpit_core::actions::{focus, pane_of, promote};

/// A new tab in the actions space, in view: its pane id, kept under `key`; None without the space.
pub fn open_tab(config: &Config, cwd: &str, label: &str, key: Option<&str>) -> Option<String> {
    cockpit_core::actions::open_tab(&config.space_names(), cwd, label, key)
}
