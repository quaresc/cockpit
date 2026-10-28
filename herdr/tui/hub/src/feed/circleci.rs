//! CircleCI through its REST API, given `ci.token`: the latest pipeline of each branch I pushed, its newest workflows'
//! jobs, the failing job with its step and tests, and the image job, as the mod's mapping of the CircleCI connector
//! fills ci.json. An ended pipeline whose workflows did not change is not read again.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::github_ci::is_image_job;
use super::http::{Failure, Fetched};
use crate::value::list;

const RUNS: usize = 30;
/// A pipeline with no workflow yet counts as running this long, then as one with nothing to run.
const STARTING_SECS: i64 = 600;

/// Reads a path of the API ("/api/v2/…") as JSON.
pub type Get<'a> = dyn FnMut(&str) -> Fetched + 'a;

#[derive(Default)]
pub struct CircleCi {
    org: Option<String>,
    /// Ended runs by pipeline id, with their workflows' ids and states when read.
    ended: HashMap<String, (String, Value)>,
}

/// How the failing job's logs are read: through the CircleCI connector in a Claude session.
pub fn logs_hint(name: &str, tools: &[String], job: &str) -> String {
    let listed = if tools.is_empty() { String::new() } else { format!(" ({}; keep log tails small)", tools.join(", ")) };
    format!("the {name} MCP{listed}, job id {job}")
}

fn workflow_outcome(status: &str) -> Option<&'static str> {
    match status {
        "success" | "not_run" => Some("succeeded"),
        "failed" | "error" | "unauthorized" => Some("failed"),
        "canceled" => Some("canceled"),
        _ => None,
    }
}

fn job_outcome(status: &str) -> String {
    match status {
        "success" => "succeeded".into(),
        "failed" | "infrastructure_fail" | "timedout" | "terminated-unknown" | "unauthorized" => "failed".into(),
        other => other.into(),
    }
}

fn is_job_ended(status: &str) -> bool {
    !matches!(status, "running" | "queued" | "not_running" | "on_hold" | "blocked")
}

fn s<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

/// Every item of a paged list, at most `pages` pages.
fn items(get: &mut Get, path: &str, pages: usize) -> Fetched<Vec<Value>> {
    let mut out = Vec::new();
    let mut token = String::new();
    for _ in 0..pages {
        let separator = if path.contains('?') { '&' } else { '?' };
        let page = if token.is_empty() { get(path)? } else { get(&format!("{path}{separator}page-token={token}"))? };
        out.extend(list(&page["items"]).iter().cloned());
        token = page["next_page_token"].as_str().unwrap_or_default().to_string();
        if token.is_empty() {
            break;
        }
    }
    Ok(out)
}

impl CircleCi {
    /// The org whose pipelines are mine: the one named like the GitHub org, else the first I belong to.
    fn org(&mut self, get: &mut Get, github_org: &str) -> Fetched<String> {
        if let Some(org) = &self.org {
            return Ok(org.clone());
        }
        let orgs = get("/api/v2/me/collaborations")?;
        let orgs = list(&orgs);
        let org = orgs.iter().find(|o| s(o, "name").eq_ignore_ascii_case(github_org)).or(orgs.first()).map(|o| s(o, "slug").to_string());
        let org = org.filter(|o| !o.is_empty()).ok_or_else(|| Failure::Other("no CircleCI organization for this token".into()))?;
        self.org = Some(org.clone());
        Ok(org)
    }

    pub fn runs(&mut self, get: &mut Get, github_org: &str, image_job: &str, logs: &dyn Fn(&str) -> String) -> Fetched<Vec<Value>> {
        let org = self.org(get, github_org)?;
        let pipelines = items(get, &format!("/api/v2/pipeline?org-slug={org}&mine=true"), 2)?;
        let mut latest: Vec<(String, Value)> = Vec::new();
        for pipeline in pipelines.into_iter().take(RUNS) {
            let vcs = &pipeline["vcs"];
            let repo = s(vcs, "origin_repository_url").trim_end_matches(".git").rsplit('/').next().unwrap_or_default().to_string();
            let branch = s(vcs, "branch");
            if repo.is_empty() || branch.is_empty() || !s(vcs, "tag").is_empty() {
                continue;
            }
            let key = format!("{repo}/{branch}");
            match latest.iter_mut().find(|(k, _)| *k == key) {
                Some((_, seen)) if s(seen, "created_at") < s(&pipeline, "created_at") => *seen = pipeline,
                Some(_) => {}
                None => latest.push((key, pipeline)),
            }
        }
        let mut runs = Vec::new();
        for (key, pipeline) in latest {
            let (repo, branch) = key.split_once('/').unwrap_or_default();
            runs.push(self.run(get, &pipeline, repo, branch, image_job, logs)?);
        }
        self.ended.retain(|id, _| runs.iter().any(|r| r["runId"] == id.as_str()));
        Ok(runs)
    }

