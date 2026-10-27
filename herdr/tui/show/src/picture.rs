//! Charts, flow charts and pictures as real images: drawn in a background thread at the pane's width in pixels (cells
//! times the terminal's font size), again whenever that width changes, then shown with ratatui-image.

use image::DynamicImage;
use image::imageops::FilterType;
use ratatui::layout::Size;
use ratatui_image::FontSize;
use ratatui_image::picker::Picker;
use ratatui_image::sliced::SlicedProtocol;
use serde_json::Value;

use cockpit_core::theme::{RED, fg};

use crate::row::Row;
use crate::{mermaid, raster, svg};

/// A tall screenshot shrinks until it fits this many rows, so it never takes over the pane.
const IMAGE_ROWS: f64 = 24.0;

#[derive(Clone, Copy)]
pub enum Draw {
    Line,
    Bar,
    Flow,
    Image,
}

pub type Drawn = Result<SlicedProtocol, String>;

pub struct Picture {
    pub draw: Draw,
    pub asked: Option<usize>,
    pub shown: Option<SlicedProtocol>,
    error: Option<String>,
}

fn rows_for(pixels: u32, font: FontSize) -> u16 {
    pixels.div_ceil(font.height as u32).max(1) as u16
}

/// The block drawn `cols` cells wide, encoded for the terminal here too, since Kitty's deflate takes a while.
pub fn draw(draw: Draw, block: &Value, cols: usize, picker: &Picker) -> Drawn {
    let (image, size) = pixels(draw, block, cols, picker.font_size())?;
    SlicedProtocol::new(picker, image, Some(size)).map_err(|e| e.to_string())
}

/// The block's pixels at `cols` cells wide, and the cells they cover.
fn pixels(draw: Draw, block: &Value, cols: usize, font: FontSize) -> Result<(DynamicImage, Size), String> {
    let (fw, fh) = (font.width as f64, font.height as f64);
    let width = cols as f64 * fw;
    let label = fh * 0.48;
    match draw {
        Draw::Line => {
            let rows = block["height"].as_u64().filter(|h| *h > 0).unwrap_or(12);
            let text = svg::line_chart(block, width, rows as f64 * fh, label).ok_or("no numbers to draw")?;
            Ok((raster::svg(&text, None)?, Size::new(cols as u16, rows as u16)))
        }
        Draw::Bar => {
            let (text, _) = svg::bar_chart(block, width, label).ok_or("no numbers to draw")?;
            let image = raster::svg(&text, None)?;
            let rows = rows_for(image.height(), font);
            Ok((image, Size::new(cols as u16, rows)))
        }
        Draw::Flow => {
            let flow = if block["type"] == "mermaid" { mermaid::flowchart(block["text"].as_str().unwrap_or_default()) } else { block.clone() };
            let (text, natural, _) = svg::flow_chart(&flow, label);
            let shown = width.min(natural);
            let image = raster::svg(&text, Some(shown.round()))?;
            Ok((image.clone(), Size::new((shown / fw).round().max(1.0) as u16, rows_for(image.height(), font))))
        }
        Draw::Image => {
            let path = raster::fetch(block["path"].as_str().or(block["url"].as_str()).unwrap_or_default())?;
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).trim_start().to_string();
            let asked = block["width"].as_f64().filter(|w| *w > 0.0).map(|w| w * fw);
            if path.extension().is_some_and(|e| e == "svg") || head.starts_with("<svg") || head.starts_with("<?xml") {
                let text = String::from_utf8_lossy(&bytes);
                let shown = width.min(asked.unwrap_or(svg::svg_size(&text).0 * 2.0));
                let image = raster::svg(&text, Some(shown.round()))?;
                return Ok((image.clone(), Size::new((shown / fw).round().max(1.0) as u16, rows_for(image.height(), font))));
            }
            let image = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
            let (w, h) = (image.width() as f64, image.height() as f64);
            let shown = width.min(asked.unwrap_or(f64::MAX)).min(IMAGE_ROWS * fh * w / h);
            let cells = (shown / fw).round().max(1.0);
            let image = image.resize((cells * fw) as u32, (IMAGE_ROWS * fh) as u32, FilterType::Triangle);
            let rows = rows_for(image.height(), font).min(IMAGE_ROWS as u16);
            Ok((image, Size::new(cells as u16, rows)))
        }
    }
}

impl Picture {
    pub fn new(draw: Draw) -> Self {
        Self { draw, asked: None, shown: None, error: None }
    }

    /// True when the picture must be drawn again for this width.
    pub fn wants(&mut self, cols: usize) -> bool {
        if self.asked == Some(cols) {
            return false;
        }
        self.asked = Some(cols);
        true
    }

    pub fn arrived(&mut self, cols: usize, drawn: Drawn) {
        if self.asked != Some(cols) {
            return;
        }
        match drawn {
            Ok(protocol) => (self.shown, self.error) = (Some(protocol), None),
            Err(err) => (self.shown, self.error) = (None, Some(err)),
        }
    }

    pub fn rows(&self) -> Vec<Row> {
        if let Some(err) = &self.error {
            return vec![Row::text(format!("cannot draw it: {err}"), fg(RED))];
        }
        let height = self.shown.as_ref().map_or(0, |p| p.size().height);
        (0..height).map(|k| Row { image: Some(k), ..Row::new() }).collect()
    }
}
