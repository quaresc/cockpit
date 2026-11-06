//! The latest CI run of each branch I pushed, through a CI connector's mapping: progress, the failing job and the image
//! job, in the inbox's `CiRun` shape.

use serde_json::{Map, Value, json};

use crate::mapping::{list_at, pick, text};
use crate::tools::Tools;

/// The outcomes that end a node when the connector reports no phase.
const FINAL: [&str; 3] = ["succeeded", "failed", "canceled"];

/// The CircleCI connector's mapping, the default for a CircleCI server named without one (config.ts `CIRCLECI`).
pub fn circleci() -> Value {
    json!({
        "enabled": true,
        "name": "CircleCI",
        "server": "circleci",
        "imageJob": "",
        "tools": {
            "listRuns": {"tool": "list_runs", "args": {"mine": true, "limit": 30}},
            "listWorkflows": {"tool": "list_run_workflows", "args": {"run": "{run}"}},
            "listJobs": {"tool": "list_workflow_jobs", "args": {"workflow": "{workflow}"}},
            "getJob": {"tool": "get_job", "args": {"job": "{job}"}},
            "listTests": {"tool": "list_job_tests", "args": {"job": "{job}", "limit": 3}},
            "logs": {"tool": "get_job_logs", "args": {"job": "{job}"}},
            "rerun": {"tool": "rerun_workflow", "args": {"workflow": "{workflow}", "from_failed": true}}
        },
        "fields": {
            "runs": "runs", "runId": "id", "runRepo": "references.event.attributes.vcs.origin_repository_url",
            "runBranch": "references.event.attributes.vcs.branch", "runTag": "references.event.attributes.vcs.tag",
            "runCreated": "attributes.created_at", "runPhase": "attributes.phase", "runOutcome": "attributes.current_outcome",
            "workflows": "workflows", "workflowId": "id", "workflowName": "attributes.name", "workflowCreated": "attributes.created_at",
            "workflowOutcome": "attributes.outcome", "jobs": "jobs", "jobId": "id", "jobName": "attributes.name", "jobPhase": "attributes.phase",
            "jobOutcome": "attributes.outcome", "steps": "attributes.parallel_executions[].steps", "stepName": "name", "stepOutcome": "outcome",
            "stepExit": "exit_code", "tests": "tests", "testName": "name|classname"
        },
        "outcomes": {},
        "endedPhases": ["ended"]
    })
}

/// The config's ci section over its base, key by key as the plugin's loadConfig spreads it: CircleCI's mapping for a
/// CircleCI server that maps no tools, else an empty one.
pub fn mapping(section: &Value) -> Value {
    let is_circleci = section["server"].as_str().is_some_and(|s| s.to_lowercase().contains("circleci"));
    let mut merged = if is_circleci && section["tools"].is_null() {
        circleci()
    } else {
        json!({"enabled": true, "name": "GitHub", "server": "", "imageJob": "", "tools": {}, "fields": {}, "outcomes": {}, "endedPhases": []})
    };
    if let (Some(base), Some(over)) = (merged.as_object_mut(), section.as_object()) {
        base.extend(over.clone());
    }
    merged
}

/// The latest run of each repo and branch, tags left out; an ended run already in `prev` is reused as it is.
pub async fn runs(tools: &impl Tools, ci: &Value, prev: &Value) -> Result<Vec<Value>, String> {
    let f = |name: &str| field(ci, name);
    let answer = tools.call("listRuns", &json!({})).await?;
    let mut latest: Vec<(String, String, &Value)> = Vec::new();
    for run in list_at(&answer, &f("runs")) {
        let url = text(pick(run, &f("runRepo"))).unwrap_or_default();
        let repo = url.strip_suffix(".git").unwrap_or(&url).rsplit('/').next().unwrap_or_default().to_string();
        let branch = text(pick(run, &f("runBranch"))).unwrap_or_default();
        if repo.is_empty() || branch.is_empty() || !text(pick(run, &f("runTag"))).unwrap_or_default().is_empty() {
            continue;
        }
        let created = created_at(run, &f("runCreated"));
        match latest.iter_mut().find(|(r, b, _)| *r == repo && *b == branch) {
            Some(seen) if created_at(seen.2, &f("runCreated")) < created => seen.2 = run,
            Some(_) => {}
            None => latest.push((repo, branch, run)),
        }
    }
    let cached: Map<String, Value> =
        prev["runs"].as_array().into_iter().flatten().filter_map(|r| Some((r["runId"].as_str()?.to_string(), r.clone()))).collect();
    let mut runs = Vec::new();
    for (repo, branch, run) in latest {
        let outcome = outcome_of(ci, pick(run, &f("runOutcome")));
        let is_running = !is_ended(ci, run, &f("runPhase"), &f("runOutcome"));
        let base = json!({
            "repo": repo,
            "branch": branch,
            "runId": text(pick(run, &f("runId"))).unwrap_or_default(),
            "createdAt": created_at(run, &f("runCreated")),
            "isRunning": is_running,
            "outcome": if outcome.is_empty() { "running".to_string() } else { outcome },
        });
        // An ended run never changes, so its details are fetched once.
        match base["runId"].as_str().and_then(|id| cached.get(id)) {
            Some(known) if known["isRunning"].as_bool() != Some(true) && !is_running => runs.push(known.clone()),
            _ => runs.push(details_of(tools, ci, base.clone()).await.unwrap_or_else(|_| with(base, json!({"done": 0, "total": 0})))),
        }
    }
    Ok(runs)
}

