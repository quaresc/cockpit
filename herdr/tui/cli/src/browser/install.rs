//! The Chromium side browser's two programs, pinned in `browser/pins.json`: Electron, which runs the shell, and
//! agent-browser, which agents drive pages with. Each is downloaded once into ~/.cache/cockpit/browser and checked.

use std::path::{Path, PathBuf};

use cockpit_core::{browser, files};
use cockpit_update::pinned;
use serde_json::Value;

/// One pinned program: its version, where it is downloaded from, and its checksum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub version: String,
    pub url: String,
    pub checksum: String,
}

/// What `browser/pins.json` pins for this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pins {
    pub electron: Pin,
    pub agent: Pin,
    /// The agent-browser program's path inside its archive.
    pub member: String,
}

/// This machine's name in the pinned downloads.
pub fn arch() -> &'static str {
    if std::env::consts::ARCH == "aarch64" { "arm64" } else { "x64" }
}

/// The shell's folder: the checkout's browser/ beside the herdr plugin folder, else the one the release packs beside
/// this binary; COCKPIT_BROWSER_DIR names another one, for a build that lives outside its checkout.
pub fn shell_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("COCKPIT_BROWSER_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    let herdr = files::herdr_dir();
    let herdr = std::fs::canonicalize(&herdr).unwrap_or(herdr);
    let exe = std::env::current_exe().map(|e| std::fs::canonicalize(&e).unwrap_or(e)).unwrap_or_default();
    let packed = exe.parent().map(|p| p.join("browser")).unwrap_or_default();
    [herdr.join("browser"), herdr.parent().map(|p| p.join("browser")).unwrap_or_default(), packed]
        .into_iter()
        .find(|p| p.join("main/index.js").exists())
        .unwrap_or_default()
}

/// The pins as written, for one machine; an error names the field that is missing.
pub fn parse(pins: &Value, arch: &str) -> Result<Pins, String> {
    let text = |node: &Value, field: &str, of: &str| {
        node[field].as_str().filter(|s| !s.is_empty()).map(str::to_string).ok_or_else(|| format!("pins.json: {of} has no {field}"))
    };
    let fill = |template: String, version: &str| template.replace("{version}", version).replace("{arch}", arch);
    let (electron, agent) = (&pins["electron"], &pins["agentBrowser"]);
    let version = text(electron, "version", "electron")?;
    let checksum =
        electron["checksum"][arch].as_str().filter(|s| !s.is_empty()).ok_or_else(|| format!("pins.json: electron has no checksum for {arch}"))?.to_string();
    let electron = Pin { url: fill(text(electron, "url", "electron")?, &version), version, checksum };
    let version = text(agent, "version", "agentBrowser")?;
    let member = fill(text(agent, "member", "agentBrowser")?, &version);
    let agent = Pin { url: fill(text(agent, "url", "agentBrowser")?, &version), checksum: text(agent, "checksum", "agentBrowser")?, version };
    Ok(Pins { electron, agent, member })
}

/// The pins of the shell that ships with this cockpit.
pub fn pins() -> Result<Pins, String> {
    let file = shell_dir().join("pins.json");
    let json = files::load(&file).ok_or_else(|| format!("no readable {}", file.display()))?;
    parse(&json, arch())
}

/// The Electron program: COCKPIT_BROWSER_ELECTRON names another one, for a checkout that borrows one.
pub fn electron(pins: &Pins) -> PathBuf {
    match std::env::var("COCKPIT_BROWSER_ELECTRON") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => browser::cache().join("electron").join(&pins.electron.version).join("Electron.app/Contents/MacOS/Electron"),
    }
}

/// The agent-browser program: COCKPIT_AGENT_BROWSER names another one.
pub fn agent_browser(pins: &Pins) -> PathBuf {
    match std::env::var("COCKPIT_AGENT_BROWSER") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => browser::cache().join("agent-browser").join(&pins.agent.version).join("agent-browser"),
    }
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Whether each program is there: Electron, then agent-browser.
pub fn installed(pins: &Pins) -> (bool, bool) {
    (executable(&electron(pins)), executable(&agent_browser(pins)))
}

/// Unpacks a zip into a folder, keeping the links and permissions an app bundle needs.
#[cfg(target_os = "macos")]
fn unzip(zip: &Path, to: &Path) -> Result<(), String> {
    let done = std::process::Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(zip).arg(to).output().map_err(|e| format!("ditto: {e}"))?;
    if done.status.success() { Ok(()) } else { Err(format!("ditto: {}", String::from_utf8_lossy(&done.stderr).trim())) }
}

