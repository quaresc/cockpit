//! The servers listening in a worktree, for the browser's new-tab page: every program with a TCP port open whose
//! working folder is inside the worktree.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

use cockpit_core::cleanup::{self, Process};
use serde_json::{Value, json};

/// `lsof -nP -iTCP -sTCP:LISTEN -Fpcn` output as (pid, command, port), one per program and port.
pub fn parse_listeners(text: &str) -> Vec<(u32, String, u16)> {
    let mut out: Vec<(u32, String, u16)> = Vec::new();
    let (mut pid, mut name) = (0u32, String::new());
    for line in text.lines() {
        let (tag, rest) = line.split_at(line.chars().next().map_or(0, char::len_utf8));
        match tag {
            "p" => (pid, name) = (rest.parse().unwrap_or(0), String::new()),
            "c" => name = rest.to_string(),
            "n" => {
                if let Some(port) = rest.rsplit(':').next().and_then(|p| p.parse::<u16>().ok())
                    && !out.iter().any(|(p, _, known)| *p == pid && *known == port)
                {
                    out.push((pid, name.clone(), port));
                }
            }
            _ => {}
        }
    }
    out
}

/// The listeners whose program works inside `root`, lowest port first: `[{port, command, worktree}]`.
pub fn inside(root: &Path, listeners: &[(u32, String, u16)], processes: &[Process]) -> Vec<Value> {
    let cwds: HashMap<u32, &Path> = processes.iter().map(|p| (p.pid, p.cwd.as_path())).collect();
    let mut found: Vec<(u16, &str)> =
        listeners.iter().filter(|(pid, _, _)| cwds.get(pid).is_some_and(|cwd| cwd.starts_with(root))).map(|(_, name, port)| (*port, name.as_str())).collect();
    found.sort();
    found.dedup_by_key(|(port, _)| *port);
    found.into_iter().map(|(port, command)| json!({"port": port, "command": command, "worktree": root})).collect()
}

/// The servers listening in a worktree now; empty when the system cannot say.
pub fn servers(root: &Path) -> Vec<Value> {
    let listed = Command::new("lsof").args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"]).stdin(Stdio::null()).stderr(Stdio::null()).output();
    let Ok(listed) = listed else { return Vec::new() };
    let listeners = parse_listeners(&String::from_utf8_lossy(&listed.stdout));
    if listeners.is_empty() {
        return Vec::new();
    }
    inside(root, &listeners, &cleanup::processes().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const LSOF: &str = "p410\ncnode\nf23\nn*:3000\nf24\nn[::1]:3000\np512\ncbeam.smp\nf9\nn127.0.0.1:4000\np600\ncpostgres\nf5\nn*:5432\n";

    #[test]
    fn listeners_are_read_once_per_program_and_port() {
        assert_eq!(parse_listeners(LSOF), [(410, "node".to_string(), 3000), (512, "beam.smp".to_string(), 4000), (600, "postgres".to_string(), 5432)]);
    }

    #[test]
    fn only_programs_working_inside_the_worktree_count() {
        let at = |pid, cwd: &str| Process { pid, name: String::new(), cwd: PathBuf::from(cwd) };
        let processes = [at(410, "/work/acme/web-eng-412/apps/site"), at(512, "/work/acme/api"), at(600, "/")];
        let found = inside(Path::new("/work/acme/web-eng-412"), &parse_listeners(LSOF), &processes);
        assert_eq!(found, [json!({"port": 3000, "command": "node", "worktree": "/work/acme/web-eng-412"})]);
        assert!(inside(Path::new("/work/acme/other"), &parse_listeners(LSOF), &processes).is_empty());
    }
}
