//! The agents a palette lists, in the order every palette shares (the TUI popup and both desktop layouts): by stage,
//! waiting on you first (the longest waiting first), then the rest stage by stage, the latest change first in each.

use serde_json::{Value, json};

/// Stages in the order the palette lists them: what waits on you, then work under way, then what rests.
const ORDER: [&str; 9] = ["needs_you", "failed", "fixing", "done", "working", "review", "idle", "merged", "snoozed"];

/// The stages that wait on you: every agent at one is listed, the longest waiting first.
const WAITING: [&str; 3] = ["needs_you", "failed", "fixing"];

/// How many agents an empty query lists when that many run; past it only waiting ones are added.
pub const LISTED: usize = 8;

/// An agent as a palette lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Jump {
    /// The handle every agents action takes (`app:t3`, a herdr pane id).
    pub terminal: String,
    /// Its frontend's own id (`t3`), what a desktop focuses.
    pub address: String,
    pub title: String,
    pub status: String,
    pub stage: String,
    /// It waits on you: it needs an answer, stopped with an error, or its PR needs fixing.
    pub waiting: bool,
    /// A session cockpit drives: no terminal to focus, its chat is where it is.
    pub driven: bool,
    /// One line: whose move, for how long, and where ("needs you · 6m · api › quotas").
    pub help: String,
    /// Herdr's icon for its agent (`claude`, `codex`), `agent` when it has none.
    pub icon: String,
    /// The plain agent word the TUI puts beside its task.
    pub mark: String,
}

impl Jump {
    pub fn json(&self) -> Value {
        json!({"terminal": self.terminal, "address": self.address, "title": self.title, "status": self.status, "stage": self.stage,
               "waiting": self.waiting, "driven": self.driven, "help": self.help, "icon": self.icon})
    }
}

/// The stage of an agents view row; a row without one gets it from its status.
fn stage_of(row: &Value) -> String {
    if let Some(stage) = row["stage"].as_str().filter(|s| !s.is_empty()) {
        return stage.to_string();
    }
    match row["status"].as_str().unwrap_or_default() {
        "blocked" => "needs_you",
        "failed" | "error" => "failed",
        "done" => "done",
        "working" => "working",
        _ => "idle",
    }
    .to_string()
}

/// "<1m", "6m", "3h", "2d": the same short ages the desktop draws.
pub fn short(ms: u64) -> String {
    let m = ms / 60_000;
    match m {
        0 => "<1m".into(),
        1..=59 => format!("{m}m"),
        _ if m / 60 < 48 => format!("{}h", m / 60),
        _ => format!("{}d", m / 60 / 24),
    }
}

fn word(row: &Value, stage: &str) -> String {
    match stage {
        "needs_you" => "needs you".into(),
        "failed" => "stopped with an error".into(),
        "fixing" => "needs fixing".into(),
        "done" if row["interrupted"] == true => "interrupted".into(),
        "review" => "in review".into(),
        "snoozed" => row["snoozed"]["label"].as_str().map(|l| format!("snoozed {l}")).unwrap_or_else(|| "snoozed".into()),
        other => other.replace('_', " "),
    }
}