    fn run(&mut self, get: &mut Get, pipeline: &Value, repo: &str, branch: &str, image_job: &str, logs: &dyn Fn(&str) -> String) -> Fetched {
        let id = s(pipeline, "id");
        // A rerun adds a workflow of the same name to the pipeline: only the newest of each name counts.
        let mut workflows: Vec<Value> = Vec::new();
        for workflow in list(&get(&format!("/api/v2/pipeline/{id}/workflow"))?["items"]) {
            match workflows.iter_mut().find(|w| w["name"] == workflow["name"]) {
                Some(seen) if s(seen, "created_at") < s(workflow, "created_at") => *seen = workflow.clone(),
                Some(_) => {}
                None => workflows.push(workflow.clone()),
            }
        }
        let signature: Vec<String> = workflows.iter().map(|w| format!("{}:{}", s(w, "id"), s(w, "status"))).collect();
        let signature = signature.join(",");
        let is_ended = |w: &Value| workflow_outcome(s(w, "status")).is_some();
        if let Some((seen, run)) = self.ended.get(id)
            && *seen == signature
        {
            return Ok(run.clone());
        }
        let created = s(pipeline, "created_at");
        let age = chrono::DateTime::parse_from_rfc3339(created).map(|at| chrono::Utc::now().timestamp() - at.timestamp()).unwrap_or(i64::MAX);
        let is_errored = s(pipeline, "state") == "errored";
        let is_running = if workflows.is_empty() { !is_errored && age < STARTING_SECS } else { !workflows.iter().all(is_ended) };
        let outcomes: Vec<&str> = workflows.iter().filter_map(|w| workflow_outcome(s(w, "status"))).collect();
        let outcome = if is_running {
            "running"
        } else if is_errored || outcomes.contains(&"failed") {
            "failed"
        } else if outcomes.contains(&"canceled") {
            "canceled"
        } else {
            "succeeded"
        };
        let mut jobs: Vec<(Value, bool)> = Vec::new();
        for workflow in &workflows {
            let ended = is_ended(workflow);
            jobs.extend(items(get, &format!("/api/v2/workflow/{}/job", s(workflow, "id")), 3)?.into_iter().map(|j| (j, ended)));
        }
        // A job left blocked by a failure upstream ends with its workflow.
        let job_ended = |(job, workflow_ended): &(Value, bool)| *workflow_ended || is_job_ended(s(job, "status"));
        let failed_workflow = workflows.iter().find(|w| workflow_outcome(s(w, "status")) == Some("failed"));
        let workflow_id = failed_workflow.or(workflows.first()).map(|w| s(w, "id")).unwrap_or(id);
        let mut run = json!({
            "repo": repo, "branch": branch, "runId": id, "createdAt": created, "isRunning": is_running, "outcome": outcome,
            "workflowId": workflow_id, "done": jobs.iter().filter(|j| job_ended(j)).count(), "total": jobs.len(),
        });
        if let Some((job, _)) = jobs.iter().find(|(j, _)| job_outcome(s(j, "status")) == "failed") {
            run["failed"] = failure(get, job, logs);
        }
        if let Some(image) = jobs.iter().find(|(j, _)| is_image_job(s(j, "name"), image_job)) {
            run["dockerTag"] = json!(if job_ended(image) { job_outcome(s(&image.0, "status")) } else { "running".into() });
        }
        if !is_running {
            self.ended.insert(id.to_string(), (signature, run.clone()));
        }
        Ok(run)
    }
}

