//! A HOME of its own for the unit tests that read cockpit's cache, set once for the whole test process.

use std::path::PathBuf;
use std::sync::OnceLock;

/// The test HOME, made and set on first use; tests keep to their own folders under its cache.
pub fn home() -> PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("cockpit-inbox-tests-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("a temp folder");
        // SAFETY: only this call sets the variable, once, in the test binary; other threads at most read it.
        unsafe { std::env::set_var("HOME", &home) };
        home
    })
    .clone()
}
