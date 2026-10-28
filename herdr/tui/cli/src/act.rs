//! `cockpit act`: what a notification's button does, through the hub's inbox actions and the inbox cache. Pressed from a
//! notification, nobody reads stderr: a failure comes back as a notification too.

use std::time::Duration;

use cockpit_core::files;
use serde_json::{Value, json};

const USAGE: &str = "What a notification's button does, with the inbox app's own actions and the inbox cache.

    cockpit act review <PR url>       a Claude review of the PR in the actions space
    cockpit act ci <PR url>           a Claude investigation of the PR's failing CI in a new space
    cockpit act rerun <PR url>        the PR's failed CI jobs rerun, by the next Claude session
    cockpit act plan <ticket id>      a Claude plan for the ticket in the actions space";

pub fn main(args: &[String]) {
    match press(args) {
        Ok(note) if !note.is_empty() => println!("{note}"),
        Ok(_) => {}
        Err(message) => {
            let cut: String = message.chars().take(200).collect();
            let _ = std::process::Command::new("sh").arg(script("notify.sh")).args(["⚠️ Cockpit", &cut, "space:inbox"]).output();
            crate::proc::fail(&message);
        }
    }
}

/// One of the plugin's scripts, through the link that follows herdr's updates, else in the herdr folder.
fn script(name: &str) -> std::path::PathBuf {
    let linked = files::config_dir().join("herdr").join(name);
    if linked.exists() { linked } else { files::herdr_dir().join(name) }
}

/// The action's note; an error says why nothing happened.
fn press(args: &[String]) -> Result<String, String> {
    let [kind, reference] = args else { return Err(USAGE.into()) };
    match kind.as_str() {
        "review" => act("review", vec![pr_of(reference)?], None),
        "ci" => {
            let pr = pr_of(reference)?;
            let run = ci_run_of(&pr);
            act("fix_ci", vec![pr, run, json!("here")], None)
        }
        "rerun" => {
            let pr = pr_of(reference)?;
            let run = ci_run_of(&pr);
            let workflow = run.get("workflowId").filter(|w| !w.is_null() && *w != "").cloned();
            let Some(workflow) = workflow else {
                return Err(format!("no CI run to rerun for {}#{}", text(&pr["repo"]), text(&pr["number"])));
            };
            // A notification's button is the confirmation the inbox asks for with a second press.
            act("rerun", vec![pr, run], Some(format!("rerun-{}", text(&workflow))))
        }
        "plan" => act("plan", vec![json!({"id": reference.to_uppercase(), "url": ticket_url(reference).unwrap_or_default()})], None),
        _ => Err(USAGE.into()),
    }
}

/// The hub's inbox action, its note as the answer.
fn act(action: &str, args: Vec<Value>, armed: Option<String>) -> Result<String, String> {
    let env = json!({"HERDR_PANE_ID": std::env::var("HERDR_PANE_ID").unwrap_or_default()});
    let request = json!({"op": "act", "app": "inbox", "action": action, "args": args, "armed": armed, "env": env});
    let answer = cockpit_core::hub::request(&request, Duration::from_secs(120))?;
    if answer["unknown"] == true {
        return Err(format!("the cockpit hub has no inbox action {action}"));
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
    runs["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["repo"] == pr["repo"] && r["branch"] == pr["branch"])
        .cloned()
        .unwrap_or(Value::Null)
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
    let quoted: String = id.bytes().map(|b| if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
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
