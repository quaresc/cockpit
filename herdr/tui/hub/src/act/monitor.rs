//! The monitor's actions: Claude sessions in the actions space that investigate how a release, an error or a monitor
//! does, fix an error, draft a bug or roll a release back, each in the app's repo; plus pins, refreshes and links.
//! Each takes the app as the monitor document has it, with its `env`.

use std::path::PathBuf;

use cockpit_core::files;
use serde_json::{Value, json};

use super::{Ctx, arg, inbox};
use crate::config;
use crate::value::{Res, get, list, s, text, truthy};

fn pinned_path() -> PathBuf {
    files::config_dir().join("monitor-pinned.json")
}

pub fn open_url(ctx: &mut Ctx, args: &[Value]) -> Res {
    inbox::open_url(ctx, args)
}

pub fn request_refresh(ctx: &mut Ctx, args: &[Value]) -> Res {
    inbox::request_refresh(ctx, args)
}

/// The item of the app's `items` list whose `field` is this id.
fn find<'a>(app: &'a Value, items: &str, field: &str, id: &Value) -> Option<&'a Value> {
    list(get(app, items)).iter().find(|item| text(get(item, field)) == text(id))
}

/// Who and where: "api (Datadog service api, env production)".
fn who(app: &Value) -> Res<String> {
    Ok(format!("{} (Datadog service {}, env {})", s(app, "name")?, s(app, "service")?, s(app, "env")?))
}

fn window(w: &Value) -> String {
    format!("{} errors in {} requests, p95 {} s", text(get(w, "errors")), text(get(w, "hits")), text(get(w, "p95")))
}

/// A release on one line: tag, who, when, the verdict and its numbers, its tickets and PRs.
fn release_fact(release: &Value) -> Res<String> {
    let verdict = get(release, "verdict");
    let mut fact = format!(
        "release {} by {} ({}), {} → {}, verdict {}: {}",
        s(release, "tag")?,
        text(get(release, "actor")),
        text(get(release, "phase")),
        text(get(release, "startedAt")),
        text(get(release, "finishedAt")),
        text(get(verdict, "state")),
        text(get(verdict, "summary"))
    );
    if truthy(get(verdict, "before")) && truthy(get(verdict, "after")) {
        fact += &format!(" (before: {}; after: {})", window(get(verdict, "before")), window(get(verdict, "after")));
    }
    let refs: Vec<String> = list(get(release, "tickets")).iter().chain(list(get(release, "prs"))).map(|r| text(get(r, "url"))).collect();
    if !refs.is_empty() {
        fact += &format!(", shipping {}", refs.join(", "));
    }
    Ok(fact)
}

fn issue_fact(issue: &Value) -> Res<String> {
    let new = if truthy(get(issue, "isNew")) { "new " } else { "" };
    Ok(format!(
        "{new}error {}: {}, {} events in the last day, first seen {} in {} ({})",
        s(issue, "type")?,
        text(get(issue, "message")),
        text(get(issue, "count")),
        text(get(issue, "firstSeen")),
        text(get(issue, "firstVersion")),
        text(get(issue, "url"))
    ))
}

fn monitor_fact(monitor: &Value) -> Res<String> {
    Ok(format!("monitor \"{}\" is {} ({})", s(monitor, "name")?, text(get(monitor, "status")), text(get(monitor, "url"))))
}

/// What a session is about: the release, issue or monitor the target names, else the app as a whole.
struct Subject {
    /// "the release v2", "the error Req.TransportError: socket closed".
    phrase: String,
    /// The tag or id that names it in a tab's label.
    name: String,
    facts: Vec<String>,
    url: Option<String>,
}

fn subject(app: &Value, target: &Value) -> Res<Subject> {
    if let Some(id) = target.get("release") {
        let release = find(app, "releases", "id", id).ok_or_else(|| format!("no release {} of {}", text(id), text(get(app, "name"))))?;
        let tag = s(release, "tag")?;
        let mut facts = vec![release_fact(release)?];
        for new in list(get(get(release, "verdict"), "newIssues")) {
            if let Some(issue) = find(app, "issues", "id", new) {
                facts.push(issue_fact(issue)?);
            }
        }
        return Ok(Subject { phrase: format!("the release {tag}"), name: tag, facts, url: None });
    }
    if let Some(id) = target.get("issue") {
        let issue = find(app, "issues", "id", id).ok_or_else(|| format!("no issue {} in {}", text(id), text(get(app, "name"))))?;
        let phrase = format!("the error {}: {}", s(issue, "type")?, text(get(issue, "message")));
        return Ok(Subject { phrase, name: text(id), facts: vec![issue_fact(issue)?], url: Some(text(get(issue, "url"))) });
    }
    if let Some(id) = target.get("monitor") {
        let monitor = find(get(app, "monitors"), "alerting", "id", id).ok_or_else(|| format!("no alerting monitor {}", text(id)))?;
        let phrase = format!("the monitor \"{}\" ({})", s(monitor, "name")?, text(get(monitor, "status")));
        return Ok(Subject { phrase, name: format!("monitor {}", text(id)), facts: vec![monitor_fact(monitor)?], url: Some(text(get(monitor, "url"))) });
    }
    Ok(Subject { phrase: String::new(), name: String::new(), facts: Vec::new(), url: None })
}

