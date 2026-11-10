//! `cockpit act`: what a notification's button does, through the hub's inbox and routines actions and the inbox cache. Pressed from a
//! notification, nobody reads stderr: a failure comes back as a notification too.

use std::time::Duration;

use cockpit_core::files;
use serde_json::{Value, json};

const USAGE: &str = "What a notification's button does, with the inbox app's own actions and the inbox cache.

    cockpit act review <PR url>       an agent's review of the PR in the actions space
    cockpit act ci <PR url>           an agent's investigation of the PR's failing CI in a new space
    cockpit act rerun <PR url>        the PR's failed CI jobs rerun
    cockpit act plan <ticket id>      an agent's plan for the ticket in the actions space
    cockpit act merge <PR url>        the PR merged (or queued) by its repo's preferred method
    cockpit act routine <slug>        the routine's last agent run resumed in the actions space";

pub fn main(args: &[String]) {
    match press(args) {
        Ok(note) if !note.is_empty() => println!("{note}"),
        Ok(_) => {}
        Err(message) => {
            let cut: String = message.chars().take(200).collect();
            cockpit_core::notify::post("⚠️ Cockpit", &cut, "space:inbox", &[]);
            crate::proc::fail(&message);
        }
    }
}

/// The action's note; an error says why nothing happened.
fn press(args: &[String]) -> Result<String, String> {
    let [kind, reference] = args else { return Err(USAGE.into()) };
    match kind.as_str() {
        "review" => act("inbox", "review", vec![pr_of(reference)?], None),
        "ci" => {
            let pr = pr_of(reference)?;
            let run = ci_run_of(&pr);
            act("inbox", "fix_ci", vec![pr, run, json!("here")], None)
        }
        "rerun" => {
            let pr = pr_of(reference)?;
            let run = ci_run_of(&pr);
            let workflow = run.get("workflowId").filter(|w| !w.is_null() && *w != "").cloned();
            let Some(workflow) = workflow else {
                return Err(format!("no CI run to rerun for {}#{}", text(&pr["repo"]), text(&pr["number"])));
            };
            // A notification's button is the confirmation the inbox asks for with a second press.
            act("inbox", "rerun", vec![pr, run], Some(format!("rerun-{}", text(&workflow))))
        }
        "routine" => act("routines", "talk", vec![json!(reference), Value::Null], None),
        "merge" => act("inbox", "merge", vec![pr_of(reference)?], None),
        "plan" => act("inbox", "plan", vec![json!({"id": reference.to_uppercase(), "url": ticket_url(reference).unwrap_or_default()})], None),
        _ => Err(USAGE.into()),
    }
}

const HUB_USAGE: &str = "cockpit hub act review send|walkthrough|stop_walkthrough <JSON list of arguments>";

/// `cockpit hub act review <action> <args>`: a remote desk's review action, run on the runner through Herdr's machine
/// connection and handed to the hub on its unix socket, `{note}` printed as JSON.
pub fn hub_review(args: &[String]) {
    match runner_review(args) {
        Ok(note) => println!("{}", json!({ "note": note })),
        Err(message) => crate::proc::fail(&message),
    }
}

/// Only the review panel's own actions, with the arguments it sends: everything else stays out of reach.
fn runner_review(args: &[String]) -> Result<String, String> {
    let [app, action, list] = args else { return Err(HUB_USAGE.into()) };
    let parsed: Vec<Value> = serde_json::from_str(list).map_err(|_| format!("review {action} takes a JSON list of arguments, not {list}"))?;
    let repo = |v: &Value| v.as_str().is_some_and(|p| p.starts_with('/'));
    let scope = |v: &Value| matches!(v.as_str(), Some("uncommitted" | "branch" | "pr"));
    let fits = match (app.as_str(), action.as_str(), parsed.as_slice()) {
        // An app terminal's own address (t7): never a handle or another machine's terminal.
        ("review", "send", [terminal, review]) => {
            terminal.as_str().is_some_and(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric())) && review["comments"].is_array()
        }
        ("review", "walkthrough", [path, s, model]) => repo(path) && scope(s) && model.is_string(),
        ("review", "stop_walkthrough", [path, s]) => repo(path) && scope(s),
        ("review", "send" | "walkthrough" | "stop_walkthrough", _) => false,
        _ => return Err(HUB_USAGE.into()),
    };
    if !fits {
        return Err(format!("review {action} does not take {list}"));
    }
    act(app, action, parsed, None)
}

