//! Programs cockpit pins to one version and one checksum and downloads once: the Chromium side browser's Electron and
//! the agent-browser tool that drives it.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256, Sha512};

/// A file's bytes over HTTPS, at most 400 MiB of them.
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let name = url.rsplit('/').next().unwrap_or(url);
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).timeout_global(Some(Duration::from_secs(900))).build().into();
    match agent.get(url).call() {
        Ok(mut response) => response.body_mut().with_config().limit(400 << 20).read_to_vec().map_err(|e| format!("download of {name} failed: {e}")),
        Err(ureq::Error::StatusCode(code)) => Err(format!("HTTP {code} for {name}")),
        Err(e) => Err(format!("download of {name} failed: {e}")),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn sha512(bytes: &[u8]) -> String {
    hex(&Sha512::digest(bytes))
}

/// Whether the bytes have the pinned checksum: 64 hex digits name a SHA-256, 128 a SHA-512.
pub fn matches(bytes: &[u8], pinned: &str) -> bool {
    let pinned = pinned.trim().to_lowercase();
    match pinned.len() {
        64 => sha256(bytes) == pinned,
        128 => sha512(bytes) == pinned,
        _ => false,
    }
}

/// One file out of a gzipped tar archive, written to `to` as an executable.
pub fn untar_member(archive: &[u8], member: &str, to: &Path) -> Result<(), String> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries().map_err(|_| "unreadable archive".to_string())? {
        let mut entry = entry.map_err(|_| "unreadable archive".to_string())?;
        if entry.path().ok().is_some_and(|p| p == Path::new(member)) {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            if let Some(dir) = to.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(to, bytes).map_err(|e| e.to_string())?;
            use std::os::unix::fs::PermissionsExt;
            return std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string());
        }
    }
    Err(format!("the archive holds no {member}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checksum_is_told_by_its_length() {
        assert!(matches(b"acme", &sha256(b"acme")) && matches(b"acme", &sha512(b"acme").to_uppercase()));
        assert!(!matches(b"acme", &sha256(b"other")) && !matches(b"acme", "abc"));
    }

    #[test]
    fn one_member_comes_out_of_an_archive_executable() {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (path, body) in [("package/bin/tool-darwin-arm64", &b"#!/bin/sh\n"[..]), ("package/readme", b"hello")] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, body).unwrap();
        }
        let archive = tar.into_inner().unwrap().finish().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("deep/tool");
        untar_member(&archive, "package/bin/tool-darwin-arm64", &to).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::read(&to).unwrap(), b"#!/bin/sh\n");
        assert_eq!(std::fs::metadata(&to).unwrap().permissions().mode() & 0o111, 0o111);
        assert_eq!(untar_member(&archive, "package/bin/nope", &to), Err("the archive holds no package/bin/nope".into()));
    }
}
