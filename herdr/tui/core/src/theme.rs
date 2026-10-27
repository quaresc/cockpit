//! Vesper: a warm orange accent, teal for good, red for broken, the rest dim, on a near-black background.

use ratatui::style::{Color, Modifier, Style};

pub const BG: Color = Color::Rgb(16, 16, 16);
pub const SURFACE: Color = Color::Rgb(22, 22, 22);
pub const PANEL: Color = Color::Rgb(30, 30, 34);
pub const LINE: Color = Color::Rgb(44, 44, 48);
pub const FG: Color = Color::Rgb(210, 210, 210);
pub const DIM: Color = Color::Rgb(112, 112, 118);
pub const ORANGE: Color = Color::Rgb(255, 199, 153);
pub const GREEN: Color = Color::Rgb(153, 255, 228);
pub const RED: Color = Color::Rgb(255, 128, 128);
pub const BLUE: Color = Color::Rgb(160, 160, 255);
pub const PINK: Color = Color::Rgb(255, 153, 204);
pub const SERIES: [Color; 5] = [ORANGE, GREEN, BLUE, PINK, RED];

pub fn fg(color: Color) -> Style {
    Style::new().fg(color)
}

pub fn dim() -> Style {
    fg(DIM)
}

pub fn bold(color: Color) -> Style {
    fg(color).add_modifier(Modifier::BOLD)
}

pub fn base() -> Style {
    Style::new().fg(FG).bg(BG)
}