/// The app around the subject: its deploy, monitors, new errors and releases, the subject's own left out.
fn context(app: &Value, target: &Value) -> Res<Vec<String>> {
    let mut facts = vec![format!("ArgoCD says {}, running {} since {}", text(get(app, "health")), text(get(app, "tag")), text(get(app, "deployedAt")))];
    let monitors = get(app, "monitors");
    if truthy(monitors) {
        let counts = ["ok", "warn", "alert", "nodata"].map(|k| format!("{} {k}", text(get(monitors, k))));
        facts.push(format!("monitors: {}", counts.join(", ")));
    }
    let skip = |kind: &str, item: &Value, field: &str| target.get(kind).is_some_and(|id| text(id) == text(get(item, field)));
    for monitor in list(get(monitors, "alerting")).iter().filter(|m| !skip("monitor", m, "id")) {
        facts.push(monitor_fact(monitor)?);
    }
    for issue in list(get(app, "issues")).iter().filter(|i| truthy(get(i, "isNew")) && !skip("issue", i, "id")) {
        if !target
            .get("release")
            .is_some_and(|r| find(app, "releases", "id", r).is_some_and(|r| list(get(get(r, "verdict"), "newIssues")).contains(get(issue, "id"))))
        {
            facts.push(issue_fact(issue)?);
        }
    }
    for release in list(get(app, "releases")).iter().filter(|r| !skip("release", r, "id")) {
        facts.push(release_fact(release)?);
    }
    Ok(facts)
}

fn links(app: &Value, subject: &Subject) -> String {
    let mut links: Vec<String> = subject.url.iter().cloned().collect();
    links.extend(["datadogUrl", "argoUrl"].iter().map(|k| text(get(app, k))).filter(|l| !l.is_empty() && l != "None"));
    links.join(", ")
}

/// A read-only investigation of the app, or of the release, issue or monitor the target names, ending on a verdict.
pub fn investigate(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (app, target) = (arg(args, 0)?, arg(args, 1)?);
    let subject = subject(app, target)?;
    let mut facts = subject.facts.clone();
    facts.extend(context(app, target)?);
    let focus = if subject.phrase.is_empty() { String::new() } else { format!(", looking at {}", subject.phrase) };
    let prompt = format!(
        "Investigate how {} does in production{focus}. Load the datadog-conventions skill, then follow the datadog-incident-investigation skill. This is read-only: change nothing in Datadog, the code, Linear or Slack. What cockpit's monitor saw: {}. Links: {}. End with a short verdict: whether something is wrong, the likely cause, and the next step (nothing, a fix, a bug or a roll back).",
        who(app)?,
        facts.join("; "),
        links(app, &subject)
    );
    let name = s(app, "name")?;
    let label = if subject.name.is_empty() { format!("investigate {name}") } else { format!("investigate {name} {}", subject.name) };
    let dir = inbox::repo_dir(ctx, &s(app, "repo")?);
    inbox::start_action(ctx, &dir, &label, &prompt, &[]);
    Ok(())
}

/// The error reproduced and fixed in the app's repo, up to a draft PR.
pub fn fix(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (app, id) = (arg(args, 0)?, arg(args, 1)?);
    let subject = subject(app, &json!({"issue": id}))?;
    let shipped = list(get(app, "releases"))
        .iter()
        .find(|r| list(get(get(r, "verdict"), "newIssues")).iter().any(|n| text(n) == text(id)))
        .map(|r| format!(" It came with the release {}.", text(get(r, "tag"))))
        .unwrap_or_default();
    let prompt = format!(
        "Fix {} in {}: {}.{shipped} Load the datadog-conventions skill and read the issue's stack trace and sample events, find the cause in this repo, reproduce it in a failing test, then fix it. Follow the repo's conventions (its CLAUDE.md and mine): branch off an updated main, name the branch as they say and ask me for the ticket key when it needs one, then open a draft PR with a short description that links the Datadog issue.",
        subject.phrase,
        who(app)?,
        subject.facts.join("; ")
    );
    let dir = inbox::repo_dir(ctx, &s(app, "repo")?);
    inbox::start_action(ctx, &dir, &format!("fix {} {}", s(app, "name")?, subject.name), &prompt, &[]);
    Ok(())
}

