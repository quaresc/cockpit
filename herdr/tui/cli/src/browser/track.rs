//! `cockpit browser track`: follows a host window on screen for the docked browser. Each line is one reading as JSON,
//! `{"host": {id, x, y, w, h} | null, "above": [{id, pid}]}`: the host's largest window and the windows in front of it,
//! nearest the viewer first. A line is printed when the reading changes.

use std::io::Write;
use std::time::Duration;

use serde_json::{Value, json};

/// A window as the system lists it: its number, its owner, its level and its frame in screen points from the top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Win {
    pub id: i64,
    pub pid: i64,
    pub layer: i64,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Smaller windows are tooltips and shadows, never a window to dock on or one that hides the browser.
const SMALLEST: f64 = 40.0;

/// One reading from the windows on screen, front to back: the host's largest ordinary window and what is in front of it.
pub fn reading(windows: &[Win], host: i64) -> Value {
    let ordinary = |w: &&Win| w.layer == 0 && w.w >= SMALLEST && w.h >= SMALLEST;
    let Some(found) = windows.iter().filter(ordinary).filter(|w| w.pid == host).max_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h))) else {
        return json!({"host": null, "above": []});
    };
    let above: Vec<Value> = windows.iter().take_while(|w| w.id != found.id).filter(ordinary).map(|w| json!({"id": w.id, "pid": w.pid})).collect();
    json!({"host": {"id": found.id, "x": found.x, "y": found.y, "w": found.w, "h": found.h}, "above": above})
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;

    use super::Win;

    type Ref = *const c_void;

    #[repr(C)]
    #[derive(Default)]
    struct Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative: u32) -> Ref;
        fn CGRectMakeWithDictionaryRepresentation(dict: Ref, rect: *mut Rect) -> bool;
        static kCGWindowNumber: Ref;
        static kCGWindowOwnerPID: Ref;
        static kCGWindowBounds: Ref;
        static kCGWindowLayer: Ref;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: Ref) -> isize;
        fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
        fn CFDictionaryGetValue(dict: Ref, key: Ref) -> Ref;
        fn CFNumberGetValue(number: Ref, kind: isize, out: *mut c_void) -> bool;
        fn CFRelease(object: Ref);
    }

    /// kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements.
    const ON_SCREEN: u32 = 1 | 16;
    /// kCFNumberSInt64Type.
    const INT64: isize = 4;

    fn number(dict: Ref, key: Ref) -> i64 {
        let mut out: i64 = 0;
        // SAFETY: `dict` is a live dictionary of the list being read, and CFNumberGetValue writes one i64 into `out`.
        unsafe {
            let found = CFDictionaryGetValue(dict, key);
            if !found.is_null() {
                CFNumberGetValue(found, INT64, (&raw mut out).cast());
            }
        }
        out
    }

    /// The windows on screen, front to back. Their frames need no permission; their titles, which do, are not read.
    pub fn windows() -> Vec<Win> {
        let mut all = Vec::new();
        // SAFETY: the list is owned by this call and released before it returns; every dictionary read belongs to it.
        unsafe {
            let list = CGWindowListCopyWindowInfo(ON_SCREEN, 0);
            if list.is_null() {
                return all;
            }
            for index in 0..CFArrayGetCount(list) {
                let dict = CFArrayGetValueAtIndex(list, index);
                let mut rect = Rect::default();
                let bounds = CFDictionaryGetValue(dict, kCGWindowBounds);
                if !bounds.is_null() {
                    CGRectMakeWithDictionaryRepresentation(bounds, &mut rect);
                }
                all.push(Win {
                    id: number(dict, kCGWindowNumber),
                    pid: number(dict, kCGWindowOwnerPID),
                    layer: number(dict, kCGWindowLayer),
                    x: rect.x,
                    y: rect.y,
                    w: rect.w,
                    h: rect.h,
                });
            }
            CFRelease(list);
        }
        all
    }
}

#[cfg(target_os = "macos")]
pub use mac::windows;

/// No window list where the docked browser does not run.
#[cfg(not(target_os = "macos"))]
pub fn windows() -> Vec<Win> {
    Vec::new()
}

/// Prints readings of `host`'s window until the parent goes away; `once` prints one and returns.
pub fn run(host: i64, once: bool) {
    let out = std::io::stdout();
    let mut last = Value::Null;
    loop {
        let now = reading(&windows(), host);
        if now != last {
            let mut out = out.lock();
            if writeln!(out, "{now}").is_err() || out.flush().is_err() {
                return;
            }
            last = now;
        }
        // The shell that started this is gone: nobody reads the next line.
        // SAFETY: getppid has no arguments and cannot fail.
        if once || unsafe { libc::getppid() } == 1 {
            return;
        }
        // A window on screen is followed frame by frame; one that is away is looked for a few times a second.
        std::thread::sleep(Duration::from_millis(if last["host"].is_null() { 200 } else { 16 }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: i64, pid: i64, layer: i64, w: f64, h: f64) -> Win {
        Win { id, pid, layer, x: 10.0, y: 20.0, w, h }
    }

    #[test]
    fn the_host_is_its_largest_ordinary_window_with_what_is_in_front() {
        let windows = [
            win(1, 50, 0, 300.0, 200.0),
            win(2, 77, 25, 1728.0, 30.0),
            win(3, 60, 0, 500.0, 400.0),
            win(4, 42, 0, 200.0, 100.0),
            win(5, 42, 0, 1280.0, 820.0),
            win(6, 9, 0, 900.0, 700.0),
        ];
        let read = reading(&windows, 42);
        assert_eq!(read["host"], json!({"id": 5, "x": 10.0, "y": 20.0, "w": 1280.0, "h": 820.0}));
        assert_eq!(
            read["above"],
            json!([{"id": 1, "pid": 50}, {"id": 3, "pid": 60}, {"id": 4, "pid": 42}]),
            "the menu bar's layer and what is behind are left out"
        );
    }

    #[test]
    fn a_host_with_no_window_on_screen_reads_as_none() {
        assert_eq!(reading(&[win(1, 50, 0, 300.0, 200.0)], 42), json!({"host": null, "above": []}));
        assert_eq!(reading(&[win(1, 42, 0, 20.0, 20.0)], 42)["host"], Value::Null, "a tooltip is no window to dock on");
    }
}
