//! CI from GitHub's checks: one run per open PR of mine, its head commit's checks, with progress, the failing check
//! (an Actions job first, whose failing step and logs gh can reach) and the image job's state, as ci.json holds them.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::value::list;

/// A cancelled check counts as failed, as in the inbox's PR rollup.
const FAILED: [&str; 6] = ["FAILURE", "ERROR", "TIMED_OUT", "STARTUP_FAILURE", "ACTION_REQUIRED", "CANCELLED"];

fn name(check: &Value) -> String {
    check["name"].as_str().or_else(|| check["context"].as_str()).unwrap_or("?").to_string()
}

/// A job of a GitHub Actions run, rather than a check another app reports.
fn job_id(check: &Value) -> Option<u64> {
    check["databaseId"].as_u64().filter(|_| check["checkSuite"]["workflowRun"]["databaseId"].as_u64().is_some())
}

fn is_done(check: &Value) -> bool {
    if check.get("context").is_some() { !matches!(check["state"].as_str(), Some("PENDING" | "EXPECTED")) } else { check["status"] == "COMPLETED" }
}

fn outcome(check: &Value) -> String {
    let value = check["conclusion"].as_str().or_else(|| check["state"].as_str()).unwrap_or_default();
    match value {
        "SUCCESS" | "NEUTRAL" | "SKIPPED" => "succeeded".into(),
        v if FAILED.contains(&v) => "failed".into(),
        v => v.to_lowercase(),
    }
}

/// Whether a job, check or image build is named after the configured image job.
pub fn is_image_job(name: &str, image_job: &str) -> bool {
    !image_job.is_empty() && (name == image_job || name.ends_with(&format!(": {image_job}")) || name.ends_with(&format!("/{image_job}")))
}

/// The runs of my PRs; `step` reads an Actions job's failing step, asked only when the previous runs do not have it.
pub fn runs(prs: &[Value], prev: &Value, image_job: &str, step: &mut dyn FnMut(&str, u64) -> Option<String>) -> Vec<Value> {
    let cached: HashMap<String, &Value> =
        list(&prev["runs"]).iter().map(|r| (format!("{}/{}/{}", r["repo"].as_str().unwrap_or_default(), r["branch"].as_str().unwrap_or_default(), r["runId"].as_str().unwrap_or_default()), r)).collect();
    let mut out = Vec::new();
    for pr in prs {
        let (Some(branch), Some(repo), Some(slug)) = (pr["headRefName"].as_str(), pr["repository"]["name"].as_str(), pr["repository"]["nameWithOwner"].as_str()) else {
            continue;
        };
        let commit = &pr["commits"]["nodes"][0]["commit"];
        let checks = list(&commit["statusCheckRollup"]["contexts"]["nodes"]);
        let Some(oid) = commit["oid"].as_str().filter(|_| !checks.is_empty()) else {
            continue;
        };
        let is_running = checks.iter().any(|c| !is_done(c));
        let failures: Vec<&Value> = checks.iter().filter(|c| is_done(c) && outcome(c) == "failed").collect();
        let failed = failures.iter().find(|c| job_id(c).is_some()).or(failures.first()).copied();
        let mut workflows: Vec<String> = Vec::new();
        for check in failures.iter().filter(|c| job_id(c).is_some()) {
            let id = check["checkSuite"]["workflowRun"]["databaseId"].to_string();
            if !workflows.contains(&id) {
                workflows.push(id);
            }
        }
        let image = checks.iter().find(|c| is_image_job(&name(c), image_job));
        let mut run = json!({
            "repo": repo, "slug": slug, "branch": branch, "runId": oid, "createdAt": commit["committedDate"], "isRunning": is_running,
            "outcome": if is_running { "running" } else if failed.is_some() { "failed" } else { "succeeded" },
            "done": checks.iter().filter(|c| is_done(c)).count(), "total": checks.len(),
        });
        if !workflows.is_empty() {
            run["workflowId"] = json!(workflows.join(","));
        }
        if let Some(failed) = failed {
            let job = job_id(failed);
            let job_text = job.map(|j| j.to_string()).unwrap_or_default();
            // A rerun keeps the commit, so only the failing step of the same job is reused from the previous runs.
            let known = &cached.get(&format!("{repo}/{branch}/{oid}")).map(|r| &r["failed"]).cloned().unwrap_or(Value::Null);
            let step = if is_running {
                None
            } else if known["jobId"] == job_text.as_str() && known["job"] == name(failed).as_str() {
                known["step"].as_str().map(str::to_string)
            } else {
                job.and_then(|j| step(slug, j))
            };
            let page = failed["detailsUrl"].as_str().or_else(|| failed["targetUrl"].as_str());
            let logs = match (job, page) {
                (Some(j), _) => format!("gh run view --job {j} --log-failed -R {slug}"),
                (None, Some(page)) => format!("its page, {page}"),
                _ => "gh pr checks".into(),
            };
            let mut failure = json!({"job": name(failed), "jobId": job_text, "tests": [], "logs": logs});
            if let Some(step) = step {
                failure["step"] = json!(step);
            }
            run["failed"] = failure;
        }
        if let Some(image) = image {
            run["dockerTag"] = json!(if is_done(image) { outcome(image) } else { "running".into() });
        }
        out.push(run);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prs() -> Vec<Value> {
        let answer: Value = serde_json::from_str(include_str!("../../tests/fixtures/feed/github.json")).unwrap();
        crate::feed::github::parse(&answer["data"]).mine_nodes
    }

    #[test]
    fn a_failed_pr_names_its_actions_job_and_failing_step() {
        let mut asked = Vec::new();
        let runs = runs(&prs(), &Value::Null, "docker_build", &mut |slug, job| {
            asked.push(format!("{slug} {job}"));
            Some("Run cargo clippy".into())
        });
        assert_eq!(asked, ["acme/api 111"]);
        assert_eq!(
            runs,
            [json!({"repo": "api", "slug": "acme/api", "branch": "me/eng-401-login", "runId": "c0ffee4", "createdAt": "2026-10-27T15:50:00Z",
                "isRunning": false, "outcome": "failed", "workflowId": "900", "done": 4, "total": 4, "dockerTag": "succeeded",
                "failed": {"job": "lint", "jobId": "111", "step": "Run cargo clippy", "tests": [], "logs": "gh run view --job 111 --log-failed -R acme/api"}})]
        );
    }

    #[test]
    fn a_known_failing_step_is_not_asked_again() {
        let prev = json!({"runs": [{"repo": "api", "branch": "me/eng-401-login", "runId": "c0ffee4", "failed": {"job": "lint", "jobId": "111", "step": "Lint"}}]});
        let runs = runs(&prs(), &prev, "", &mut |_, _| panic!("asked again"));
        assert_eq!(runs[0]["failed"]["step"], "Lint");
        assert!(runs[0].get("dockerTag").is_none());
    }

    #[test]
    fn a_running_check_keeps_the_run_running() {
        let mut nodes = prs();
        nodes[0]["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"][2]["status"] = json!("IN_PROGRESS");
        let runs = runs(&nodes, &Value::Null, "", &mut |_, _| None);
        assert_eq!((runs[0]["outcome"].as_str(), runs[0]["done"].as_u64()), (Some("running"), Some(3)));
        assert!(runs[0]["failed"].get("step").is_none());
    }

    #[test]
    fn image_jobs_match_by_suffix() {
        assert!(is_image_job("ci/circleci: docker_build", "docker_build"));
        assert!(is_image_job("build/docker_build", "docker_build"));
        assert!(!is_image_job("docker_build_x", "docker_build"));
        assert!(!is_image_job("x", ""));
    }
}
