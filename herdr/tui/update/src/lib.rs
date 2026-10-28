//! Installs the release's `cockpit` binary for the plugin's version and this machine into the herdr folder's bin/:
//! `cockpit update-apps` once per version (fetch-apps.sh runs it), and the hub every 10 minutes while it runs.

mod folder;
mod install;
mod release;
mod watch;

use std::path::Path;

pub use folder::{dev_build, installed, wanted};
pub use watch::{keep_current, restart_apps};

/// Where the releases' files are, `<base>/v<version>/<file>`; COCKPIT_RELEASES_URL points elsewhere for tests.
pub fn releases() -> String {
    std::env::var("COCKPIT_RELEASES_URL").unwrap_or_else(|_| "https://github.com/quaresc/cockpit/releases/download".into())
}

/// What an update did.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// bin/ holds the plugin's version, a dev build runs the folder, or nothing is built for this machine.
    Current,
    /// Another update holds the folder's lock.
    Busy,
    /// The release has no binary for the version yet: bin/ stays as it is until it does.
    Unpublished,
    /// The release has no binary yet and bin/ had none: this binary, on this version, went in so the spaces run.
    Copied(String),
    /// The plugin's version went into bin/.
    Installed(String),
}

/// Brings `dir`'s bin/ to the plugin's version from the releases at `base`; an error says what failed.
pub fn update(dir: &Path, base: &str) -> Result<Outcome, String> {
    let Some(version) = wanted(dir) else { return Ok(Outcome::Current) };
    if installed(dir).as_deref() == Some(version.as_str()) || dev_build(dir) {
        return Ok(Outcome::Current);
    }
    let Some(target) = release::target() else { return Ok(Outcome::Current) };
    let Some(_lock) = folder::Lock::take(dir) else { return Ok(Outcome::Busy) };
    match release::fetch(base, &version, target)? {
        Some(archive) => {
            install::unpack(dir, &archive, &version)?;
            Ok(Outcome::Installed(version))
        }
        None if folder::has_binary(dir) => Ok(Outcome::Unpublished),
        None => {
            let own = env!("CARGO_PKG_VERSION");
            install::copy_own(dir, own)?;
            Ok(Outcome::Copied(own.into()))
        }
    }
}

#[cfg(test)]
mod tests;
