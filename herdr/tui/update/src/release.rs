//! The release's archive for a version and machine, checked against the `.sha256` file published beside it.

use std::time::Duration;

use sha2::{Digest, Sha256};

/// The release's name for this machine.
pub fn target() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => return None,
    })
}

/// The archive's bytes once checked; None while the release has no archive or no checksum for it yet, since the
/// release workflow uploads them about 10 minutes after the version reaches main.
pub fn fetch(base: &str, version: &str, target: &str) -> Result<Option<Vec<u8>>, String> {
    let file = format!("cockpit-apps-{version}-{target}.tar.gz");
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(10))).timeout_global(Some(Duration::from_secs(300))).build().into();
    let Some(archive) = get(&agent, &format!("{base}/v{version}/{file}"), &file)? else { return Ok(None) };
    let Some(checksum) = get(&agent, &format!("{base}/v{version}/{file}.sha256"), &format!("{file}.sha256"))? else { return Ok(None) };
    let want = String::from_utf8_lossy(&checksum).split_whitespace().next().unwrap_or_default().to_lowercase();
    let got: String = Sha256::digest(&archive).iter().map(|b| format!("{b:02x}")).collect();
    if want != got {
        return Err("checksum mismatch".into());
    }
    Ok(Some(archive))
}

/// A file's bytes, None on a 404.
fn get(agent: &ureq::Agent, url: &str, file: &str) -> Result<Option<Vec<u8>>, String> {
    match agent.get(url).call() {
        Ok(mut response) => response.body_mut().with_config().limit(200 << 20).read_to_vec().map(Some).map_err(|e| format!("download of {file} failed: {e}")),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(ureq::Error::StatusCode(code)) => Err(format!("HTTP {code} for {file}")),
        Err(e) => Err(format!("download of {file} failed: {e}")),
    }
}
