//! The Fix and Fix all buttons of Cockpit settings' Health tab: a check of the last `cockpit doctor --json` run fixed by
//! its command, typed into a tab of the actions space, or by the hub action it names.

use cockpit_core::files;
use serde_json::Value;

use super::{Ctx, arg, sources};
use crate::value::{Res, text};

/// The kept report, when this cockpit wrote it and it is the one the person looked at (`at`).
fn report(at: u64) -> Res<Value> {
    let report = files::load(&files::cache().join("doctor.json")).ok_or("no doctor report yet: run cockpit doctor --json")?;
    let ours = env!("CARGO_PKG_VERSION");
    if report["cockpit"] != ours {
        return Err(format!("the last report is from cockpit {}, this is {ours}: run the checks again", report["cockpit"].as_str().unwrap_or("?")));
    }
    if report["at"].as_u64() != Some(at) {
        return Err("the checks ran again since: look at what they say now".into());
    }
    Ok(report)
}

/// The hub actions a doctor check's fix may name: those that do what they say at once, without asking first.
const ALLOWED: [(&str, &str); 2] = [("sources", "refresh"), ("sources", "sign_in")];

/// A fix as the person saw it: its command, or its action with its arguments (`app action [args]`).
pub fn shown(fix: &Value) -> String {
    match fix["command"].as_str() {
        Some(command) => command.to_string(),
        None => format!(
            "{} {} {}",
            fix["act"]["app"].as_str().unwrap_or_default(),
            fix["act"]["action"].as_str().unwrap_or_default(),
            serde_json::to_string(fix["act"]["args"].as_array().unwrap_or(&Vec::new())).unwrap_or_default()
        ),
    }
}

/// What a fix's command does in a sandbox, whose terminals run on the real HOME (`COCKPIT_HC_BIN` names its hc): shown,
/// not run. None outside one.
fn sandboxed(in_sandbox: bool, command: &str) -> Option<String> {
    in_sandbox.then(|| format!("In a sandbox a fix's command is shown, not run: {command}"))
}

/// The fix the kept report gives check `id`, refused unless it is still the one the person saw (`seen`).
fn checked<'a>(report: &'a Value, id: &str, seen: &str) -> Res<&'a Value> {
    let check = report["checks"].as_array().into_iter().flatten().find(|c| c["id"] == id).ok_or_else(|| format!("no check {id} in the last report"))?;
    if shown(&check["fix"]) != seen {
        return Err("The checks changed: look again".into());
    }
    Ok(&check["fix"])
}

/// What one fix did: ran (its note), or, in a sandbox, its command shown and not run.
#[derive(Debug, PartialEq)]
enum Done {
    Ran(String),
    Shown(String),
}

/// Runs check `id`'s fix as the report gives it: its hub action when allowed, else its command typed into a tab of the
/// actions space. Only the doctor's own commands and a few hub actions run, never one the caller sends.
fn run_one(ctx: &mut Ctx, id: &str, fix: &Value) -> Res<Done> {
    if let Some(act) = fix["act"].as_object() {
        let (app, action) = (act.get("app").and_then(Value::as_str).unwrap_or_default(), act.get("action").and_then(Value::as_str).unwrap_or_default());
        if !ALLOWED.contains(&(app, action)) {
            return Err(format!("{id}'s fix names {app} {action}, which a fix may not run: do it from its own place"));
        }
        let args = act.get("args").and_then(Value::as_array).cloned().unwrap_or_default();
        let done = super::act(app, action, args, None, ctx.env.clone());
        if done["unknown"] == true {
            return Err(format!("{id}'s fix names an action this hub does not know ({app} {action})"));
        }
        if let Some(error) = done["error"].as_str() {
            return Err(error.to_string());
        }
        // An action that waits to be asked says nothing: that is no fix done.
        let note = done["note"].as_str().filter(|n| !n.is_empty()).ok_or_else(|| format!("{id}'s fix did not run: do it from its own place"))?;
        return Ok(Done::Ran(note.to_string()));
    }
    let Some(command) = fix["command"].as_str() else {
        return Err(format!("{} has no fix to run: {}", id, fix["label"].as_str().unwrap_or("see cockpit doctor")));
    };
    if let Some(shown) = sandboxed(std::env::var_os("COCKPIT_HC_BIN").is_some(), command) {
        return Ok(Done::Shown(shown));
    }
    // The report says `cockpit`; the tab runs this binary, the one the hub is.
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "cockpit".into());
    let line = match command.strip_prefix("cockpit ") {
        Some(rest) => format!("{} {rest}", crate::value::quote(&exe)),
        None => command.to_string(),
    };
    let place = sources::in_tab(ctx, &format!("fix · {id}"), &line, command)?;
    Ok(Done::Ran(match place.strip_suffix(", already open") {
        Some(open) => format!("{command} is already open {open}: finish it there"),
        None => format!("running {command} {place}"),
    }))
}