/// Reruns the failed jobs of a workflow through the connector's rerun tool.
pub async fn rerun(tools: &impl Tools, workflow_id: &str) -> Result<(), String> {
    tools.call("rerun", &json!({"workflow": workflow_id, "run": workflow_id})).await.map(|_| ())
}

/// A run's jobs, counted, with its failing job, its image job and the workflow a rerun targets.
async fn details_of(tools: &impl Tools, ci: &Value, base: Value) -> Result<Value, String> {
    let f = |name: &str| field(ci, name);
    let run_id = base["runId"].as_str().unwrap_or_default().to_string();
    let mut jobs: Vec<Value> = Vec::new();
    let mut workflows: Vec<(String, bool)> = Vec::new();
    if tools.has("listWorkflows") {
        // A rerun adds a workflow of the same name to the run: only the newest of each name counts.
        let answer = tools.call("listWorkflows", &json!({"run": run_id})).await?;
        let mut newest: Vec<(String, &Value)> = Vec::new();
        for workflow in list_at(&answer, &f("workflows")) {
            let name = text(pick(workflow, &f("workflowName"))).unwrap_or_default();
            let created = created_at(workflow, &f("workflowCreated"));
            match newest.iter_mut().find(|(n, _)| *n == name) {
                Some(seen) if created_at(seen.1, &f("workflowCreated")) < created => seen.1 = workflow,
                Some(_) => {}
                None => newest.push((name, workflow)),
            }
        }
        workflows = newest
            .into_iter()
            .map(|(_, w)| (text(pick(w, &f("workflowId"))).unwrap_or_default(), outcome_of(ci, pick(w, &f("workflowOutcome"))) == "failed"))
            .collect();
        for (id, _) in &workflows {
            let answer = tools.call("listJobs", &json!({"workflow": id, "run": run_id})).await?;
            jobs.extend(list_at(&answer, &f("jobs")).into_iter().cloned());
        }
    } else {
        let answer = tools.call("listJobs", &json!({"run": run_id, "workflow": run_id})).await?;
        jobs.extend(list_at(&answer, &f("jobs")).into_iter().cloned());
    }
    let image_job = ci["imageJob"].as_str().unwrap_or_default();
    let failed_job = jobs.iter().find(|j| outcome_of(ci, pick(j, &f("jobOutcome"))) == "failed");
    let image = jobs.iter().find(|j| is_image_job(&text(pick(j, &f("jobName"))).unwrap_or_default(), image_job));
    let workflow_id = workflows.iter().find(|(_, is_failed)| *is_failed).or(workflows.first()).map_or(run_id.clone(), |(id, _)| id.clone());
    let mut extra = json!({
        "workflowId": workflow_id,
        "done": jobs.iter().filter(|j| is_ended(ci, j, &f("jobPhase"), &f("jobOutcome"))).count(),
        "total": jobs.len(),
    });
    if let Some(job) = failed_job {
        extra["failed"] = failure_of(tools, ci, job).await;
    }
    if let Some(image) = image {
        let ended = is_ended(ci, image, &f("jobPhase"), &f("jobOutcome"));
        extra["dockerTag"] = json!(if ended { outcome_of(ci, pick(image, &f("jobOutcome"))) } else { "running".to_string() });
    }
    Ok(with(base, extra))
}

