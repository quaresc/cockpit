//! The week view's picture: the SVG rasterized with resvg (system fonts read once, in the background) and encoded for
//! the terminal in a background thread, again whenever the pane's size, the minute, the week or the selection changes.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use image::{DynamicImage, RgbaImage};
use ratatui::layout::Size;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::Resize;
use resvg::{tiny_skia, usvg};

use crate::timeline;
use crate::week::{Clock, Week};

static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();

fn fonts() -> Arc<usvg::fontdb::Database> {
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// Starts reading the system fonts, so the first week view does not wait for them.
pub fn warm_up() {
    std::thread::spawn(fonts);
}

/// The SVG's pixels, straight alpha.
pub fn raster(text: &str) -> Result<DynamicImage, String> {
    let options = usvg::Options { fontdb: fonts(), ..Default::default() };
    let tree = usvg::Tree::from_str(text, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (w, h) = (size.width().round().max(1.0) as u32, size.height().round().max(1.0) as u32);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("empty picture")?;
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    let data: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    RgbaImage::from_raw(w, h, data).map(DynamicImage::ImageRgba8).ok_or_else(|| "bad pixels".into())
}

/// What a picture was drawn for: when any of it changes, it is drawn again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub cols: u16,
    pub rows: u16,
    pub minute: i64,
    pub version: u64,
    pub selected: Option<String>,
}

type Drawn = (Key, Result<Protocol, String>);

/// One picture is drawn at a time, the newest asked for winning.
static DRAWING: Mutex<()> = Mutex::new(());

pub struct Picture {
    pub picker: Picker,
    tx: Sender<Drawn>,
    rx: Receiver<Drawn>,
    asked: Option<Key>,
    pub shown: Option<(Key, Protocol)>,
    pub error: Option<String>,
}

impl Picture {
    pub fn new(picker: Picker) -> Self {
        let (tx, rx) = channel();
        Self { picker, tx, rx, asked: None, shown: None, error: None }
    }

    /// The font's cell in pixels.
    pub fn cell(&self) -> (f64, f64) {
        let font = self.picker.font_size();
        (font.width as f64, font.height as f64)
    }

    /// Draws the week for this key in the background, unless it is the one drawn or being drawn.
    pub fn want(&mut self, key: Key, week: &Week, clock: &Clock) {
        if self.asked.as_ref() == Some(&key) {
            return;
        }
        self.asked = Some(key.clone());
        let (cw, ch) = self.cell();
        let (week, clock, picker, tx) = (week.clone(), *clock, self.picker.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let _turn = DRAWING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let (w, h) = (key.cols as f64 * cw, key.rows as f64 * ch);
            let text = timeline::svg(&week, &clock, key.selected.as_deref(), w, h, (cw, ch));
            let drawn = raster(&text)
                .and_then(|image| picker.new_protocol(image, Size::new(key.cols, key.rows), Resize::Fit(None)).map_err(|e| e.to_string()));
            let _ = tx.send((key, drawn));
        });
    }

    /// Takes a picture drawn meanwhile, if it is still the one wanted; true when the screen changes.
    pub fn take(&mut self) -> bool {
        let mut changed = false;
        while let Ok((key, drawn)) = self.rx.try_recv() {
            if self.asked.as_ref() != Some(&key) {
                continue;
            }
            match drawn {
                Ok(protocol) => (self.shown, self.error) = (Some((key, protocol)), None),
                Err(err) => self.error = Some(err),
            }
            changed = true;
        }
        changed
    }

    pub fn is_drawing(&self) -> bool {
        self.asked.is_some() && self.asked.as_ref() != self.shown.as_ref().map(|(k, _)| k) && self.error.is_none()
    }
}
