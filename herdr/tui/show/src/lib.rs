//! The view a Claude session shows beside itself, read from ~/.cache/cockpit/show/<name>.json, its
//! answers and line comments written back for the mod to hand to the session.

mod answers;
mod app;
mod blocks;
mod chart;
mod compare;
mod diff;
mod highlight;
mod log;
mod markdown;
mod mermaid;
mod patch;
mod picture;
mod question;
mod raster;

pub use raster::use_only_font;
mod row;
mod sequence;
mod svg;
mod table;
mod text_drawing;
mod value;

pub use app::ShowApp;
pub use raster::{release_freed_pages, warm_up};
