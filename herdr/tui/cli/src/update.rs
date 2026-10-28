//! `cockpit update-apps [herdr folder]`: installs the release's binary for the plugin's version into the folder's bin/,
//! the one fetch-apps.sh runs. It exits 3 when the install failed, so the script tells that from a binary without it.

use std::path::PathBuf;

use cockpit_core::files;

pub fn main(args: &[String]) {
    let dir = args.first().map(PathBuf::from).unwrap_or_else(files::herdr_dir);
    if let Err(error) = cockpit_update::update(&dir, &cockpit_update::releases()) {
        let version = cockpit_update::wanted(&dir).unwrap_or_default();
        eprintln!("cockpit: cockpit {version} could not be installed ({error}) → sh {}/fetch-apps.sh", dir.display());
        std::process::exit(3);
    }
}
