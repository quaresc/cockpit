//! Charts, flow charts, pictures and before/after pairs as real images: drawn in a background thread at the pane's width
//! in pixels (cells times the terminal's font size), again whenever the width, the hovered point, the zoom, the split or
//! a glide step asks, one draw at a time per picture, the newest ask winning. A GIF plays while it is in view.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use image::codecs::gif::GifDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage};
use ratatui::layout::Size;
use ratatui_image::FontSize;
use ratatui_image::picker::Picker;
use ratatui_image::sliced::SlicedProtocol;
use serde_json::Value;

use cockpit_core::theme::{DIM, FG, ORANGE, bold, dim, fg};

use crate::chart;
use crate::compare::{self, Pair};
use crate::row::{Row, Target};
use crate::svg::Geometry;
use crate::value::rows as rows_of;
use crate::{mermaid, raster, svg};

/// A tall screenshot shrinks until it fits this many rows, so it never takes over the pane.
const IMAGE_ROWS: f64 = 24.0;
/// A refresh's new values glide in over this long, a frame every `FRAME`.
const GLIDE: Duration = Duration::from_millis(500);
pub const FRAME: Duration = Duration::from_millis(80);
/// A GIF plays at most this many frames, so a long recording does not hold the memory of a film.
const GIF_FRAMES: usize = 240;

#[derive(Clone, Copy, PartialEq)]
pub enum Draw {
    Line,
    Bar,
    Flow,
    Image,
    Compare,
}

/// What one background draw is asked for.
pub struct Job {
    draw: Draw,
    block: Value,
    cols: usize,
    hover: Option<usize>,
    /// A compare's split, None side by side.
    split: Option<f32>,
    pair: Option<Pair>,
}

pub struct Drawn {
    cols: usize,
    frames: Vec<(SlicedProtocol, Duration)>,
    geometry: Option<Geometry>,
    pair: Option<Pair>,
}

/// One picture is drawn at a time: each takes a few full-size pixel buffers, and ten charts drawn at once would leave
/// the process holding all of them at its peak.
static DRAWING: Mutex<()> = Mutex::new(());

/// Rows for a height in pixels, to the nearest whole row: rounding up left an empty row under flows and pictures.
fn rows_for(pixels: u32, font: FontSize) -> u16 {
    (pixels as f64 / font.height.max(1) as f64).round().max(1.0) as u16
}

fn encode(picker: &Picker, image: DynamicImage, size: Size) -> Result<SlicedProtocol, String> {
    SlicedProtocol::new(picker, image, Some(size)).map_err(|e| e.to_string())
}