/// The failing job: its name and id, the step that failed and its first failed tests, each best effort.
fn failure(get: &mut Get, job: &Value, logs: &dyn Fn(&str) -> String) -> Value {
    let id = s(job, "id");
    let mut failed = json!({"job": s(job, "name"), "jobId": id, "tests": [], "logs": logs(id)});
    let (Some(number), slug) = (job["job_number"].as_u64(), s(job, "project_slug")) else {
        return failed;
    };
    let mut parts = slug.splitn(3, '/');
    let vcs = match parts.next() {
        Some("gh") => "github",
        Some("bb") => "bitbucket",
        Some(other) => other,
        None => "",
    };
    let (org, repo) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    if let Ok(detail) = get(&format!("/api/v1.1/project/{vcs}/{org}/{repo}/{number}")) {
        let step = list(&detail["steps"]).iter().find(|step| {
            list(&step["actions"]).iter().any(|a| a["failed"] == true || a["status"] == "failed" || a["exit_code"].as_i64().is_some_and(|c| c != 0))
        });
        if let Some(name) = step.and_then(|step| step["name"].as_str()) {
            failed["step"] = json!(name);
        }
    }
    if let Ok(tests) = get(&format!("/api/v2/project/{slug}/{number}/tests")) {
        let names: Vec<Value> = list(&tests["items"])
            .iter()
            .filter(|t| t["result"] == "failure")
            .take(3)
            .map(|t| json!(t["name"].as_str().filter(|n| !n.is_empty()).or_else(|| t["classname"].as_str()).unwrap_or("?")))
            .collect();
        failed["tests"] = json!(names);
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/feed/circleci.json")).unwrap()
    }

    fn serve<'a>(answers: &'a Value, asked: &'a mut Vec<String>) -> impl FnMut(&str) -> Fetched + 'a {
        move |path: &str| {
            asked.push(path.to_string());
            answers.get(path).cloned().ok_or_else(|| Failure::Other(format!("HTTP 404: {path}")))
        }
    }

    fn hint(job: &str) -> String {
        logs_hint("CircleCI", &["get_job".into(), "get_job_logs".into()], job)
    }

    #[test]
    fn the_latest_pipeline_of_each_branch_with_its_failure_and_image_job() {
        let answers = answers();
        let mut asked = Vec::new();
        let runs = CircleCi::default().runs(&mut serve(&answers, &mut asked), "acme", "docker_tag", &hint).unwrap();
        assert_eq!(
            runs,
            [
                json!({"repo": "api", "branch": "me/eng-401-login", "runId": "p-api-2", "createdAt": "2026-10-27T15:54:24.775Z", "isRunning": false,
                    "outcome": "failed", "workflowId": "w-api-2b", "done": 3, "total": 3, "dockerTag": "blocked",
                    "failed": {"job": "test", "jobId": "j-test", "step": "mix test", "tests": ["login works", "Acme.SessionTest"],
                        "logs": "the CircleCI MCP (get_job, get_job_logs; keep log tails small), job id j-test"}}),
                json!({"repo": "web", "branch": "me/eng-401-form", "runId": "p-web-1", "createdAt": "2026-10-27T14:00:00.000Z", "isRunning": true,
                    "outcome": "running", "workflowId": "w-web-1", "done": 1, "total": 2, "dockerTag": "running"}),
            ]
        );
        assert!(!asked.iter().any(|p| p.contains("p-api-1") || p.contains("w-api-2a")), "older pipelines and workflows are skipped: {asked:?}");
    }

    #[test]
    fn an_ended_pipeline_is_read_once_until_a_workflow_changes() {
        let answers = answers();
        let mut ci = CircleCi::default();
        ci.runs(&mut serve(&answers, &mut Vec::new()), "acme", "docker_tag", &hint).unwrap();
        let mut asked = Vec::new();
        ci.runs(&mut serve(&answers, &mut asked), "acme", "docker_tag", &hint).unwrap();
        assert!(!asked.contains(&"/api/v2/workflow/w-api-2b/job".to_string()), "{asked:?}");
        assert!(asked.contains(&"/api/v2/workflow/w-web-1/job".to_string()));
        assert!(!asked.contains(&"/api/v2/me/collaborations".to_string()));
    }

    #[test]
    fn the_org_is_the_one_named_like_the_github_org() {
        let answers = answers();
        let mut asked = Vec::new();
        CircleCi::default().runs(&mut serve(&answers, &mut asked), "ACME", "", &hint).unwrap();
        assert!(asked.contains(&"/api/v2/pipeline?org-slug=gh/acme&mine=true".to_string()));
    }

    #[test]
    fn a_failed_listing_fails_the_whole_read() {
        let mut get = |_: &str| -> Fetched { Err(Failure::Limited(9)) };
        assert_eq!(CircleCi::default().runs(&mut get, "acme", "", &hint), Err(Failure::Limited(9)));
    }
}
