//! What the Rust space apps share: the Vesper theme, the terminal loop, file watching, the Python bridge that runs the
//! text apps' actions, and the command palette, dialogs, toasts and action buttons.

pub mod acts;
pub mod bridge;
pub mod dialog;
pub mod files;
pub mod palette;
pub mod term;
pub mod text;
pub mod theme;
pub mod toast;