/// Every agent of an agents view model (of one frontend, or all), in the palette's order: stage first, then within a
/// stage the longest waiting first for those waiting on you and the latest change first for the others.
pub fn jumps(agents: &Value, frontend: Option<&str>, now: u64) -> Vec<Jump> {
    let table = cockpit_core::agent::all();
    let rank = |stage: &str| ORDER.iter().position(|s| *s == stage).unwrap_or(ORDER.len());
    let mut rows: Vec<(usize, u64, Jump)> = agents["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| frontend.is_none_or(|f| r["frontend"].as_str().is_none_or(|rf| rf == f)))
        .filter_map(|r| {
            let terminal = r["terminal"].as_str().filter(|t| !t.is_empty())?.to_string();
            let stage = stage_of(r);
            let since = r["since"].as_u64().or_else(|| r["waiting_since"].as_u64());
            let text = |v: &Value| v.as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
            let place = match (text(&r["space"]["name"]), text(&r["tab"]["name"])) {
                (Some(space), Some(tab)) => format!("{space} › {tab}"),
                (space, tab) => space.or(tab).or_else(|| text(&r["repo"])).unwrap_or_default(),
            };
            let help = [Some(word(r, &stage)), since.map(|s| short(now.saturating_sub(s))), Some(place).filter(|p| !p.is_empty())];
            // The row's own icon, else one from the agent it runs; a row naming none gets the neutral one.
            let icon = text(&r["icon"]).unwrap_or_else(|| {
                let id = text(&r["agent"]).or_else(|| text(&r["program"])).unwrap_or_default();
                if id.is_empty() { "agent".into() } else { cockpit_core::agent::icon_in(&table, &id).to_string() }
            });
            let jump = Jump {
                address: text(&r["address"]).unwrap_or_else(|| terminal.trim_start_matches("app:").to_string()),
                title: text(&r["title"]).or_else(|| text(&r["tab"]["name"])).unwrap_or_else(|| terminal.clone()),
                status: r["status"].as_str().unwrap_or_default().to_string(),
                waiting: WAITING.contains(&stage.as_str()),
                driven: r["driven"] == true,
                help: help.into_iter().flatten().collect::<Vec<_>>().join(" · "),
                mark: cockpit_core::agent::mark_in(&table, &text(&r["agent"]).or_else(|| text(&r["program"])).unwrap_or_else(|| icon.clone())),
                icon,
                stage,
                terminal,
            };
            Some((rank(&jump.stage), since.unwrap_or(now), jump))
        })
        .collect();
    // Waiting the longest first; elsewhere the latest change first.
    rows.sort_by(|(ra, sa, a), (rb, sb, b)| ra.cmp(rb).then(if a.waiting { sa.cmp(sb) } else { sb.cmp(sa) }).then(a.title.cmp(&b.title)));
    rows.into_iter().map(|(_, _, j)| j).collect()
}

/// How many of `jumps` an empty query lists, from the top of their order: every waiting one, and the next ones until
/// `LISTED` are listed.
pub fn listed(jumps: &[Jump]) -> usize {
    jumps.iter().filter(|j| j.waiting).count().max(LISTED.min(jumps.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 10_000_000;

    fn row(terminal: &str, stage: &str, since: u64) -> Value {
        json!({"terminal": terminal, "address": terminal.trim_start_matches("app:"), "frontend": "app", "title": terminal, "stage": stage,
               "status": "idle", "since": since, "space": {"name": "api"}, "tab": {"name": "quotas"}})
    }

    #[test]
    fn waiting_first_the_longest_first_then_the_latest() {
        let model = json!({"agents": [
            row("app:t1", "idle", NOW - 60_000),
            row("app:t2", "working", NOW - 600_000),
            row("app:t3", "needs_you", NOW - 120_000),
            row("app:t4", "done", NOW - 300_000),
            row("app:t5", "needs_you", NOW - 360_000),
            row("app:t6", "working", NOW - 30_000),
            row("app:t7", "failed", NOW - 10_000),
            json!({"terminal": "w1:p2", "frontend": "tui", "stage": "needs_you", "since": 1}),
        ]});
        let order: Vec<String> = jumps(&model, Some("app"), NOW).into_iter().map(|j| j.address).collect();
        assert_eq!(order, ["t5", "t3", "t7", "t4", "t6", "t2", "t1"]);
        assert_eq!(jumps(&model, None, NOW).len(), 8, "no frontend: every agent");
    }

    #[test]
    fn a_line_says_whose_move_how_long_and_where() {
        let model = json!({"agents": [row("app:t3", "needs_you", NOW - 360_000), json!({"terminal": "app:t9", "status": "blocked"})]});
        let list = jumps(&model, None, NOW);
        assert_eq!((list[0].help.as_str(), list[0].waiting), ("needs you · 6m · api › quotas", true));
        assert_eq!((list[1].stage.as_str(), list[1].help.as_str(), list[1].title.as_str()), ("needs_you", "needs you", "app:t9"), "a stage from its status");
        assert_eq!(list[1].icon, "agent", "no agent named: the neutral icon");
        let named = json!({"agents": [{"terminal": "app:t1", "icon": "codex"}, {"terminal": "app:t2", "agent": "", "program": "agy"}]});
        let icons: Vec<String> = jumps(&named, None, NOW).into_iter().map(|j| j.icon).collect();
        assert!(icons.contains(&"codex".to_string()) && icons.contains(&"antigravity".to_string()), "the row's own, else its program's: {icons:?}");
    }

    #[test]
    fn an_empty_query_lists_every_waiting_agent_and_the_latest_others() {
        let many: Vec<Value> = (0..12).map(|i| row(&format!("app:t{i}"), if i < 3 { "needs_you" } else { "idle" }, NOW - i * 1000)).collect();
        assert_eq!(listed(&jumps(&json!({"agents": many}), None, NOW)), LISTED);
        let waiting: Vec<Value> = (0..11).map(|i| row(&format!("app:t{i}"), "needs_you", NOW - i * 1000)).collect();
        assert_eq!(listed(&jumps(&json!({"agents": waiting}), None, NOW)), 11);
        assert_eq!(listed(&jumps(&json!({"agents": [row("app:t1", "idle", 0)]}), None, NOW)), 1);
    }

    #[test]
    fn ages_read_as_the_desktop_draws_them() {
        assert_eq!([short(10_000), short(6 * 60_000), short(3 * 3_600_000), short(50 * 3_600_000)], ["<1m", "6m", "3h", "2d"]);
    }
}