/// Marks the report the person saw `stale`, so the next `--cached` read runs the checks again; a newer one stays.
fn mark_stale(at: u64) {
    let kept = files::cache().join("doctor.json");
    files::locked(&kept, || {
        if let Some(mut now) = files::load(&kept).filter(|r| r["at"].as_u64() == Some(at)) {
            now["stale"] = Value::Bool(true);
            let _ = files::write(&kept, &now);
        }
    });
}

/// `[check id, report at, the fix as shown]`: the fix that report gives that check, refused unless it is still the one
/// the person confirmed.
pub fn fix(ctx: &mut Ctx, args: &[Value]) -> Res {
    let id = text(arg(args, 0)?);
    let at = arg(args, 1)?.as_u64().ok_or("which report: the fix names the report it was shown in")?;
    let seen = text(arg(args, 2)?);
    let report = report(at)?;
    let fix = checked(&report, &id, &seen)?;
    match run_one(ctx, &id, fix)? {
        Done::Shown(note) => ctx.note(note),
        Done::Ran(note) => {
            mark_stale(at);
            ctx.note(note);
        }
    }
    Ok(())
}

/// What a batch of fixes did, in the order they ran: each one's note, and each failure with its check.
#[derive(Debug, Default, PartialEq)]
struct Batch {
    ran: Vec<String>,
    shown: Vec<String>,
    failed: Vec<(String, String)>,
}

impl Batch {
    /// The first line says how many were fixed and failed ("Fixed 3 · 1 failed: why"); one line per fix follows.
    fn words(&self) -> String {
        let mut head = Vec::new();
        if !self.ran.is_empty() {
            head.push(format!("Fixed {}", self.ran.len()));
        }
        if !self.shown.is_empty() {
            head.push(format!("{} shown, not run", self.shown.len()));
        }
        match self.failed.as_slice() {
            [] => {}
            [(_, why)] => head.push(format!("1 failed: {why}")),
            many => head.push(format!("{} failed", many.len())),
        }
        // One failure is said in the first line already; several each get their own, named by their check.
        let failed = self
            .failed
            .iter()
            .filter(|_| self.failed.len() > 1)
            .map(|(id, why)| if why.starts_with(id.as_str()) { why.clone() } else { format!("{id}: {why}") });
        let lines = self.ran.iter().chain(&self.shown).cloned().chain(failed);
        std::iter::once(head.join(" · ")).chain(lines).collect::<Vec<_>>().join("\n")
    }
}

/// Checks every listed fix against `report` before any runs, refusing the whole batch on one that changed; then runs
/// each with `run`, a failure noted without stopping the rest, and calls `ran_some` once when any fix ran.
fn run_batch(report: &Value, listed: &[(String, String)], mut run: impl FnMut(&str, &Value) -> Res<Done>, ran_some: impl FnOnce()) -> Res<Batch> {
    let mut fixes = Vec::new();
    for (id, seen) in listed {
        if fixes.iter().any(|(named, _)| named == id) {
            continue;
        }
        let fix = checked(report, id, seen).map_err(|_| "The checks changed: look again".to_string())?;
        fixes.push((id.clone(), fix));
    }
    if fixes.is_empty() {
        return Err("no fix to run".into());
    }
    let mut batch = Batch::default();
    for (id, fix) in fixes {
        match run(&id, fix) {
            Ok(Done::Ran(note)) => batch.ran.push(note),
            Ok(Done::Shown(note)) => batch.shown.push(note),
            Err(why) => batch.failed.push((id, why)),
        }
    }
    if !batch.ran.is_empty() {
        ran_some();
    }
    Ok(batch)
}