/// The job drawn and encoded for the terminal here too, since Kitty's deflate takes a while.
/// The block's key for `COCKPIT_SHOT_DIR`: the README's screenshot script lays each picture, saved there, over the cells
/// its block holds, since a captured terminal keeps no images.
pub fn shot_key(block: &Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    block.to_string().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn draw(job: Job, picker: &Picker) -> Result<Drawn, String> {
    let _turn = DRAWING.lock().unwrap_or_else(PoisonError::into_inner);
    let (cols, font) = (job.cols, picker.font_size());
    let shots = std::env::var("COCKPIT_SHOT_DIR").ok();
    let key = shot_key(&job.block);
    let still = |image: DynamicImage, size: Size, geometry: Option<Geometry>, pair: Option<Pair>| -> Result<Drawn, String> {
        if let Some(dir) = &shots {
            let _ = image.save(std::path::Path::new(dir).join(format!("{key}.png")));
        }
        Ok(Drawn { cols, frames: vec![(encode(picker, image, size)?, Duration::ZERO)], geometry, pair })
    };
    let (fw, fh) = (font.width as f64, font.height as f64);
    let width = cols as f64 * fw;
    let label = fh * 0.48;
    match job.draw {
        Draw::Line => {
            let rows = job.block["height"].as_u64().filter(|h| *h > 0).unwrap_or(12);
            let (text, geometry) = svg::line_chart(&job.block, width, rows as f64 * fh, label, job.hover).ok_or("no numbers to draw")?;
            still(raster::svg(&text, None)?, Size::new(cols as u16, rows as u16), Some(geometry), None)
        }
        Draw::Bar => {
            let (text, _, geometry) = svg::bar_chart(&job.block, width, label, job.hover).ok_or("no numbers to draw")?;
            let image = raster::svg(&text, None)?;
            let rows = rows_for(image.height(), font);
            still(image, Size::new(cols as u16, rows), Some(geometry), None)
        }
        Draw::Flow => {
            let block = &job.block;
            let flow = if block["type"] == "mermaid" { mermaid::flowchart(block["text"].as_str().unwrap_or_default()) } else { block.clone() };
            if flow["nodes"].as_array().is_none_or(Vec::is_empty) {
                return Err("no nodes to draw".into());
            }
            let (text, natural, _) = svg::flow_chart(&flow, label);
            let shown = width.min(natural);
            let image = raster::svg(&text, Some(shown.round()))?;
            let size = Size::new((shown / fw).round().max(1.0) as u16, rows_for(image.height(), font));
            still(image, size, None, None)
        }
        Draw::Image => Ok(Drawn { cols, frames: image(&job.block, width, font, picker)?, geometry: None, pair: None }),
        Draw::Compare => {
            let pair = match job.pair.filter(|p| p.2 == cols) {
                Some(pair) => pair,
                None => compare::load(&job.block, cols, IMAGE_ROWS as u16, font)?,
            };
            let (image, size) = compare::compose(&pair, cols, IMAGE_ROWS as u16, job.split, font);
            still(image, size, None, Some(pair))
        }
    }
}

/// A picture file or URL fitted to the width and at most `IMAGE_ROWS` rows: one frame, or every frame of an animated
/// GIF with its delay.
fn image(block: &Value, width: f64, font: FontSize, picker: &Picker) -> Result<Vec<(SlicedProtocol, Duration)>, String> {
    let (fw, fh) = (font.width as f64, font.height as f64);
    let path = raster::fetch(block["path"].as_str().or(block["url"].as_str()).unwrap_or_default())?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let asked = block["width"].as_f64().filter(|w| *w > 0.0).map(|w| w * fw);
    if raster::is_svg(&path, &bytes) {
        let text = String::from_utf8_lossy(&bytes);
        let shown = width.min(asked.unwrap_or(svg::svg_size(&text).0 * 2.0));
        let image = raster::svg(&text, Some(shown.round()))?;
        let size = Size::new((shown / fw).round().max(1.0) as u16, rows_for(image.height(), font).min(IMAGE_ROWS as u16));
        return Ok(vec![(encode(picker, image, size)?, Duration::ZERO)]);
    }
    let first = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    let (w, h) = (first.width() as f64, first.height() as f64);
    let shown = width.min(asked.unwrap_or(f64::MAX)).min(IMAGE_ROWS * fh * w / h);
    let cells = (shown / fw).round().max(1.0);
    let (pw, ph) = ((cells * fw) as u32, (IMAGE_ROWS * fh) as u32);
    // Each frame is scaled to the size shown and encoded as it is decoded, so only the encoded frames stay.
    let fitted = |frame: DynamicImage, delay: Duration| -> Result<(SlicedProtocol, Duration), String> {
        let frame = frame.resize(pw, ph, FilterType::Triangle);
        let size = Size::new(cells as u16, rows_for(frame.height(), font).min(IMAGE_ROWS as u16));
        Ok((encode(picker, frame, size)?, delay))
    };
    if !bytes.starts_with(b"GIF8") {
        return Ok(vec![fitted(first, Duration::ZERO)?]);
    }
    drop(first);
    let decoder = GifDecoder::new(std::io::Cursor::new(&bytes)).map_err(|e| e.to_string())?;
    decoder
        .into_frames()
        .take(GIF_FRAMES)
        .map(|frame| {
            let frame = frame.map_err(|e| e.to_string())?;
            let (numer, denom) = frame.delay().numer_denom_ms();
            // Like browsers, a delay under 20 ms plays at 100 ms.
            let ms = numer / denom.max(1);
            fitted(DynamicImage::ImageRgba8(frame.into_buffer()), Duration::from_millis(if ms < 20 { 100 } else { u64::from(ms) }))
        })
        .collect()
}

pub struct Picture {
    pub draw: Draw,
    /// The data drawn: the spec's, or the rows a command printed.
    pub block: Value,
    asked: Option<usize>,
    stale: bool,
    busy: bool,
    frames: Vec<(SlicedProtocol, Duration)>,
    frame: usize,
    next_frame: Option<Instant>,
    /// Whether any of it was on screen at the last frame: a GIF pauses out of view.
    pub visible: bool,
    error: Option<String>,
    geometry: Option<Geometry>,
    /// The hovered point, an index into all the rows.
    hover: Option<usize>,
    zoom: Option<(usize, usize)>,
    /// The rows a refresh replaced and when it came: the values glide from them for `GLIDE`.
    glide: Option<(Value, Instant)>,
    drew_at: Option<Instant>,
    split: f32,
    /// A compare shows its split even in a pane wide enough for side by side (`s`).
    slider: bool,
    pair: Option<Pair>,
}

impl Picture {
    pub fn new(draw: Draw, block: Value) -> Self {
        Self {
            draw,
            block,
            asked: None,
            stale: true,
            busy: false,
            frames: Vec::new(),
            frame: 0,
            next_frame: None,
            visible: false,
            error: None,
            geometry: None,
            hover: None,
            zoom: None,
            glide: None,
            drew_at: None,
            split: 0.5,
            slider: false,
            pair: None,
        }
    }

    fn is_chart(&self) -> bool {
        matches!(self.draw, Draw::Line | Draw::Bar)
    }

    /// Takes over what a picture it replaces showed, so a refresh never blanks it: its image until the new one comes, its
    /// hover, zoom and split; a chart whose rows changed glides to the new values.
    pub fn inherit(&mut self, old: Picture) {
        if old.draw != self.draw {
            return;
        }
        if self.is_chart() && !old.frames.is_empty() && rows_of(&old.block) != rows_of(&self.block) {
            self.glide = Some((old.block.clone(), Instant::now()));
        }
        if self.draw == Draw::Compare && old.block["before"] == self.block["before"] && old.block["after"] == self.block["after"] {
            self.pair = old.pair;
        }
        (self.frames, self.geometry, self.asked) = (old.frames, old.geometry, old.asked);
        (self.hover, self.zoom, self.split, self.slider) = (old.hover, old.zoom, old.split, old.slider);
        let total = self.total();
        self.hover = self.hover.filter(|h| *h < total);
        self.zoom = self.zoom.filter(|(start, len)| start + len <= total);
    }

    /// The width last asked for, for a draw due without a new layout (a glide step, a hover).
    pub fn asked(&self) -> Option<usize> {
        self.asked
    }

    /// The next background draw, when one is due and none runs: a width change, a hover, a zoom, a split, a glide step.
    pub fn job(&mut self, cols: usize) -> Option<Job> {
        if self.asked != Some(cols) {
            (self.asked, self.stale) = (Some(cols), true);
        }
        if self.busy {
            return None;
        }
        let mut block = chart::windowed(&self.block, self.zoom);
        if let Some((from, at)) = &self.glide {
            let t = at.elapsed().as_secs_f64() / GLIDE.as_secs_f64();
            if t >= 1.0 {
                (self.glide, self.stale) = (None, true);
            } else if self.drew_at.is_none_or(|d| d.elapsed() >= FRAME) {
                let eased = f64::from(cockpit_core::anim::out_cubic(t as f32));
                block = chart::between(&chart::windowed(from, self.zoom), &block, eased, self.draw == Draw::Bar);
                self.stale = true;
            }
        }
        if !self.stale {
            return None;
        }
        (self.stale, self.busy, self.drew_at) = (false, true, Some(Instant::now()));
        let hover = self.hover.map(|h| h - self.zoom.map_or(0, |z| z.0));
        let split = self.splits(cols).then_some(self.split);
        Some(Job { draw: self.draw, block, cols, hover, split, pair: self.pair.clone() })
    }

    /// Whether a compare shows its split: always in a narrow pane, when asked in a wide one.
    fn splits(&self, cols: usize) -> bool {
        self.slider || cols < compare::WIDE
    }

    /// A draw came back; false when it was for a width no longer asked.
    pub fn arrived(&mut self, drawn: Result<Drawn, String>) -> bool {
        self.busy = false;
        match drawn {
            Ok(drawn) if Some(drawn.cols) == self.asked => {
                (self.frames, self.geometry, self.error) = (drawn.frames, drawn.geometry, None);
                self.pair = drawn.pair.or(self.pair.take());
                self.frame = self.frame.min(self.frames.len().saturating_sub(1));
                true
            }
            Ok(_) => false,
            Err(err) => {
                (self.frames, self.error) = (Vec::new(), Some(err));
                true
            }
        }
    }

    /// True while values glide: the loop ticks at the glide's pace.
    pub fn moving(&self) -> bool {
        self.glide.is_some()
    }

    /// True while a draw is wanted or under way.
    pub fn pending(&self) -> bool {
        self.busy || self.stale || self.glide.is_some()
    }

    pub fn protocol(&self) -> Option<&SlicedProtocol> {
        self.frames.get(self.frame).map(|f| &f.0)
    }

    /// The next GIF frame when its time came and the picture is in view; true when it changed.
    pub fn advance(&mut self) -> bool {
        if self.frames.len() < 2 || !self.visible {
            self.next_frame = None;
            return false;
        }
        let now = Instant::now();
        let Some(due) = self.next_frame else {
            self.next_frame = Some(now + self.frames[self.frame].1);
            return false;
        };
        if now < due {
            return false;
        }
        self.frame = (self.frame + 1) % self.frames.len();
        self.next_frame = Some(now + self.frames[self.frame].1);
        true
    }

    /// How long until the next GIF frame, for the loop's wait.
    pub fn next_due(&self) -> Option<Duration> {
        self.next_frame.map(|at| at.saturating_duration_since(Instant::now()))
    }

    pub fn targets(&self) -> Vec<Target> {
        match self.draw {
            Draw::Line | Draw::Bar if self.geometry.is_some() => vec![Target::Chart],
            Draw::Compare if self.protocol().is_some() => vec![Target::Compare],
            _ => vec![],
        }
    }

    fn total(&self) -> usize {
        rows_of(&self.block).len()
    }

    /// The point under a cell of the picture (its column and image row); true when the hovered point changed.
    pub fn hover(&mut self, at: Option<(usize, u16)>, font: FontSize) -> bool {
        let point = match (at, self.geometry) {
            (Some((col, row)), Some(geometry)) => {
                let x = (col as f64 + 0.5) * font.width as f64;
                let y = (row as f64 + 0.5) * font.height as f64;
                Some(geometry.at(x, y) + self.zoom.map_or(0, |z| z.0))
            }
            _ => None,
        };
        self.set_hover(point)
    }

    fn set_hover(&mut self, point: Option<usize>) -> bool {
        if point == self.hover {
            return false;
        }
        (self.hover, self.stale) = (point, true);
        true
    }

    /// The compare split moved to a column of the picture; true when it moved.
    pub fn drag(&mut self, col: usize) -> bool {
        if !self.asked.is_some_and(|cols| self.splits(cols)) {
            return false;
        }
        let cells = self.protocol().map_or(1, |p| p.size().width.max(1));
        let split = ((col as f32 + 0.5) / f32::from(cells)).clamp(0.0, 1.0);
        if (split - self.split).abs() < 0.005 {
            return false;
        }
        (self.split, self.stale) = (split, true);
        true
    }

    /// A focused chart: ←/→ move the hovered point, +/− zoom around it, 0 shows everything; a compare: h/l move the split.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if self.draw == Draw::Compare {
            if key.code == KeyCode::Char('s') && self.asked.is_some_and(|cols| cols >= compare::WIDE) {
                (self.slider, self.stale) = (!self.slider, true);
                return true;
            }
            if !self.asked.is_some_and(|cols| self.splits(cols)) {
                return false;
            }
            let by = match key.code {
                KeyCode::Char('h') | KeyCode::Left => -0.05,
                KeyCode::Char('l') | KeyCode::Right => 0.05,
                KeyCode::Char('H') => -0.25,
                KeyCode::Char('L') => 0.25,
                _ => return false,
            };
            (self.split, self.stale) = ((self.split + by).clamp(0.0, 1.0), true);
            return true;
        }
        let total = self.total();
        let (start, len) = self.zoom.unwrap_or((0, total));
        let last = (start + len).saturating_sub(1);
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                let point = self.hover.map_or(last, |h| h.saturating_sub(1).max(start));
                self.set_hover(Some(point));
            }
            KeyCode::Right | KeyCode::Char('l') => {
                let point = self.hover.map_or(last, |h| (h + 1).min(last));
                self.set_hover(Some(point));
            }
            KeyCode::Char('+' | '=') => self.rezoom(chart::zoom(self.zoom, total, self.hover, true)),
            KeyCode::Char('-' | '_') => self.rezoom(chart::zoom(self.zoom, total, self.hover, false)),
            KeyCode::Char('0') => self.rezoom(None),
            _ => return false,
        }
        true
    }

    fn rezoom(&mut self, zoom: Option<(usize, usize)>) {
        self.zoom = zoom;
        if let (Some(h), Some((start, len))) = (self.hover, zoom)
            && !(start..start + len).contains(&h)
        {
            self.hover = None;
        }
        self.stale = true;
    }

    pub fn rows(&self, cols: usize, focused: bool, compact: bool) -> Vec<Row> {
        if let Some(err) = &self.error {
            return crate::blocks::error_rows(&format!("cannot draw it: {err}"), cols);
        }
        let Some(protocol) = self.protocol() else {
            return if self.busy { vec![Row::text("drawing…", dim())] } else { vec![] };
        };
        let mut out = Vec::new();
        if self.draw == Draw::Compare {
            out.push(self.compare_label(cols, protocol.size().width as usize, focused));
        }
        let target = match self.draw {
            Draw::Line | Draw::Bar => Some(Target::Chart),
            Draw::Compare => Some(Target::Compare),
            _ => None,
        };
        out.extend((0..protocol.size().height).map(|k| Row { image: Some(k), target: target.clone(), ..Row::new() }));
        if self.is_chart() {
            let zoomed = self.zoom.map(|(start, len)| format!("   points {}–{} of {} · 0 shows all", start + 1, start + len, self.total())).unwrap_or_default();
            match self.hover {
                Some(k) => out.push(chart::tooltip(&self.block, k).add(zoomed, dim()).target(Target::Chart)),
                None if focused || !compact => {
                    let hint = if focused { "←→ values · + − zoom" } else { "hover for values · tab then + − zooms" };
                    out.push(Row::text(hint, dim()).add(zoomed, dim()).target(Target::Chart));
                }
                None => {}
            }
        }
        out
    }

    fn compare_label(&self, cols: usize, width: usize, focused: bool) -> Row {
        if !self.splits(cols) {
            let left = (cols - 2) / 2;
            let hint = if focused { "   s splits" } else { "" };
            return Row::text(format!("{:<w$}", "before", w = left + 2), bold(DIM)).add("after", bold(DIM)).add(hint, dim()).target(Target::Compare);
        }
        let at = ((self.split * width as f32).round() as usize).min(width);
        let hint = match (focused, cols >= compare::WIDE) {
            (true, true) => "h/l move the split · s side by side",
            (true, false) => "h/l move the split",
            _ => "drag, or tab then h/l, moves the split",
        };
        Row::text("◀ before", bold(if at > 0 { FG } else { DIM }))
            .add("  │  ", fg(ORANGE))
            .add("after ▶", bold(if at < width { FG } else { DIM }))
            .add(format!("   {hint} · {}%", (self.split * 100.0).round()), dim())
            .target(Target::Compare)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Frame, Rgba, RgbaImage};

    #[test]
    fn rows_round_to_the_nearest() {
        let font = FontSize::new(10, 20);
        assert_eq!((rows_for(405, font), rows_for(410, font), rows_for(5, font)), (20, 21, 1));
    }

    #[test]
    fn a_gif_keeps_every_frame_and_its_delay() {
        let path = std::env::temp_dir().join(format!("cockpit-show-{}.gif", std::process::id()));
        let mut gif = GifEncoder::new(std::fs::File::create(&path).unwrap());
        for (shade, ms) in [(0, 150), (128, 0), (255, 300)] {
            let frame = RgbaImage::from_pixel(40, 20, Rgba([shade, shade, shade, 255]));
            gif.encode_frame(Frame::from_parts(frame, 0, 0, Delay::from_numer_denom_ms(ms, 1))).unwrap();
        }
        drop(gif);
        let frames = image(&serde_json::json!({"path": path}), 400.0, FontSize::new(10, 20), &Picker::halfblocks()).unwrap();
        let delays: Vec<u128> = frames.iter().map(|(_, d)| d.as_millis()).collect();
        assert_eq!(delays, vec![150, 100, 300]);
        std::fs::remove_file(path).unwrap();
    }
}