/// A Linear bug drafted from the evidence, shown before it is filed.
pub fn bug(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (app, target) = (arg(args, 0)?, arg(args, 1)?);
    let subject = subject(app, target)?;
    let mut facts = subject.facts.clone();
    facts.extend(context(app, target)?);
    let prompt = format!(
        "Draft a Linear bug for {} in {}. The evidence cockpit's monitor has: {}. Links: {}. Add what the bug needs from Datadog (load the datadog-conventions skill: the stack trace, sample events, when it started, how many requests or users it hits), then write it following my conventions for Linear issues in CLAUDE.md. Show me the draft and ask before filing it: file nothing until I say so.",
        subject.phrase,
        who(app)?,
        facts.join("; "),
        links(app, &subject)
    );
    let dir = inbox::repo_dir(ctx, &s(app, "repo")?);
    inbox::start_action(ctx, &dir, &format!("bug {} {}", s(app, "name")?, subject.name), &prompt, &[]);
    Ok(())
}

/// Once confirmed, a session that checks the previous tag and proposes its Backstage deploy, asking before it.
pub fn roll_back(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (app, release, previous) = (arg(args, 0)?, arg(args, 1)?, text(arg(args, 2)?));
    let subject = subject(app, &json!({"release": release}))?;
    let (id, name, env, tag) = (s(app, "id")?, s(app, "name")?, s(app, "env")?, subject.name.clone());
    if !ctx.confirmed(&format!("rollback-{id}-{tag}")) {
        return Ok(());
    }
    let prompt = format!(
        "Roll {name} back in {env} from {tag} to {previous}. Why: {}. First check {previous}: what changed between it and {tag} (git log {previous}..{tag} in this repo, and the PRs in between), whether anything {tag} shipped makes going back unsafe (a migration, a message or API contract other apps rely on), and what Backstage says runs now (get-deploy-status, list-releases). Then propose the Backstage deploy of {previous} to {env}, naming the environment, the app and the version, and ask me before calling deploy-app. Deploy nothing until I say yes; once deployed, follow it with get-deploy-workflow until it ends.",
        subject.facts.join("; ")
    );
    let dir = inbox::repo_dir(ctx, &s(app, "repo")?);
    inbox::start_action(ctx, &dir, &format!("roll back {name} {tag}"), &prompt, &[]);
    Ok(())
}

/// The talk about the ticket a release of Clement's shipped, its session forked, told how the release does.
pub fn talk(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (app, id) = (arg(args, 0)?, arg(args, 1)?);
    let release = find(app, "releases", "id", id).ok_or_else(|| format!("no release {}", text(id)))?;
    let Some(shipped) = list(get(release, "tickets")).first() else {
        ctx.note(format!("{} ships no ticket to talk about", s(release, "tag")?));
        return Ok(());
    };
    let data = config::read_inbox();
    let ticket_id = s(shipped, "id")?;
    let ticket = list(get(&data, "tickets")).iter().find(|t| text(get(t, "id")) == ticket_id).cloned().unwrap_or_else(
        || json!({"id": ticket_id, "url": get(shipped, "url"), "title": format!("shipped in {} {}", text(get(app, "name")), text(get(release, "tag")))}),
    );
    let urls: Vec<String> = list(get(release, "prs")).iter().map(|p| text(get(p, "url"))).collect();
    let prs: Vec<Value> = list(get(&data, "mine")).iter().filter(|p| urls.contains(&text(get(p, "url")))).cloned().collect();
    let verdict = get(release, "verdict");
    let news = format!(
        " Its release {} of {} reached {} at {}, and the monitor's verdict is {}: {}.",
        s(release, "tag")?,
        s(app, "name")?,
        s(app, "env")?,
        text(get(release, "finishedAt")),
        text(get(verdict, "state")),
        text(get(verdict, "summary"))
    );
    inbox::talk_over(ctx, &ticket, &prs, &data, None, &news)
}

/// Pins or unpins an app in monitor-pinned.json: a pinned app is watched even without a recent release.
pub fn pin(ctx: &mut Ctx, args: &[Value]) -> Res {
    let (app, is_pinned) = (text(arg(args, 0)?), truthy(arg(args, 1)?));
    let mut pinned: Vec<String> = files::load(&pinned_path()).map(|m| list(&m).iter().map(text).collect()).unwrap_or_default();
    pinned.retain(|a| a != &app);
    if is_pinned {
        pinned.push(app.clone());
    }
    pinned.sort();
    files::write(&pinned_path(), &json!(pinned)).map_err(|e| e.to_string())?;
    inbox::request_refresh(ctx, &[])?;
    ctx.note(format!("{} {app}", if is_pinned { "pinned" } else { "unpinned" }));
    Ok(())
}