/// `[report at, [[check id, the fix as shown], …]]`: every fix the person confirmed in one go, each checked against
/// the report first and run as `fix` runs it. Its note is a line of counts, then one line per fix.
pub fn fix_all(ctx: &mut Ctx, args: &[Value]) -> Res {
    let at = arg(args, 0)?.as_u64().ok_or("which report: the fixes name the report they were shown in")?;
    let listed: Vec<(String, String)> =
        arg(args, 1)?.as_array().ok_or("which fixes: a list of [check id, fix as shown]")?.iter().map(|pair| (text(&pair[0]), text(&pair[1]))).collect();
    let report = report(at)?;
    let batch = run_batch(&report, &listed, |id, fix| run_one(ctx, id, fix), || mark_stale(at))?;
    // Nothing done at all is an error, so its toast says so.
    if batch.ran.is_empty() && batch.shown.is_empty() {
        return Err(batch.words());
    }
    ctx.note(batch.words());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A report of three checks: a command, an action and one said in words only.
    fn three() -> Value {
        serde_json::json!({"checks": [
            {"id": "tools.gh", "fix": {"command": "gh auth login"}},
            {"id": "sources.inbox", "fix": {"act": {"app": "sources", "action": "refresh", "args": []}}},
            {"id": "claude.plugin", "fix": {"label": "/plugin install cockpit"}},
        ]})
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(id, seen)| (id.to_string(), seen.to_string())).collect()
    }

    #[test]
    fn a_batch_with_one_fix_changed_runs_none() {
        let mut runs = Vec::new();
        let mut marks = 0;
        for listed in [
            pairs(&[("tools.gh", "gh auth login"), ("sources.inbox", "sources refresh [\"github\"]")]),
            pairs(&[("tools.gh", "gh auth login"), ("tools.gone", "")]),
            pairs(&[]),
        ] {
            let refused = run_batch(&three(), &listed, |id, _| (runs.push(id.to_string()), Ok(Done::Ran(id.into()))).1, || marks += 1);
            assert!(refused.is_err(), "{listed:?}");
        }
        assert_eq!(run_batch(&three(), &pairs(&[("tools.gh", "x")]), |_, _| Ok(Done::Ran("x".into())), || ()).unwrap_err(), "The checks changed: look again");
        assert!(runs.is_empty(), "nothing ran: {runs:?}");
        assert_eq!(marks, 0);
    }

    #[test]
    fn a_batch_runs_every_fix_and_marks_the_report_once() {
        let mut runs = Vec::new();
        let mut marks = 0;
        let listed = pairs(&[("tools.gh", "gh auth login"), ("sources.inbox", "sources refresh []"), ("tools.gh", "gh auth login")]);
        let batch = run_batch(&three(), &listed, |id, _| (runs.push(id.to_string()), Ok(Done::Ran(format!("ran {id}")))).1, || marks += 1).unwrap();
        assert_eq!(runs, ["tools.gh", "sources.inbox"], "each check once, in order");
        assert_eq!(marks, 1);
        assert_eq!(batch.words(), "Fixed 2\nran tools.gh\nran sources.inbox");
    }

    #[test]
    fn a_failed_fix_is_noted_and_the_rest_still_run() {
        let mut runs = Vec::new();
        let mut marks = 0;
        let listed = pairs(&[("claude.plugin", "  []"), ("tools.gh", "gh auth login"), ("sources.inbox", "sources refresh []")]);
        let batch = run_batch(
            &three(),
            &listed,
            |id, fix| {
                runs.push(id.to_string());
                if fix.get("label").is_some() { Err(format!("{id} has no fix to run")) } else { Ok(Done::Ran(format!("ran {id}"))) }
            },
            || marks += 1,
        )
        .unwrap();
        assert_eq!(runs, ["claude.plugin", "tools.gh", "sources.inbox"]);
        assert_eq!(marks, 1);
        assert_eq!(batch.words(), "Fixed 2 · 1 failed: claude.plugin has no fix to run\nran tools.gh\nran sources.inbox");
        // Only shown, in a sandbox: nothing ran, so the report stays as it is.
        let shown = run_batch(&three(), &pairs(&[("tools.gh", "gh auth login")]), |_, _| Ok(Done::Shown("shown".into())), || marks += 1).unwrap();
        assert_eq!((marks, shown.words()), (1, "1 shown, not run\nshown".to_string()));
        let failed = Batch { failed: vec![("a".into(), "x".into()), ("b".into(), "b's fix did not run".into())], ..Batch::default() };
        assert_eq!(failed.words(), "2 failed\na: x\nb's fix did not run");
    }

    #[test]
    fn a_sandbox_shows_a_fix_s_command_rather_than_run_it() {
        let shown = sandboxed(true, "terminal-browser config set render.fps 60");
        assert_eq!(shown.as_deref(), Some("In a sandbox a fix's command is shown, not run: terminal-browser config set render.fps 60"));
        assert_eq!(sandboxed(false, "gh auth login"), None);
    }
}
