//! `cockpit connector map`: the default agent, run headless, writes the mapping of a server cockpit has no preset for.
//! cockpit lists the tools, tries the agent's mapping on the real server and shows it what came back, up to three times.

use std::time::Duration;

use cockpit_connectors::{Connection, Server};
use cockpit_core::agent::{self, Headless};
use serde_json::{Value, json};

use crate::proc;

const ROUNDS: usize = 3;

/// What a section's mapping holds, as the agent is told it.
pub fn brief(section: &str) -> Option<&'static str> {
    Some(match section {
        "tickets" => {
            "Map these capabilities, each `{\"tool\", \"args\"}` with the placeholders {ticket} (its key or id), {team} (the key's prefix) and {state} \
             (an id listStates gives): listMine (my open tickets, e.g. a search for assignee = me and not done), get (one ticket with description and \
             attachments), comments (a ticket's comments), listStates (the states a ticket can move to), setState (moves a ticket to one). \
             Fill `fields` with dotted paths into the answers (`a.b`, `a.0.b`, `a|b` for the first that has one; \"\" is the answer itself): tickets (the \
             list in listMine's answer), id, title, url, status (the state's name), statusType (its category), parentId, project, description, \
             attachments, attachmentUrl, and in listStates' answer states, stateId, stateName, stateType. cockpit reads Linear's categories (triage, \
             backlog, unstarted, started, completed, canceled): map the tool's own to them in `statusTypes`, e.g. Jira's {\"new\": \"unstarted\", \
             \"indeterminate\": \"started\", \"done\": \"completed\"}. Answer {\"tools\", \"fields\", \"statusTypes\"}."
        }
        "ci" => {
            "Map these capabilities, each `{\"tool\", \"args\"}` with the placeholders {run}, {workflow} (a run's id when the CI has no workflows) and \
             {job}: listRuns (my recent pipelines, newest first), listWorkflows (a run's workflows, only for a CI that nests them), listJobs (a \
             workflow's or run's jobs), getJob (one job with its steps), listTests (a job's failed tests), logs (a job's logs), rerun (reruns the \
             failed jobs). Fill `fields` with dotted paths (`a.b`, `a.0.b`, `a|b`; a `[]` segment flattens nested lists): runs, runId, runRepo (a repo \
             name or URL), runBranch, runTag (set on tag builds), runCreated, runPhase, runOutcome; workflows, workflowId, workflowName, \
             workflowCreated, workflowOutcome; jobs, jobId, jobName, jobPhase, jobOutcome; steps, stepName, stepOutcome, stepExit; tests, testName. \
             Map the CI's outcomes to succeeded, failed and canceled in `outcomes`, and list in `endedPhases` the phase values that mean ended. \
             Answer {\"tools\", \"fields\", \"outcomes\", \"endedPhases\"}."
        }
        "calendar" => {
            "Map listEvents (my coming events, with {pageToken} for the next page when it pages) and getEvent (one event, {event}), each \
             `{\"tool\", \"args\"}`. Fill `fields` with dotted paths (`a.b`, `a.0.b`, `a|b`): events (the list), nextPage, id, title, start and end \
             (date-times; all-day events must have none at that path), url, join (the video call), location, description, series, myResponse, \
             attendees, attendeeSelf, attendeeResponse, attendeeResource, attendeeName, attachments, attachmentTitle, attachmentUrl. List in `skip` \
             the values that leave an event out by field path (Outlook: {\"isCancelled\": [\"true\"], \"showAs\": [\"free\"]}), in `only` the values \
             an event must hold (\"\" is a missing field), and map invite answers to accepted, tentative, declined, needsAction in `responses`. \
             Answer {\"tools\", \"fields\", \"skip\", \"only\", \"responses\"}."
        }
        "platform" => {
            "Map these capabilities, each `{\"tool\", \"args\"}` with the placeholders {appId}, {login} (my GitHub login), {preview}, {app}, {branch} \
             and {days}: deployStatus (the deployed apps and their versions), deployHistory (an app's recent deploys), listPreviews, previewStatus \
             (one preview's app URLs), createPreview, redeployPreview, stopPreview, startPreview, extendPreview (with `keep`, the settings to send \
             back as they were when an update replaces the whole setup). Fill `fields` with dotted paths (`a.b`, `a.0.b`, `a|b`): apps, appId, \
             appRepo, appLink, appTag, appSha, appKind, deployments, deployWorkflow, deployNamespace, deployTag, deployBranch, previews, previewId, \
             previewExpires, previewActive, previewHealth, previewApps, appName, appEnabled, appBranch, statusApps, statusUrl, createdId, healthy, \
             appHealth, appDeployedAt, appBehind, appService, appDatadog, deployId, deployActor, deployPhase, deployStarted, deployFinished, and \
             `skipKinds` with app kinds that never build from a repo. Answer {\"tools\", \"fields\"}."
        }
        _ => return None,
    })
}