#[cfg(not(target_os = "macos"))]
fn unzip(_zip: &Path, _to: &Path) -> Result<(), String> {
    Err(browser::MAC_ONLY.into())
}

/// Downloads what is missing, checks it against its pin and puts it in place; `say` gets a line per step.
pub fn install(pins: &Pins, say: &mut dyn FnMut(&str)) -> Result<(), String> {
    let (has_electron, has_agent) = installed(pins);
    if !has_electron {
        say(&format!("downloading Electron {} ({})", pins.electron.version, arch()));
        let bytes = pinned::download(&pins.electron.url)?;
        if !pinned::matches(&bytes, &pins.electron.checksum) {
            return Err(format!("Electron {}: the download does not have the pinned checksum", pins.electron.version));
        }
        let dest = browser::cache().join("electron").join(&pins.electron.version);
        let staging = dest.with_extension("staging");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
        let zip = staging.join("electron.zip");
        std::fs::write(&zip, &bytes).map_err(|e| e.to_string())?;
        drop(bytes);
        let unpacked = unzip(&zip, &staging);
        let _ = std::fs::remove_file(&zip);
        unpacked.inspect_err(|_| drop(std::fs::remove_dir_all(&staging)))?;
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&staging, &dest).map_err(|e| e.to_string())?;
        say(&format!("installed Electron {} in {}", pins.electron.version, dest.display()));
    }
    if !has_agent {
        say(&format!("downloading agent-browser {}", pins.agent.version));
        let bytes = pinned::download(&pins.agent.url)?;
        if !pinned::matches(&bytes, &pins.agent.checksum) {
            return Err(format!("agent-browser {}: the download does not have the pinned checksum", pins.agent.version));
        }
        let to = browser::cache().join("agent-browser").join(&pins.agent.version).join("agent-browser");
        pinned::untar_member(&bytes, &pins.member, &to)?;
        say(&format!("installed agent-browser {} in {}", pins.agent.version, to.display()));
    }
    // The versions before these are of no use to anyone.
    for (folder, keep) in [("electron", &pins.electron.version), ("agent-browser", &pins.agent.version)] {
        for entry in std::fs::read_dir(browser::cache().join(folder)).into_iter().flatten().flatten() {
            if entry.file_name().to_string_lossy() != *keep.as_str() {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn written() -> Value {
        json!({
            "electron": {"version": "44.5.1", "url": "https://dl.example.com/v{version}/electron-v{version}-darwin-{arch}.zip", "checksum": {"arm64": "aa", "x64": "bb"}},
            "agentBrowser": {"version": "0.38.1", "url": "https://dl.example.com/agent-browser-{version}.tgz", "member": "package/bin/agent-browser-darwin-{arch}", "checksum": "cc"},
        })
    }

    #[test]
    fn the_pins_are_read_for_one_machine() {
        let pins = parse(&written(), "arm64").unwrap();
        assert_eq!(
            pins.electron,
            Pin { version: "44.5.1".into(), url: "https://dl.example.com/v44.5.1/electron-v44.5.1-darwin-arm64.zip".into(), checksum: "aa".into() }
        );
        assert_eq!(
            (pins.agent.url.as_str(), pins.member.as_str()),
            ("https://dl.example.com/agent-browser-0.38.1.tgz", "package/bin/agent-browser-darwin-arm64")
        );
        assert_eq!(parse(&written(), "x64").unwrap().electron.checksum, "bb");
    }

    #[test]
    fn a_pin_with_a_field_missing_is_refused_by_name() {
        let mut pins = written();
        pins["electron"]["checksum"] = json!({"x64": "bb"});
        assert_eq!(parse(&pins, "arm64"), Err("pins.json: electron has no checksum for arm64".into()));
        pins = written();
        pins["agentBrowser"]["member"] = json!("");
        assert_eq!(parse(&pins, "arm64"), Err("pins.json: agentBrowser has no member".into()));
    }

    #[test]
    fn the_pins_that_ship_are_whole() {
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../browser/pins.json");
        let json = files::load(&file).expect("browser/pins.json");
        for arch in ["arm64", "x64"] {
            let pins = parse(&json, arch).unwrap();
            assert_eq!(pins.electron.checksum.len(), 64);
            assert_eq!(pins.agent.checksum.len(), 128);
            assert!(
                pins.electron.url.starts_with("https://github.com/electron/electron/releases/download/v")
                    && pins.electron.url.ends_with(&format!("-darwin-{arch}.zip"))
            );
        }
    }
}