/// The failing job with its failing step and first failed tests, each left out when its tool fails.
async fn failure_of(tools: &impl Tools, ci: &Value, job: &Value) -> Value {
    let f = |name: &str| field(ci, name);
    let id = text(pick(job, &f("jobId"))).unwrap_or_default();
    let detail = match tools.has("getJob") {
        true => tools.call("getJob", &json!({"job": id})).await.unwrap_or(Value::Null),
        false => Value::Null,
    };
    let step = list_at(&detail, &f("steps")).into_iter().find(|s| {
        let exit = pick(s, &f("stepExit"));
        outcome_of(ci, pick(s, &f("stepOutcome"))) == "failed" || exit.is_some_and(|e| e.as_f64() != Some(0.0))
    });
    let tests: Vec<String> = match tools.has("listTests") {
        true => tools
            .call("listTests", &json!({"job": id}))
            .await
            .map(|t| list_at(&t, &f("tests")).into_iter().take(3).map(|x| text(pick(x, &f("testName"))).unwrap_or_else(|| "?".into())).collect())
            .unwrap_or_default(),
        false => Vec::new(),
    };
    let mut failure = json!({
        "job": text(pick(job, &f("jobName"))).unwrap_or_else(|| "?".into()),
        "jobId": id,
        "tests": tests,
        "logs": logs_hint(ci, &id),
    });
    if let Some(step) = step.and_then(|s| text(pick(s, &f("stepName")))) {
        failure["step"] = json!(step);
    }
    failure
}

/// How a session reads the failing job's logs, from the tools the connector has.
fn logs_hint(ci: &Value, job: &str) -> String {
    let tools: Vec<&str> = ["getJob", "listTests", "logs"].iter().filter_map(|c| ci["tools"][c]["tool"].as_str().filter(|t| !t.is_empty())).collect();
    let name = text(Some(&ci["name"])).unwrap_or_default();
    let list = if tools.is_empty() { String::new() } else { format!(" ({}; keep log tails small)", tools.join(", ")) };
    format!("cockpit's cockpit_read tool (source ci, verb logs or getJob, values {{\"job\": \"{job}\"}}), else the {name} MCP{list}, job id {job}")
}

/// A connector's outcome as cockpit's, through the mapping's outcomes.
fn outcome_of(ci: &Value, value: Option<&Value>) -> String {
    let raw = text(value).unwrap_or_default();
    ci["outcomes"][&raw].as_str().map(str::to_string).unwrap_or(raw)
}

/// Ended by its phase when the connector reports one, else by a final outcome.
fn is_ended(ci: &Value, node: &Value, phase_path: &str, outcome_path: &str) -> bool {
    if phase_path.is_empty() {
        return FINAL.contains(&outcome_of(ci, pick(node, outcome_path)).as_str());
    }
    let phase = text(pick(node, phase_path)).unwrap_or_default();
    ci["endedPhases"].as_array().is_some_and(|ended| ended.iter().any(|p| p.as_str() == Some(phase.as_str())))
}

/// Whether a job is the configured image job, alone or under a workflow's prefix.
fn is_image_job(name: &str, image_job: &str) -> bool {
    !image_job.is_empty() && (name == image_job || name.ends_with(&format!(": {image_job}")) || name.ends_with(&format!("/{image_job}")))
}

fn field(ci: &Value, name: &str) -> String {
    ci["fields"][name].as_str().unwrap_or_default().to_string()
}

fn created_at(node: &Value, path: &str) -> String {
    text(pick(node, path)).unwrap_or_default()
}