/// The read the mapping is tried with, and what it gives: a short sample, or the error and the tool's raw answer.
async fn trial(connection: &Connection, section: &str, mapping: &Value) -> Result<String, String> {
    use cockpit_connectors::{calendar, ci, platform, tickets, tools::Mapped};
    let sample = |items: Vec<Value>| format!("{} item(s), the first: {}", items.len(), items.first().map(Value::to_string).unwrap_or_default());
    match section {
        "tickets" => {
            let mapping = tickets::mapping(mapping);
            tickets::mine(&Mapped { connection, section: &mapping }, &mapping).await.map(sample)
        }
        "ci" => {
            let mapping = ci::mapping(mapping);
            ci::runs(&Mapped { connection, section: &mapping }, &mapping, &Value::Null).await.map(sample)
        }
        "calendar" => {
            let mapping = calendar::mapping(mapping);
            calendar::meetings(&Mapped { connection, section: &mapping }, &mapping).await.map(sample)
        }
        "platform" => {
            let mapping = platform::mapping(mapping);
            let tools = Mapped { connection, section: &mapping };
            platform::deployed_apps(&tools, &mapping).await.map(|apps| sample(apps.iter().map(|a| serde_json::to_value(a).unwrap_or_default()).collect()))
        }
        _ => Err(format!("no trial for {section}")),
    }
}

/// The first read verb's raw answer, cut short, for the agent to see where the values sit.
async fn raw_answer(connection: &Connection, section: &str, mapping: &Value) -> String {
    let verb = match section {
        "tickets" => "listMine",
        "ci" => "listRuns",
        "calendar" => "listEvents",
        _ => "deployStatus",
    };
    match cockpit_connectors::call_mapped(connection, mapping, verb, &json!({})).await {
        Ok(answer) => answer.to_string().chars().take(6000).collect(),
        Err(e) => format!("error: {e}"),
    }
}

/// The JSON between <mapping> and </mapping> in the agent's answer.
fn extract(answer: &str) -> Option<Value> {
    let start = answer.rfind("<mapping>")? + "<mapping>".len();
    let end = start + answer[start..].find("</mapping>")?;
    serde_json::from_str(answer[start..end].trim().trim_start_matches("```json").trim_matches('`').trim()).ok()
}

fn ask_agent(agent: &agent::Agent, prompt: &str) -> Result<String, String> {
    let argv = agent.headless(prompt, &Headless::default()).ok_or_else(|| format!("{} has no headless mode to write the mapping", agent.name))?;
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let out = proc::run(&refs, &[], Duration::from_secs(300)).ok_or_else(|| format!("{} did not answer in 5 minutes", agent.name))?;
    if !out.ok() {
        return Err(format!("{}: {}", agent.name, out.stderr.lines().last().unwrap_or("failed")));
    }
    Ok(out.stdout)
}

/// Writes a mapping for `section` from the server's tools, tried on the server; Ok holds the mapping and the trial's
/// sample.
pub fn map(section: &str, server: &Server, name: &str, agent: &agent::Agent) -> Result<(Value, String), String> {
    let brief = brief(section).ok_or_else(|| format!("cockpit maps tickets, ci, calendar and platform, not {section}"))?;
    cockpit_connectors::block_on(async {
        let connection = cockpit_connectors::connect(server).await?;
        let tools = connection.tool_list().await?;
        let mut prompt = format!(
            "You map an MCP server's tools for cockpit, a dashboard that calls them itself. The server is {name}; its tools:\n{tools}\n\n{brief}\n\
             Use only tools listed above, read-only ones for reads. Write nothing else than the JSON between <mapping> and </mapping>."
        );
        let mut last = String::new();
        for round in 0..ROUNDS {
            let answer = ask_agent(agent, &prompt)?;
            let Some(mapping) = extract(&answer) else {
                last = "the answer held no <mapping> JSON".into();
                prompt = format!("{prompt}\n\nYour last answer held no JSON between <mapping> and </mapping>. Answer again.");
                continue;
            };
            let mut merged = mapping.clone();
            merged["name"] = name.into();
            match trial(&connection, section, &merged).await {
                Ok(sample) if !sample.starts_with("0 item") || round == ROUNDS - 1 => {
                    connection.close().await;
                    return Ok((mapping, sample));
                }
                outcome => {
                    last = outcome.unwrap_or_else(|e| e);
                    let raw = raw_answer(&connection, section, &merged).await;
                    prompt = format!(
                        "{prompt}\n\nYou answered:\n{mapping}\ncockpit tried it and got: {last}\nThe first read's raw answer: {raw}\nFix the mapping and answer again."
                    );
                }
            }
        }
        connection.close().await;
        Err(format!("no working mapping after {ROUNDS} tries: {last}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mapping_is_the_last_json_between_markers() {
        assert_eq!(extract("x <mapping>{\"a\":1}</mapping> then <mapping>```json\n{\"b\":2}\n```</mapping>"), Some(json!({"b": 2})));
        assert_eq!(extract("no markers"), None);
        assert!(brief("tickets").is_some() && brief("other").is_none());
    }
}
