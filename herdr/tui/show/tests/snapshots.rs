//! Every block type drawn at 80 and 120 columns from the specs under tests/fixtures, with pictures drawn as half blocks
//! and every background draw, read and run waited for; times are blanked, the clock not being the test's.

use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Once};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use image::codecs::gif::GifEncoder;
use image::{Delay, Frame, Rgba, RgbaImage};
use ratatui_image::picker::Picker;
use regex::Regex;

use cockpit_core::files::Density;
use cockpit_core::term::App;
use cockpit_show::ShowApp;

static TIME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d\d:\d\d(:\d\d)?\b").unwrap());

fn root() -> PathBuf {
    std::env::temp_dir().join(format!("cockpit-show-snapshots-{}", std::process::id()))
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn gradient(w: u32, h: u32, warm: bool) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let (a, b) = ((x * 255 / w) as u8, (y * 255 / h) as u8);
        if warm { Rgba([255, a, b, 255]) } else { Rgba([b, a, 255, 255]) }
    })
}

/// A sandbox HOME with the CI caches, the pictures the specs point at, and show.py's renderers found in the repo.
fn setup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let home = root().join("home");
        let cache = home.join(".cache/cockpit");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::copy(fixtures().join("ci-cache.json"), cache.join("ci.json")).unwrap();
        std::fs::copy(fixtures().join("inbox-cache.json"), cache.join("inbox.json")).unwrap();
        let pictures = root().join("pictures");
        std::fs::create_dir_all(&pictures).unwrap();
        gradient(320, 160, true).save(pictures.join("photo.png")).unwrap();
        gradient(400, 200, true).save(pictures.join("before.png")).unwrap();
        gradient(400, 200, false).save(pictures.join("after.png")).unwrap();
        let mut gif = GifEncoder::new(std::fs::File::create(pictures.join("anim.gif")).unwrap());
        for warm in [true, false, true] {
            gif.encode_frame(Frame::from_parts(gradient(120, 60, warm), 0, 0, Delay::from_numer_denom_ms(150, 1))).unwrap();
        }
        // SAFETY: set once, before any test thread reads the environment.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("COCKPIT_HERDR_DIR", Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
        }
    });
}

/// The fixture's view, ready: its spec in a folder of its own, everything it starts in the background come back.
fn app(fixture: &str, width: u16, density: Density, test: &str) -> ShowApp {
    setup();
    let dir = root().join(format!("{test}-{fixture}-{width}-{density:?}"));
    std::fs::create_dir_all(&dir).unwrap();
    let spec = std::fs::read_to_string(fixtures().join(format!("{fixture}.json"))).unwrap();
    std::fs::write(dir.join("view.json"), spec.replace("{fixtures}", &root().join("pictures").to_string_lossy())).unwrap();
    let mut app = ShowApp::new("view".into(), dir, Picker::halfblocks()).with_density(density);
    app.settle(width, 40);
    app
}

fn screen(app: &mut ShowApp, width: u16) -> String {
    TIME.replace_all(&cockpit_core::testing::render(app, width, 40), "hh:mm").into_owned()
}

fn press(app: &mut ShowApp, keys: &[KeyCode]) {
    for &code in keys {
        app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }
}

macro_rules! snapshots {
    ($($fixture:ident),*) => {$(
        #[test]
        fn $fixture() {
            for width in [80, 120] {
                let mut app = app(stringify!($fixture), width, Density::Comfortable, "plain");
                insta::assert_snapshot!(format!("{}_{width}", stringify!($fixture)), screen(&mut app, width));
            }
        }
    )*};
}

snapshots!(markdown, table, chart_line, chart_bar, flow, mermaid, sequence, image, compare, diff, log, ci, choice, confirm, mixed);

#[test]
fn mixed_compact() {
    for width in [80, 120] {
        let mut app = app("mixed", width, Density::Compact, "compact");
        insta::assert_snapshot!(format!("mixed_compact_{width}"), screen(&mut app, width));
    }
}

#[test]
fn diff_side_by_side_with_a_comment() {
    let mut app = app("diff", 120, Density::Comfortable, "comment");
    press(&mut app, &[KeyCode::Tab, KeyCode::Char('s'), KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('j'), KeyCode::Char('c')]);
    for c in "200 is the new pro quota".chars() {
        press(&mut app, &[KeyCode::Char(c)]);
    }
    insta::assert_snapshot!("diff_side_by_side_typing_120", screen(&mut app, 120));
    press(&mut app, &[KeyCode::Enter]);
    insta::assert_snapshot!("diff_side_by_side_sent_120", screen(&mut app, 120));
    let answer: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root().join("comment-diff-120-Comfortable/view.0.answer")).unwrap()).unwrap();
    assert_eq!(answer["type"], "comment");
    assert_eq!(answer["value"], serde_json::json!({"path": "src/quota.rs", "line": 3, "text": "200 is the new pro quota"}));
}

#[test]
fn chart_point_and_zoom() {
    let mut app = app("chart_line", 80, Density::Comfortable, "zoom");
    press(&mut app, &[KeyCode::Tab, KeyCode::Left, KeyCode::Left, KeyCode::Char('+')]);
    app.settle(80, 40);
    insta::assert_snapshot!("chart_line_zoomed_80", screen(&mut app, 80));
}

#[test]
fn log_scrolls_back_and_follows_again() {
    let mut app = app("log", 80, Density::Comfortable, "scroll");
    press(&mut app, &[KeyCode::Tab, KeyCode::Char('k'), KeyCode::Char('k'), KeyCode::Char('k')]);
    insta::assert_snapshot!("log_scrolled_80", screen(&mut app, 80));
    press(&mut app, &[KeyCode::Char('G')]);
    assert!(screen(&mut app, 80).contains("following the last 20 of 30"));
}
