//! Pull or merge requests from a code host's MCP server (GitLab, Bitbucket, Gitea…), as the inbox lists GitHub's: mine,
//! the ones asking for my review, the merged ones of the week; merged, readied or drafted through its tools.

use serde_json::{Value, json};

use crate::mapping::{list_at, pick, text};
use crate::tools::Tools;

/// The config's code section with its defaults: no tools, the review and CI states as GitHub's.
pub fn mapping(section: &Value) -> Value {
    let mut merged = json!({"name": "Code", "server": "", "tools": {}, "fields": {}, "reviews": {}, "checks": {}});
    for (key, value) in section.as_object().into_iter().flatten() {
        merged[key] = value.clone();
    }
    merged
}

/// One request in the inbox's `Pr` shape: the review and CI states mapped to GitHub's (APPROVED, CHANGES_REQUESTED,
/// REVIEW_REQUIRED; SUCCESS, FAILURE, PENDING).
fn pr(raw: &Value, mapping: &Value, is_direct: bool) -> Value {
    let f = |name: &str| mapping["fields"][name].as_str().unwrap_or("").to_string();
    let get = |name: &str| text(pick(raw, &f(name)));
    let mapped = |table: &str, name: &str| {
        let raw = get(name)?;
        Some(mapping[table][&raw].as_str().map(str::to_string).unwrap_or(raw))
    };
    let repo_path = get("repo").unwrap_or_default();
    let repo = repo_path.trim_end_matches(".git").rsplit('/').next().unwrap_or("").to_string();
    let number = pick(raw, &f("number")).and_then(|n| n.as_i64().or_else(|| n.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0);
    let draft = pick(raw, &f("isDraft")).is_some_and(|d| d == true || d.as_str().is_some_and(|s| s == "true"));
    json!({
        "repo": repo,
        "slug": repo_path.trim_end_matches(".git"),
        "number": number,
        "title": get("title").unwrap_or_default(),
        "url": get("url").unwrap_or_default(),
        "author": get("author").unwrap_or_default(),
        "authorName": get("authorName"),
        "updatedAt": get("updatedAt").unwrap_or_default(),
        "isDraft": draft,
        "isDirect": is_direct,
        "review": mapped("reviews", "review"),
        "ci": mapped("checks", "ci"),
        "branch": get("branch"),
        "base": get("base"),
    })
}

async fn list(tools: &impl Tools, mapping: &Value, verb: &str, is_direct: bool) -> Result<Vec<Value>, String> {
    if !tools.has(verb) {
        return Ok(Vec::new());
    }
    let answer = tools.call(verb, &json!({})).await?;
    let path = mapping["fields"]["prs"].as_str().unwrap_or("");
    let mut prs: Vec<Value> = list_at(&answer, path).into_iter().map(|raw| pr(raw, mapping, is_direct)).collect();
    prs.sort_by(|a, b| b["updatedAt"].as_str().cmp(&a["updatedAt"].as_str()));
    Ok(prs)
}

/// The inbox's code fields: `direct` (asking for my review), `team` (none: a code host's MCP has no team requests),
/// `botRepos`, `mine`, and `merged` (mine merged lately, with the merge commit the deploys check).
pub async fn inbox(tools: &impl Tools, mapping: &Value) -> Result<[Value; 5], String> {
    let direct = list(tools, mapping, "listReviews", true).await?;
    let mine = list(tools, mapping, "listMine", false).await?;
    let merged: Vec<Value> = if tools.has("listMerged") {
        let answer = tools.call("listMerged", &json!({})).await?;
        let f = |name: &str| mapping["fields"][name].as_str().unwrap_or("").to_string();
        list_at(&answer, mapping["fields"]["prs"].as_str().unwrap_or(""))
            .into_iter()
            .map(|raw| {
                let mut p = pr(raw, mapping, false);
                p["mergeSha"] = text(pick(raw, &f("mergeSha"))).unwrap_or_default().into();
                p["mergedAt"] = text(pick(raw, &f("mergedAt"))).unwrap_or_default().into();
                json!({"repo": p["repo"], "slug": p["slug"], "url": p["url"], "number": p["number"], "branch": p["branch"], "mergeSha": p["mergeSha"], "mergedAt": p["mergedAt"]})
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok([json!(direct), json!([]), json!([]), json!(mine), json!(merged)])
}

/// One action on a request (`merge`, `ready`, `draft`), by its repo path and number.
pub async fn act(tools: &impl Tools, verb: &str, pr: &Value) -> Result<(), String> {
    if !tools.has(verb) {
        return Err(format!("no tool mapped for {verb}"));
    }
    let values = json!({"repo": pr["slug"].as_str().or(pr["repo"].as_str()).unwrap_or(""), "number": pr["number"], "url": pr["url"], "branch": pr["branch"]});
    tools.call(verb, &values).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Fake;

    /// A GitLab-shaped mapping.
    fn gitlab() -> Value {
        mapping(&json!({
            "name": "GitLab",
            "tools": {
                "listMine": {"tool": "list_merge_requests", "args": {"scope": "created_by_me", "state": "opened"}},
                "listReviews": {"tool": "list_merge_requests", "args": {"reviewer_username": "me", "state": "opened"}},
                "listMerged": {"tool": "list_merge_requests", "args": {"scope": "created_by_me", "state": "merged"}},
                "merge": {"tool": "merge_merge_request", "args": {"project_id": "{repo}", "merge_request_iid": "{number}"}}
            },
            "fields": {
                "prs": "", "repo": "project_path", "number": "iid", "title": "title", "url": "web_url", "author": "author.username",
                "authorName": "author.name", "updatedAt": "updated_at", "isDraft": "draft", "review": "detailed_merge_status", "ci": "head_pipeline.status",
                "branch": "source_branch", "base": "target_branch", "mergeSha": "merge_commit_sha", "mergedAt": "merged_at"
            },
            "reviews": {"mergeable": "APPROVED", "not_approved": "REVIEW_REQUIRED", "requested_changes": "CHANGES_REQUESTED"},
            "checks": {"success": "SUCCESS", "failed": "FAILURE", "running": "PENDING", "pending": "PENDING"}
        }))
    }

    #[test]
    fn merge_requests_read_as_the_inbox_prs() {
        let mr = |iid: i64, updated: &str, status: &str| {
            json!({"iid": iid, "title": "T", "web_url": format!("https://gitlab.com/acme/api/-/merge_requests/{iid}"), "author": {"username": "me", "name": "Me"},
                "updated_at": updated, "draft": false, "detailed_merge_status": "not_approved", "head_pipeline": {"status": status},
                "source_branch": "eng-1-x", "target_branch": "main", "references": {"full": "acme/api!1"}, "project_path": "acme/api",
                "merge_commit_sha": "abc", "merged_at": "2026-11-03T10:00:00Z"})
        };
        let fake = Fake::default()
            .with("listReviews", json!([mr(7, "2026-11-04T09:00:00Z", "running")]))
            .with("listMine", json!([mr(1, "2026-11-01T09:00:00Z", "success"), mr(2, "2026-11-04T09:00:00Z", "failed")]))
            .with("listMerged", json!([mr(3, "2026-11-03T09:00:00Z", "success")]));
        let [direct, team, _, mine, merged] = crate::block_on(inbox(&fake, &gitlab())).unwrap();
        assert_eq!(
            (direct[0]["isDirect"].clone(), direct[0]["ci"].clone(), direct[0]["review"].clone()),
            (json!(true), json!("PENDING"), json!("REVIEW_REQUIRED"))
        );
        assert_eq!(team, json!([]));
        assert_eq!(mine.as_array().unwrap().iter().map(|p| p["number"].as_i64().unwrap()).collect::<Vec<_>>(), [2, 1]);
        assert_eq!((mine[0]["ci"].clone(), mine[0]["repo"].clone()), (json!("FAILURE"), json!("api")));
        assert_eq!((merged[0]["mergeSha"].clone(), merged[0]["branch"].clone()), (json!("abc"), json!("eng-1-x")));
        let fake = Fake::default().with("merge", json!({}));
        crate::block_on(act(&fake, "merge", &json!({"slug": "acme/api", "number": 2}))).unwrap();
        assert_eq!(fake.calls.borrow()[0].1["number"], 2);
        assert!(crate::block_on(act(&fake, "draft", &json!({}))).is_err());
    }
}
