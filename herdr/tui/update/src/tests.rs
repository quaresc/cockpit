//! Updates against a fake release served over HTTP from a folder, on a herdr folder in a temp dir.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use super::*;

/// Files served from a folder, each requested path kept; anything missing is a 404.
struct Release {
    dir: tempfile::TempDir,
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Release {
    fn serve() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (root, seen) = (dir.path().to_path_buf(), requests.clone());
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let mut header = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    header.clear();
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                seen.lock().unwrap().push(path.clone());
                let (status, body) = match std::fs::read(root.join(path.trim_start_matches('/'))) {
                    Ok(body) => ("200 OK", body),
                    Err(_) => ("404 Not Found", b"Not Found".to_vec()),
                };
                let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(&body));
            }
        });
        Release { dir, url, requests }
    }

    fn asset(&self, version: &str) -> PathBuf {
        self.dir.path().join(format!("v{version}/cockpit-apps-{version}-{}.tar.gz", release::target().unwrap()))
    }

    /// Publishes an archive of these files, not executable, and its checksum file unless `checksum` is Some("").
    fn publish(&self, version: &str, files: &[(&str, &str)], checksum: Option<&str>) {
        let mut packed = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (name, body) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            packed.append_data(&mut header, name, body.as_bytes()).unwrap();
        }
        let bytes = packed.into_inner().unwrap().finish().unwrap();
        let asset = self.asset(version);
        std::fs::create_dir_all(asset.parent().unwrap()).unwrap();
        std::fs::write(&asset, &bytes).unwrap();
        let digest = match checksum {
            Some("") => return,
            Some(sum) => sum.to_string(),
            None => Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect(),
        };
        let name = asset.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(asset.with_file_name(format!("{name}.sha256")), format!("{digest}  {name}\n")).unwrap();
    }

    fn requests(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

/// A herdr folder on this version, with these files in bin/.
fn folder(version: &str, bin: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("herdr-plugin.toml"), format!("id = \"cockpit\"\nversion = \"{version}\"\nmin_herdr_version = \"0.7.5\"\n")).unwrap();
    for (name, body) in bin {
        let path = dir.path().join("bin").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_default()
}

#[test]
fn the_versions_come_from_the_manifest_and_bin() {
    let dir = folder("1.2.3", &[(".version", "1.2.2\n")]);
    assert_eq!(wanted(dir.path()).as_deref(), Some("1.2.3"));
    assert_eq!(installed(dir.path()).as_deref(), Some("1.2.2"));
    let empty = tempfile::tempdir().unwrap();
    assert_eq!((wanted(empty.path()), installed(empty.path())), (None, None));
}

#[test]
fn installs_the_release_once_per_version() {
    let (dir, release) = (folder("9.9.9", &[]), Release::serve());
    release.publish("9.9.9", &[("cockpit", "new")], None);
    assert_eq!(update(dir.path(), &release.url), Ok(Outcome::Installed("9.9.9".into())));
    assert_eq!((read(dir.path(), "bin/cockpit"), read(dir.path(), "bin/.version")), ("new".into(), "9.9.9\n".into()));
    assert!(folder::has_binary(dir.path()), "bin/cockpit is executable");
    assert_eq!(listing(dir.path()), ["bin", "herdr-plugin.toml"]);
    let seen = release.requests();
    assert_eq!(update(dir.path(), &release.url), Ok(Outcome::Current));
    assert_eq!(release.requests(), seen);
}

#[test]
fn a_new_version_replaces_the_whole_bin_folder() {
    let (dir, release) = (folder("9.9.9", &[("cockpit", "old"), ("cockpit-extra", "old"), (".version", "9.9.8\n")]), Release::serve());
    release.publish("9.9.9", &[("cockpit", "new")], None);
    assert_eq!(update(dir.path(), &release.url), Ok(Outcome::Installed("9.9.9".into())));
    assert_eq!(listing(&dir.path().join("bin")), [".version", "cockpit"]);
    assert_eq!(read(dir.path(), "bin/cockpit"), "new");
    assert_eq!(listing(dir.path()), ["bin", "herdr-plugin.toml"]);
}

#[test]
fn before_the_release_has_its_binary_an_empty_bin_gets_this_one() {
    let (dir, release) = (folder("9.9.9", &[]), Release::serve());
    let own = env!("CARGO_PKG_VERSION");
    assert_eq!(update(dir.path(), &release.url), Ok(Outcome::Copied(own.into())));
    assert_eq!(read(dir.path(), "bin/.version"), format!("{own}\n"));
    let copied = std::fs::metadata(dir.path().join("bin/cockpit")).unwrap();
    assert_eq!(copied.len(), std::fs::metadata(std::env::current_exe().unwrap()).unwrap().len());
    assert!(folder::has_binary(dir.path()));
    // Still due: the next check installs the release once it is out.
    release.publish("9.9.9", &[("cockpit", "new")], None);
    assert_eq!(update(dir.path(), &release.url), Ok(Outcome::Installed("9.9.9".into())));
    assert_eq!(read(dir.path(), "bin/cockpit"), "new");
}

#[test]
fn before_the_release_has_its_binary_a_bin_with_one_stays() {
    let (dir, release) = (folder("9.9.9", &[("cockpit", "old"), (".version", "9.9.8\n")]), Release::serve());
    assert_eq!(update(dir.path(), &release.url), Ok(Outcome::Unpublished));
    release.publish("9.9.9", &[("cockpit", "new")], Some(""));
    assert_eq!(update(dir.path(), &release.url), Ok(Outcome::Unpublished), "an archive whose checksum is not up yet waits too");
    assert_eq!((read(dir.path(), "bin/cockpit"), read(dir.path(), "bin/.version")), ("old".into(), "9.9.8\n".into()));
    assert_eq!(listing(dir.path()), ["bin", "herdr-plugin.toml"]);
}

#[test]
fn a_wrong_checksum_or_archive_installs_nothing() {
    let (dir, release) = (folder("9.9.9", &[("cockpit", "old"), (".version", "9.9.8\n")]), Release::serve());
    release.publish("9.9.9", &[("cockpit", "new")], Some(&"0".repeat(64)));
    assert_eq!(update(dir.path(), &release.url), Err("checksum mismatch".into()));
    release.publish("9.9.9", &[("cockpit-hub", "new")], None);
    assert_eq!(update(dir.path(), &release.url), Err("no cockpit binary in the archive".into()));
    assert_eq!(read(dir.path(), "bin/cockpit"), "old");
    assert_eq!(listing(dir.path()), ["bin", "herdr-plugin.toml"]);
}

#[test]
fn an_unreachable_release_says_so() {
    let dir = folder("9.9.9", &[]);
    let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let error = update(dir.path(), &format!("http://{closed}")).unwrap_err();
    assert!(error.starts_with("download of cockpit-apps-9.9.9-"), "{error}");
    assert_eq!(listing(dir.path()), ["herdr-plugin.toml"]);
}

#[test]
fn a_dev_build_or_another_update_leaves_the_folder_alone() {
    let (dir, release) = (folder("9.9.9", &[]), Release::serve());
    release.publish("9.9.9", &[("cockpit", "new")], None);
    std::fs::create_dir(dir.path().join("fetch.lock")).unwrap();
    assert_eq!(update(dir.path(), &release.url), Ok(Outcome::Busy));
    std::fs::remove_dir(dir.path().join("fetch.lock")).unwrap();
    std::fs::create_dir_all(dir.path().join("tui/target/release")).unwrap();
    std::fs::write(dir.path().join("tui/target/release/cockpit"), "").unwrap();
    assert_eq!(update(dir.path(), &release.url), Ok(Outcome::Current));
    assert_eq!(release.requests(), 0);
    assert!(!dir.path().join("bin").exists());
}

#[test]
fn the_hub_hands_over_to_a_binary_on_the_plugins_version_only() {
    let script = |says: &str| format!("#!/bin/sh\necho 'cockpit {says}'\n");
    let dir = folder("9.9.9", &[("cockpit", &script("9.9.9")), (".version", "9.9.9\n")]);
    assert_eq!(watch::newer_binary(dir.path()), Some(dir.path().join("bin/cockpit")));
    let lying = folder("9.9.9", &[("cockpit", &script("9.9.8")), (".version", "9.9.9\n")]);
    assert_eq!(watch::newer_binary(lying.path()), None);
    let behind = folder("9.9.9", &[("cockpit", &script("9.9.8")), (".version", "9.9.8\n")]);
    assert_eq!(watch::newer_binary(behind.path()), None);
    let own = env!("CARGO_PKG_VERSION");
    let same = folder(own, &[("cockpit", &script(own)), (".version", &format!("{own}\n"))]);
    assert_eq!(watch::newer_binary(same.path()), None);
}

#[test]
fn the_space_apps_restarted_are_those_of_this_folders_binary() {
    let (dir, other) = (folder("9.9.9", &[("cockpit", "")]), folder("9.9.9", &[("cockpit", "")]));
    let link = dir.path().with_extension("link");
    std::os::unix::fs::symlink(dir.path(), &link).unwrap();
    let ours = dir.path().join("bin/cockpit").canonicalize().unwrap();
    let (here, through_link, elsewhere) = (dir.path().join("bin/cockpit"), link.join("bin/cockpit"), other.path().join("bin/cockpit"));
    let listed = format!(
        "  101 {} inbox cockpit-space\n  102 {} runbook cockpit-space\n 103 {} todo cockpit-space\n104 {} show demo\n105 {} hub\n106 bin/cockpit meetings cockpit-space\n107 sh app.sh inbox cockpit-space\n",
        here.display(),
        through_link.display(),
        elsewhere.display(),
        here.display(),
        here.display()
    );
    assert_eq!(watch::space_apps(&listed, &ours), [101, 102]);
    std::fs::remove_file(link).unwrap();
}