fn with(mut base: Value, extra: Value) -> Value {
    if let (Some(base), Value::Object(extra)) = (base.as_object_mut(), extra) {
        base.extend(extra);
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use crate::tools::Fake;

    fn run(id: &str, repo: &str, branch: &str, created: &str, phase: &str, outcome: &str) -> Value {
        json!({
            "id": id,
            "attributes": {"created_at": created, "phase": phase, "current_outcome": outcome},
            "references": {"event": {"attributes": {"vcs": {"origin_repository_url": format!("https://github.com/acme/{repo}.git"), "branch": branch}}}}
        })
    }

    fn workflow(id: &str, name: &str, created: &str, outcome: &str) -> Value {
        json!({"id": id, "attributes": {"name": name, "created_at": created, "outcome": outcome}})
    }

    fn job(id: &str, name: &str, phase: &str, outcome: Value) -> Value {
        json!({"id": id, "attributes": {"name": name, "phase": phase, "outcome": outcome}})
    }

    fn calls(fake: &Fake, verb: &str) -> Vec<Value> {
        fake.calls.borrow().iter().filter(|(v, _)| v == verb).map(|(_, values)| values.clone()).collect()
    }

    #[test]
    fn latest_run_of_each_branch_without_tags() {
        let mut tagged = run("r4", "api", "main", "2026-11-04T12:00:00Z", "ended", "success");
        tagged["references"]["event"]["attributes"]["vcs"]["tag"] = json!("v1.2.0");
        let fake = Fake::default()
            .with(
                "listRuns",
                json!({"runs": [
                    run("r1", "api", "main", "2026-11-04T09:00:00Z", "ended", "success"),
                    run("r2", "api", "cq/roi-1-x", "2026-11-04T09:30:00Z", "running", "running"),
                    run("r3", "api", "main", "2026-11-04T10:00:00Z", "ended", "success"),
                    tagged
                ]}),
            )
            .with("listWorkflows", json!({"workflows": [workflow("w1", "build", "2026-11-04T10:00:01Z", "success")]}))
            .with("listJobs", json!({"jobs": [job("j1", "test", "ended", json!("success")), job("j2", "lint", "running", Value::Null)]}));
        let runs = block_on(runs(&fake, &circleci(), &Value::Null)).unwrap();
        let ids: Vec<&str> = runs.iter().map(|r| r["runId"].as_str().unwrap()).collect();
        assert_eq!(ids, ["r3", "r2"]);
        assert_eq!((runs[0]["repo"].as_str(), runs[1]["branch"].as_str()), (Some("api"), Some("cq/roi-1-x")));
        assert_eq!((runs[0]["isRunning"].as_bool(), runs[1]["isRunning"].as_bool()), (Some(false), Some(true)));
        assert_eq!((runs[0]["done"].as_u64(), runs[0]["total"].as_u64()), (Some(1), Some(2)));
        assert_eq!(runs[0]["workflowId"], "w1");
        assert!(runs[0].get("failed").is_none() && runs[0].get("dockerTag").is_none());
        assert_eq!(calls(&fake, "listWorkflows")[0], json!({"run": "r3"}));
    }

    #[test]
    fn ended_run_reused_from_prev() {
        let fake = Fake::default().with("listRuns", json!({"runs": [run("r1", "api", "main", "2026-11-04T09:00:00Z", "ended", "success")]}));
        let known = json!({"repo": "api", "branch": "main", "runId": "r1", "createdAt": "x", "isRunning": false, "outcome": "success", "done": 5, "total": 5});
        let prev = json!({"at": 1, "version": 3, "runs": [known]});
        let runs = block_on(runs(&fake, &circleci(), &prev)).unwrap();
        assert_eq!(runs, vec![known]);
        assert!(calls(&fake, "listWorkflows").is_empty() && calls(&fake, "listJobs").is_empty());
    }

    #[test]
    fn details_failing_count_as_none_done() {
        let fake = Fake::default().with("listRuns", json!({"runs": [run("r1", "api", "main", "2026-11-04T09:00:00Z", "running", "")]}));
        let runs = block_on(runs(&fake, &circleci(), &Value::Null)).unwrap();
        assert_eq!(runs[0]["outcome"], "running");
        assert_eq!((runs[0]["done"].as_u64(), runs[0]["total"].as_u64()), (Some(0), Some(0)));
        assert!(runs[0].get("workflowId").is_none());
    }

    #[test]
    fn failing_job_image_job_and_newest_workflow_after_a_rerun() {
        let ci = mapping(&json!({"server": "circleci", "imageJob": "docker_tag"}));
        let detail = json!({"attributes": {"parallel_executions": [{"steps": [
            {"name": "Checkout code", "outcome": "success", "exit_code": 0},
            {"name": "mix test", "outcome": "failed", "exit_code": 2}
        ]}]}});
        let fake = Fake::default()
            .with("listRuns", json!({"runs": [run("r1", "api", "main", "2026-11-04T09:00:00Z", "ended", "failed")]}))
            .with(
                "listWorkflows",
                json!({"workflows": [
                    workflow("w-old", "build", "2026-11-04T09:00:01Z", "failed"),
                    workflow("w-deploy", "deploy", "2026-11-04T09:00:02Z", "success"),
                    workflow("w-new", "build", "2026-11-04T09:20:00Z", "failed")
                ]}),
            )
            .with("listJobs", json!({"jobs": [job("j1", "test", "ended", json!("failed")), job("j2", "build: docker_tag", "running", Value::Null)]}))
            .with("listJobs", json!({"jobs": [job("j3", "deploy", "ended", json!("success"))]}))
            .with("getJob", detail)
            .with("listTests", json!({"tests": [{"name": "quota test"}, {"classname": "QuotaTest"}, {"name": "c"}, {"name": "d"}]}));
        let runs = block_on(runs(&fake, &ci, &Value::Null)).unwrap();
        let listed: Vec<Value> = calls(&fake, "listJobs").iter().map(|c| c["workflow"].clone()).collect();
        assert_eq!(listed, [json!("w-new"), json!("w-deploy")]);
        assert_eq!(runs[0]["workflowId"], "w-new");
        assert_eq!(runs[0]["dockerTag"], "running");
        assert_eq!((runs[0]["done"].as_u64(), runs[0]["total"].as_u64()), (Some(2), Some(3)));
        assert_eq!(
            runs[0]["failed"],
            json!({
                "job": "test", "jobId": "j1", "step": "mix test", "tests": ["quota test", "QuotaTest", "c"],
                "logs": "cockpit's cockpit_read tool (source ci, verb logs or getJob, values {\"job\": \"j1\"}), else the CircleCI MCP (get_job, list_job_tests, get_job_logs; keep log tails small), job id j1"
            })
        );
        assert_eq!(calls(&fake, "getJob")[0], json!({"job": "j1"}));
    }

    #[test]
    fn outcomes_and_ended_phases_from_the_mapping() {
        let section = json!({
            "name": "GitLab", "server": "gitlab", "imageJob": "image",
            "tools": {"listRuns": {"tool": "list_pipelines"}, "listJobs": {"tool": "list_jobs"}},
            "fields": {"runs": "", "runId": "id", "runRepo": "project", "runBranch": "ref", "runCreated": "created", "runOutcome": "status",
                "jobs": "", "jobId": "id", "jobName": "name", "jobPhase": "stage_state", "jobOutcome": "status"},
            "outcomes": {"success": "succeeded", "failed_hard": "failed"},
            "endedPhases": ["done"]
        });
        let ci = mapping(&section);
        assert!(ci["tools"]["getJob"].is_null() && ci["fields"]["runPhase"].is_null());
        let fake =
            Fake::default().with("listRuns", json!([{"id": 7, "project": "group/web", "ref": "main", "created": "2026-11-04", "status": "failed_hard"}])).with(
                "listJobs",
                json!([
                    {"id": 1, "name": "rspec", "stage_state": "done", "status": "failed_hard"},
                    {"id": 2, "name": "image", "stage_state": "done", "status": "success"},
                    {"id": 3, "name": "e2e", "stage_state": "pending", "status": "created"}
                ]),
            );
        let runs = block_on(runs(&fake, &ci, &Value::Null)).unwrap();
        assert_eq!(runs[0]["repo"], "web");
        assert_eq!((runs[0]["outcome"].as_str(), runs[0]["isRunning"].as_bool()), (Some("failed"), Some(false)));
        assert_eq!((runs[0]["done"].as_u64(), runs[0]["total"].as_u64()), (Some(2), Some(3)));
        assert_eq!(runs[0]["workflowId"], "7");
        assert_eq!(runs[0]["dockerTag"], "succeeded");
        assert_eq!(
            runs[0]["failed"],
            json!({"job": "rspec", "jobId": "1", "tests": [], "logs": "cockpit's cockpit_read tool (source ci, verb logs or getJob, values {\"job\": \"1\"}), else the GitLab MCP, job id 1"})
        );
        assert_eq!(calls(&fake, "listJobs")[0], json!({"run": "7", "workflow": "7"}));
    }

    #[test]
    fn mapping_keeps_circleci_unless_tools_are_mapped() {
        let ci = mapping(&json!({"server": "claude.ai CircleCI", "imageJob": "docker_tag", "outcomes": {"x": "failed"}}));
        assert_eq!((ci["name"].as_str(), ci["imageJob"].as_str()), (Some("CircleCI"), Some("docker_tag")));
        assert_eq!(ci["outcomes"], json!({"x": "failed"}));
        assert_eq!(ci["tools"]["rerun"]["tool"], "rerun_workflow");
        assert_eq!(mapping(&json!({"server": "circleci", "tools": {}}))["fields"], json!({}));
    }

    #[test]
    fn rerun_calls_the_mapped_tool() {
        let fake = Fake::default().with("rerun", json!({"ok": true}));
        block_on(rerun(&fake, "w1")).unwrap();
        assert_eq!(calls(&fake, "rerun")[0], json!({"workflow": "w1", "run": "w1"}));
    }
}
