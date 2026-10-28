//! A before/after pair of pictures: side by side when the pane is wide, else (or when asked) one over the other with a
//! split the keyboard or the mouse moves, before on its left and after on its right.

use std::sync::Arc;

use image::imageops::{FilterType, overlay};
use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::layout::Size;
use ratatui_image::FontSize;
use serde_json::Value;

use crate::raster;

/// Side by side needs room for two pictures worth looking at.
pub const WIDE: usize = 100;
const BACKGROUND: Rgba<u8> = Rgba([16, 16, 16, 255]);
const DIVIDER: Rgba<u8> = Rgba([255, 199, 153, 255]);

/// Before, after, and the width in cells they were scaled down for: only that much of each picture is kept.
pub type Pair = Arc<(DynamicImage, DynamicImage, usize)>;

/// A picture from a path or URL: an SVG rasterized, any other format decoded (a GIF's first frame).
fn picture(source: &str) -> Result<DynamicImage, String> {
    let path = raster::fetch(source)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if raster::is_svg(&path, &bytes) {
        let text = String::from_utf8_lossy(&bytes);
        let natural = crate::svg::svg_size(&text).0 * 2.0;
        return raster::svg(&text, Some(natural));
    }
    image::load_from_memory(&bytes).map_err(|e| format!("{source}: {e}"))
}

/// Both pictures, no larger than they can show `cols` cells wide and `rows` rows tall.
pub fn load(block: &Value, cols: usize, rows: u16, font: FontSize) -> Result<Pair, String> {
    let (w, h) = (cols as u32 * u32::from(font.width), u32::from(rows) * u32::from(font.height));
    let get = |key: &str| -> Result<DynamicImage, String> {
        let image = picture(block[key].as_str().unwrap_or_default())?;
        Ok(if image.width() > w || image.height() > h { fit(&image, w, h) } else { image })
    };
    Ok(Arc::new((get("before")?, get("after")?, cols)))
}

/// The picture scaled to fit `w`×`h` pixels, keeping its shape.
fn fit(image: &DynamicImage, w: u32, h: u32) -> DynamicImage {
    let scale = (w as f64 / image.width().max(1) as f64).min(h as f64 / image.height().max(1) as f64);
    let (w, h) = ((image.width() as f64 * scale).round().max(1.0) as u32, (image.height() as f64 * scale).round().max(1.0) as u32);
    image.resize_exact(w, h, FilterType::Triangle)
}

/// Cells for pixels, to the nearest whole cell.
fn cells(pixels: u32, cell: u16) -> u16 {
    (pixels as f64 / cell.max(1) as f64).round().max(1.0) as u16
}

/// The pair laid out `cols` cells wide, at most `rows` rows tall: split at `split` (0 to 1), or side by side without one.
pub fn compose(pair: &Pair, cols: usize, rows: u16, split: Option<f32>, font: FontSize) -> (DynamicImage, Size) {
    let (before, after) = (&pair.0, &pair.1);
    let tall = rows as u32 * font.height as u32;
    let Some(split) = split else {
        let left = (cols - 2) / 2;
        let right = cols - 2 - left;
        let (a, b) = (fit(before, left as u32 * font.width as u32, tall), fit(after, right as u32 * font.width as u32, tall));
        let height = a.height().max(b.height());
        let mut canvas = RgbaImage::from_pixel(cols as u32 * font.width as u32, height, BACKGROUND);
        overlay(&mut canvas, &a.to_rgba8(), 0, 0);
        overlay(&mut canvas, &b.to_rgba8(), (left + 2) as i64 * font.width as i64, 0);
        return (DynamicImage::ImageRgba8(canvas), Size::new(cols as u16, cells(height, font.height)));
    };
    let a = fit(before, cols as u32 * font.width as u32, tall);
    let (w, h) = (a.width(), a.height());
    let b = fit(after, w, h);
    let mut back = RgbaImage::from_pixel(w, h, BACKGROUND);
    overlay(&mut back, &b.to_rgba8(), ((w - b.width()) / 2) as i64, ((h - b.height()) / 2) as i64);
    let mut canvas = a.to_rgba8();
    let at = (split.clamp(0.0, 1.0) * w as f32).round() as u32;
    for y in 0..h {
        for x in at..w {
            canvas.put_pixel(x, y, *back.get_pixel(x, y));
        }
        for x in at.saturating_sub(1)..(at + 1).min(w) {
            canvas.put_pixel(x, y, DIVIDER);
        }
    }
    (DynamicImage::ImageRgba8(canvas), Size::new(cells(w, font.width), cells(h, font.height)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: u8) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(w, h, Rgba([c, c, c, 255])))
    }

    #[test]
    fn the_split_shows_before_left_and_after_right() {
        let pair: Pair = Arc::new((solid(100, 50, 0), solid(100, 50, 255), 20));
        let (image, size) = compose(&pair, 20, 24, Some(0.5), FontSize { width: 10, height: 20 });
        let rgba = image.to_rgba8();
        assert_eq!((rgba.get_pixel(10, 10)[0], rgba.get_pixel(190, 10)[0], rgba.get_pixel(100, 10)[0]), (0, 255, 255));
        assert_eq!(size, Size::new(20, 5));
    }
}