/// The hub's action of that app, its note as the answer.
fn act(app: &str, action: &str, args: Vec<Value>, armed: Option<String>) -> Result<String, String> {
    let env = json!({"HERDR_PANE_ID": std::env::var("HERDR_PANE_ID").unwrap_or_default()});
    let request = json!({"op": "act", "app": app, "action": action, "args": args, "armed": armed, "env": env});
    let answer = cockpit_core::hub::request(&request, Duration::from_secs(120))?;
    if answer["unknown"] == true {
        return Err(format!("the cockpit hub has no {app} action {action}"));
    }
    if let Some(error) = answer["error"].as_str() {
        return Err(error.to_string());
    }
    Ok(answer["note"].as_str().unwrap_or_default().to_string())
}

fn text(value: &Value) -> String {
    value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string())
}

fn pr_of(url: &str) -> Result<Value, String> {
    let data = files::load(&files::cache().join("inbox.json")).unwrap_or(Value::Null);
    ["mine", "direct", "team"]
        .iter()
        .flat_map(|field| data[field].as_array().cloned().unwrap_or_default())
        .find(|pr| pr["url"] == url)
        .ok_or_else(|| format!("{url} is not in the inbox cache"))
}

/// The CI run of the PR's branch, null when the CI cache has none.
fn ci_run_of(pr: &Value) -> Value {
    let runs = files::load(&files::cache().join("ci.json")).unwrap_or(Value::Null);
    runs["runs"].as_array().into_iter().flatten().find(|r| r["repo"] == pr["repo"] && r["branch"] == pr["branch"]).cloned().unwrap_or(Value::Null)
}

/// A ticket's URL: the configured template's, else an assigned ticket's URL cut where its id is a path segment, with the
/// id swapped.
fn ticket_url(id: &str) -> Option<String> {
    let upper = id.to_uppercase();
    let config = files::config();
    if let Some(template) = config["tickets"]["urlTemplate"].as_str().filter(|t| !t.is_empty()) {
        return Some(fill(template, &upper));
    }
    let inbox = files::load(&files::cache().join("inbox.json")).unwrap_or(Value::Null);
    inbox["tickets"].as_array().into_iter().flatten().find_map(|ticket| {
        let (url, known) = (ticket["url"].as_str().unwrap_or_default(), ticket["id"].as_str().unwrap_or_default());
        let at = url.rfind(&format!("/{known}")).filter(|_| !known.is_empty())?;
        Some(format!("{}{upper}", &url[..at + 1]))
    })
}

/// The template with `{id}` filled in, URL-quoted, and any other `{name}` left empty.
fn fill(template: &str, id: &str) -> String {
    let quoted: String =
        id.bytes().map(|b| if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let name_len = rest[open + 1..].find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len() - open - 1);
        let after = open + 1 + name_len;
        if name_len > 0 && rest[after..].starts_with('}') {
            out.push_str(&rest[..open]);
            if &rest[open + 1..after] == "id" {
                out.push_str(&quoted);
            }
            rest = &rest[after + 1..];
        } else {
            out.push_str(&rest[..=open]);
            rest = &rest[open + 1..];
        }
    }
    out + rest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_gets_the_id_quoted() {
        assert_eq!(fill("https://linear.app/acme/issue/{id}", "ENG-4"), "https://linear.app/acme/issue/ENG-4");
        assert_eq!(fill("https://x/{id}?t={team}&{", "A B/1"), "https://x/A%20B%2F1?t=&{");
        assert_eq!(fill("{x-y}{id}", "E"), "{x-y}E");
    }
}
