//! Pixels for the image blocks: SVG rasterized with resvg (system fonts loaded once, in the background), and pictures
//! fetched once, a URL downloaded into the cache under its SHA-1.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use image::{DynamicImage, RgbaImage};
use resvg::{tiny_skia, usvg};

use cockpit_core::files;

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

/// Draws every picture's text with this one font instead of the system's, before anything is drawn: snapshot tests
/// use it so a chart's pixels are the same on macOS and on the Linux CI. False when pictures were drawn already.
pub fn use_only_font(file: &std::path::Path) -> bool {
    let mut db = usvg::fontdb::Database::new();
    if db.load_font_file(file).is_err() {
        return false;
    }
    let family = db.faces().next().and_then(|face| face.families.first().map(|(name, _)| name.clone()));
    if let Some(family) = family {
        db.set_sans_serif_family(family.clone());
        db.set_serif_family(family.clone());
        db.set_monospace_family(family);
    }
    FONTS.set(Arc::new(db)).is_ok()
}

/// mimalloc's `mi_option_disallow_arena_alloc` (its place in v3's `mi_option_e`, which libmimalloc-sys does not name).
const DISALLOW_ARENA_ALLOC: libmimalloc_sys::mi_option_t = 26;

/// Has mimalloc take memory from the system segment by segment rather than from a reserved arena, so the pages a
/// collection frees leave the process (an arena keeps them resident); called first thing in main.
pub fn release_freed_pages() {
    // SAFETY: setting a mimalloc option, safe at any time.
    unsafe { libmimalloc_sys::mi_option_set(DISALLOW_ARENA_ALLOC, 1) };
}

/// Hands the memory a draw freed back to the system at once: its pixel buffers are large, and keeping them for reuse
/// would leave an idle pane holding tens of megabytes.
pub fn give_back() {
    // SAFETY: mimalloc's own collection, safe from any thread.
    unsafe { libmimalloc_sys::mi_collect(true) };
}

/// Starts reading the system fonts, so the first chart does not wait for them.
pub fn warm_up() {
    std::thread::spawn(fonts);
}

/// The SVG drawn at `width` pixels wide (its own size when None), straight alpha.
pub fn svg(text: &str, width: Option<f64>) -> Result<DynamicImage, String> {
    let options = usvg::Options { fontdb: fonts(), ..Default::default() };
    let tree = usvg::Tree::from_str(text, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = width.map_or(1.0, |w| w as f32 / size.width());
    let (w, h) = ((size.width() * scale).round().max(1.0) as u32, (size.height() * scale).round().max(1.0) as u32);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("empty picture")?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let data: Vec<u8> = pixmap.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }).collect();
    RgbaImage::from_raw(w, h, data).map(DynamicImage::ImageRgba8).ok_or_else(|| "bad pixels".into())
}

/// Whether a picture's bytes are an SVG, by its name or its first characters.
pub fn is_svg(path: &std::path::Path, bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).trim_start().to_string();
    path.extension().is_some_and(|e| e == "svg") || head.starts_with("<svg") || head.starts_with("<?xml")
}

/// A local path for a path or an http(s) URL, downloading the URL once.
pub fn fetch(source: &str) -> Result<PathBuf, String> {
    if !source.starts_with("http://") && !source.starts_with("https://") {
        return Ok(source.strip_prefix("~/").map_or_else(|| PathBuf::from(source), |rest| files::home().join(rest)));
    }
    let dir = files::cache().join("show/images");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(sha1_hex(source.as_bytes()));
    if !path.exists() {
        let part = path.with_extension("part");
        let done = std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "20", "-A", "cockpit", "-o"])
            .arg(&part)
            .arg(source)
            .output()
            .map_err(|e| format!("curl did not start: {e}"))?;
        if !done.status.success() {
            let _ = std::fs::remove_file(&part);
            return Err(String::from_utf8_lossy(&done.stderr).trim().to_string());
        }
        std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// SHA-1, so a URL always lands in the same cache file.
fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in message.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..20 => ((b & c) | (!b & d), 0x5A827999),
                20..40 => (b ^ c ^ d, 0x6ED9EBA1),
                40..60 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*word);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

