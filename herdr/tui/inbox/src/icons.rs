//! The inbox's Nerd Font glyphs (FiraCode Nerd Font 3.0), the CI's own mark among them.

use std::sync::OnceLock;

static CI: OnceLock<&'static str> = OnceLock::new();

/// The CI's glyph, from the config's CI name: CircleCI's, GitHub Actions', GitLab's, else a generic run.
pub fn set_ci(name: &str) {
    let mark = match name.to_lowercase().as_str() {
        "circleci" => "\u{e78c}",
        "github" => "\u{eaff}",
        "gitlab" => "\u{f0ba0}",
        _ => "\u{eb9e}",
    };
    let _ = CI.set(mark);
}

pub fn icon(name: &str) -> &'static str {
    match name {
        "ci" => CI.get().copied().unwrap_or("\u{eb9e}"),
        "inbox" => "\u{f01c}",
        "review" => "\u{f441}",
        "pr" | "new" => "\u{f407}",
        "ticket" | "progress" => "\u{f41b}",
        "github" => "\u{f408}",
        "docker" => "\u{e7b0}",
        "preview" | "redeploy" => "\u{f499}",
        "rocket" | "released" => "\u{f427}",
        "fix" | "this" => "\u{f425}",
        "rerun" | "refresh" => "\u{f46a}",
        "open" => "\u{f465}",
        "mute" => "\u{f466}",
        "plan" => "\u{f400}",
        "continue" => "\u{f500}",
        "talk" => "\u{f086}",
        "done" | "approved" => "\u{f42e}",
        "sure?" => "\u{f420}",
        "alert" => "\u{f421}",
        "cancel" => "\u{f467}",
        "extend" | "waiting" => "\u{f43a}",
        "ArgoCD" => "\u{f0833}",
        "Dashboard" => "\u{f0e4}",
        "deploy" => "\u{f52e}",
        "stop" => "\u{f28d}",
        "today" => "\u{f185}",
        "calendar" => "\u{f455}",
        "dismiss" => "\u{f00d}",
        "ok" => "\u{f49e}",
        "fail" => "\u{f52f}",
        "running" => "\u{f4e3}",
        "changes" => "\u{f41f}",
        "fold" => "\u{f44a}",
        "unfold" => "\u{f47c}",
        "user" => "\u{f415}",
        "team" => "\u{f4fd}",
        "work" => "\u{f502}",
        "backlog" => "\u{f4a5}",
        _ => "·",
    }
}
