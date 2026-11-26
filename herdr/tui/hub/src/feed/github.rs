//! GitHub in one GraphQL query: my review requests (direct or through a team, bots apart, every page of them), my open
//! PRs with their review state and checks, and what I merged these 30 days; the inbox's GitHub fields read from it, as the
//! mod writes them.

use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};

use super::http::{self, Failure, Fetched};
use crate::proc;
use crate::value::list;

pub fn api() -> String {
    http::base("COCKPIT_GITHUB_API", "https://api.github.com")
}

static TOKEN: Mutex<Option<String>> = Mutex::new(None);

/// GH_TOKEN, else GITHUB_TOKEN, else what `gh auth token` prints; kept until GitHub refuses it.
pub fn token() -> Option<String> {
    let mut kept = TOKEN.lock().ok()?;
    if kept.is_none() {
        let from_env = ["GH_TOKEN", "GITHUB_TOKEN"].iter().find_map(|v| std::env::var(v).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()));
        *kept = from_env.or_else(|| {
            let out = proc::run(&proc::strings(&["gh", "auth", "token"]), Duration::from_secs(10), None);
            out.ok().then(|| cockpit_connectors::printed_token(&out.stdout)).flatten()
        });
    }
    kept.clone()
}

/// A refused token is read again next time: `gh auth login` may have replaced it.
pub fn forget_token() {
    if let Ok(mut kept) = TOKEN.lock() {
        *kept = None;
    }
}

const PR_BASE: &str = "number title url isDraft updatedAt headRefName baseRefName headRefOid isCrossRepository \
  headRepositoryOwner { login } repository { name nameWithOwner } author { __typename login ... on User { name } } reviewDecision";

/// How many review requests one page of the search holds. Small: the first page shares its query with my own PRs' checks,
/// and a hundred requests there made GitHub answer HTTP 502 (a page of fifty is lighter than the sixty it replaces).
const REVIEW_PAGE: usize = 50;

/// The most pages of review requests followed: a thousand PRs, far past what one person reviews by hand.
const REVIEW_PAGES: usize = 20;

/// The search for my review requests, one page of it: the first, or the one after the cursor `after`. Bots' PRs are
/// listed with the rest (they are told apart on reading), so they must all be fetched for the people's not to be cut off.
/// Review requests only need the rollup: listing the checks of every one makes the query too heavy for GitHub (HTTP 502).
fn review_search(after: Option<&str>) -> String {
    let after = after.map(|c| c.chars().filter(|c| c.is_ascii_alphanumeric() || "=_-+/".contains(*c)).collect::<String>());
    let after = after.map(|c| format!(", after: \"{c}\"")).unwrap_or_default();
    format!(
        r#"search(query: "is:pr is:open archived:false review-requested:@me", type: ISSUE, first: {REVIEW_PAGE}{after}) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ __typename ... on PullRequest {{ {PR_BASE} commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ state }} }} }} }} }} }} }}"#
    )
}

/// My own PRs list their checks, which give both the failing names and the CI runs.
pub fn query(org: &str, merged_since: &str) -> String {
    let org: String = org.chars().filter(|c| c.is_ascii_alphanumeric() || "_.-".contains(*c)).collect();
    let scope = if org.is_empty() { String::new() } else { format!(" org:{org}") };
    let review = review_search(None);
    format!(
        r#"query {{
  review: {review}
  direct: search(query: "is:pr is:open archived:false user-review-requested:@me", type: ISSUE, first: 50) {{ nodes {{ ... on PullRequest {{ url }} }} }}
  mine: search(query: "is:pr is:open archived:false author:@me{scope}", type: ISSUE, first: 30) {{ nodes {{ __typename ... on PullRequest {{ {PR_BASE} reviewThreads(first: 50) {{ nodes {{ isResolved }} }}
    commits(last: 1) {{ nodes {{ commit {{ oid committedDate statusCheckRollup {{ state contexts(first: 30) {{ nodes {{
      ... on CheckRun {{ databaseId name status conclusion detailsUrl checkSuite {{ workflowRun {{ databaseId }} }} }}
      ... on StatusContext {{ context state targetUrl }} }} }} }} }} }} }} }} }} }}
  merged: search(query: "is:pr is:merged author:@me{scope} merged:>={merged_since}", type: ISSUE, first: 50) {{
    nodes {{ ... on PullRequest {{ number url headRefName headRefOid createdAt mergedAt repository {{ name nameWithOwner }} mergeCommit {{ oid }} }} }} }}
}}"#
    )
}

/// The inbox's GitHub fields, and my open PRs as GitHub gave them, for their CI runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Github {
    pub direct: Vec<Value>,
    pub team: Vec<Value>,
    pub bot_repos: Vec<String>,
    /// The review requests whose author is a bot, as PRs: what the Review section folds away, not counts only.
    pub bots: Vec<Value>,
    pub mine: Vec<Value>,
    pub merged: Vec<Value>,
    pub mine_nodes: Vec<Value>,
}

/// `data` with the review requests of the pages after its first appended, as long as GitHub says there are more, up to
/// REVIEW_PAGES. `more` is handed a page's cursor and gives that page's `review` search, or None when it cannot: what was
/// read stays, as for any answer that is partly there.
fn with_every_review_request(data: &mut Value, mut more: impl FnMut(&str) -> Option<Value>) {
    for _ in 1..REVIEW_PAGES {
        let next = &data["review"]["pageInfo"];
        let Some(cursor) = next["endCursor"].as_str().filter(|_| next["hasNextPage"] == true).map(str::to_string) else { return };
        let Some(page) = more(&cursor) else { return };
        let mut nodes = list(&data["review"]["nodes"]).to_vec();
        nodes.extend_from_slice(list(&page["nodes"]));
        data["review"]["nodes"] = json!(nodes);
        data["review"]["pageInfo"] = page["pageInfo"].clone();
    }
}

/// One query, then the rest of the review requests a page at a time; the answer's data even beside errors (a repo behind
/// SAML), unless it has none.
pub fn fetch(token: &str, org: &str) -> Fetched<Github> {
    let since = (chrono::Utc::now() - chrono::Duration::days(30)).format("%Y-%m-%d").to_string();
    let auth = format!("bearer {token}");
    let url = format!("{}/graphql", api());
    let body = http::post(&url, &[("Authorization", &auth)], &json!({"query": query(org, &since)}));
    if matches!(&body, Err(Failure::Other(m)) if m.contains("token refused")) {
        forget_token();
    }
    let body = body?;
    if body["data"].is_null() {
        return Err(Failure::Other(http::message(&body)));
    }
    let mut data = body["data"].clone();
    with_every_review_request(&mut data, |cursor| {
        let page = http::post(&url, &[("Authorization", &auth)], &json!({"query": format!("query {{ review: {} }}", review_search(Some(cursor)))})).ok()?;
        Some(page["data"]["review"].clone()).filter(|r| !r.is_null())
    });
    Ok(parse(&data))
}

const FAILED: [&str; 5] = ["FAILURE", "ERROR", "TIMED_OUT", "CANCELLED", "STARTUP_FAILURE"];

fn is_pr(node: &Value) -> bool {
    node["__typename"] == "PullRequest"
}

fn or(value: &Value, default: Value) -> Value {
    if value.is_null() { default } else { value.clone() }
}

fn to_pr(node: &Value, direct: &[&str]) -> Value {
    let url = node["url"].as_str().unwrap_or_default();
    let rollup = &node["commits"]["nodes"][0]["commit"]["statusCheckRollup"];
    let failed: Vec<Value> = list(&rollup["contexts"]["nodes"])
        .iter()
        .filter(|c| {
            let state = c["conclusion"].as_str().or_else(|| c["state"].as_str()).unwrap_or_default();
            FAILED.contains(&state)
        })
        .map(|c| or(&c["name"], or(&c["context"], json!("?"))))
        .collect();
    let mut pr = json!({
        "repo": or(&node["repository"]["name"], json!("?")),
        "number": or(&node["number"], json!(0)),
        "title": or(&node["title"], json!("")),
        "url": url,
        "author": or(&node["author"]["login"], json!("?")),
        "authorName": or(&node["author"]["name"], json!("")),
        "updatedAt": or(&node["updatedAt"], json!("1970-01-01T00:00:00.000Z")),
        "isDraft": or(&node["isDraft"], json!(false)),
        "isDirect": direct.contains(&url),
        "review": node["reviewDecision"],
        "ci": rollup["state"],
        "failedChecks": failed,
    });
    if let Some(threads) = node["reviewThreads"]["nodes"].as_array() {
        pr["threads"] = json!(threads.iter().filter(|t| t["isResolved"] != true).count());
    }
    for (field, key) in [("branch", "headRefName"), ("base", "baseRefName"), ("headOid", "headRefOid")] {
        if let Some(value) = node[key].as_str() {
            pr[field] = json!(value);
        }
    }
    // Start work fetches a fork's PR apart from your own branches.
    if let Some(cross) = node["isCrossRepository"].as_bool() {
        pr["isCrossRepository"] = json!(cross);
        pr["headOwner"] = node["headRepositoryOwner"]["login"].clone();
    }
    pr
}

pub fn parse(data: &Value) -> Github {
    let direct: Vec<&str> = list(&data["direct"]["nodes"]).iter().filter_map(|n| n["url"].as_str()).collect();
    let prs: Vec<&Value> = list(&data["review"]["nodes"]).iter().filter(|n| is_pr(n)).collect();
    let is_bot = |n: &Value| n["author"]["__typename"] == "Bot";
    let humans: Vec<Value> = prs.iter().filter(|n| !is_bot(n)).map(|n| to_pr(n, &direct)).collect();
    let mine_nodes: Vec<Value> = list(&data["mine"]["nodes"]).iter().filter(|n| is_pr(n)).cloned().collect();
    Github {
        direct: humans.iter().filter(|p| p["isDirect"] == true).cloned().collect(),
        team: humans.iter().filter(|p| p["isDirect"] != true).cloned().collect(),
        bot_repos: prs.iter().filter(|n| is_bot(n)).map(|n| n["repository"]["name"].as_str().unwrap_or("?").to_string()).collect(),
        bots: prs.iter().filter(|n| is_bot(n)).map(|n| to_pr(n, &direct)).collect(),
        mine: mine_nodes.iter().map(|n| to_pr(n, &direct)).collect(),
        merged: list(&data["merged"]["nodes"])
            .iter()
            .filter(|n| !n["mergeCommit"].is_null() && n["headRefName"].as_str().is_some_and(|b| !b.is_empty()))
            .map(|n| {
                let mut pr = json!({
                    "repo": or(&n["repository"]["name"], json!("?")),
                    "slug": or(&n["repository"]["nameWithOwner"], json!("?")),
                    "url": or(&n["url"], json!("")),
                    "number": or(&n["number"], json!(0)),
                    "branch": n["headRefName"],
                    "mergeSha": or(&n["mergeCommit"]["oid"], json!("")),
                    // The branch's last commit as merged: a worktree past it holds work the merge did not take.
                    "headSha": or(&n["headRefOid"], json!("")),
                    "mergedAt": or(&n["mergedAt"], json!("")),
                });
                if let Some(created) = n["createdAt"].as_str() {
                    pr["createdAt"] = json!(created);
                }
                pr
            })
            .collect(),
        mine_nodes,
    }
}

/// The failing step of a GitHub Actions job.
pub fn failed_step(token: &str, slug: &str, job: u64) -> Option<String> {
    let auth = format!("bearer {token}");
    let body = http::get(&format!("{}/repos/{slug}/actions/jobs/{job}", api()), &[("Authorization", &auth), ("Accept", "application/vnd.github+json")]).ok()?;
    list(&body["steps"]).iter().find(|s| s["conclusion"] == "failure").and_then(|s| s["name"].as_str()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/feed/github.json")).unwrap()
    }

    #[test]
    fn review_requests_split_into_direct_team_and_bots() {
        let github = parse(&answer()["data"]);
        let urls = |prs: &[Value]| prs.iter().map(|p| p["url"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(urls(&github.direct), ["https://github.com/acme/web/pull/9"]);
        assert_eq!(urls(&github.team), ["https://github.com/acme/docs/pull/3"]);
        assert_eq!(github.bot_repos, ["api"]);
        assert_eq!(github.bots.len(), 1, "a bot's request is kept as a PR, not only counted");
        assert_eq!(github.bots[0]["repo"], "api");
        assert_eq!(github.direct[0]["authorName"], "Maya Haddad");
        assert_eq!(github.direct[0]["ci"], "PENDING");
        assert_eq!(github.team[0]["isDirect"], false);
    }

    fn page(numbers: &[u64], next: Option<&str>) -> Value {
        let nodes: Vec<Value> = numbers.iter().map(|n| json!({"__typename": "PullRequest", "number": n})).collect();
        json!({"pageInfo": {"hasNextPage": next.is_some(), "endCursor": next}, "nodes": nodes})
    }

    fn numbers(review: &Value) -> Vec<u64> {
        list(&review["nodes"]).iter().filter_map(|n| n["number"].as_u64()).collect()
    }

    #[test]
    fn every_page_of_review_requests_is_read_so_bots_do_not_cut_the_people_off() {
        let mut data = json!({"review": page(&[1, 2], Some("c2"))});
        let mut asked = Vec::new();
        with_every_review_request(&mut data, |cursor| {
            asked.push(cursor.to_string());
            Some(if cursor == "c2" { page(&[3, 4], Some("c4")) } else { page(&[5], None) })
        });
        assert_eq!(asked, ["c2", "c4"]);
        assert_eq!(numbers(&data["review"]), [1, 2, 3, 4, 5]);
        assert_eq!(data["review"]["pageInfo"]["hasNextPage"], false);
    }

    #[test]
    fn a_page_that_cannot_be_read_leaves_the_pages_before_it() {
        let mut data = json!({"review": page(&[1], Some("c1"))});
        with_every_review_request(&mut data, |_| None);
        assert_eq!(numbers(&data["review"]), [1]);
        let mut whole = json!({"review": page(&[1], None)});
        with_every_review_request(&mut whole, |_| panic!("no page is asked for after the last"));
    }

    #[test]
    fn the_pages_followed_are_bounded() {
        let mut data = json!({"review": page(&[0], Some("more"))});
        let mut asked = 0;
        with_every_review_request(&mut data, |_| {
            asked += 1;
            Some(page(&[asked], Some("more")))
        });
        assert_eq!(asked as usize, REVIEW_PAGES - 1);
    }

    #[test]
    fn the_review_search_asks_for_a_page_after_a_cursor_that_is_only_cursor_characters() {
        assert!(review_search(None).contains("first: 50) { pageInfo { hasNextPage endCursor } nodes"));
        assert!(review_search(Some("Y3Vyc29yOjEwMA==")).contains(r#"first: 50, after: "Y3Vyc29yOjEwMA==")"#));
        assert!(review_search(Some(r#"a"b) { x"#)).contains(r#"after: "abx")"#), "a quote or a brace never leaves the string");
    }

    #[test]
    fn my_prs_carry_their_review_state_and_failing_checks() {
        let github = parse(&answer()["data"]);
        let pr = &github.mine[0];
        assert_eq!(
            *pr,
            json!({"repo": "api", "number": 4, "title": "Google sign-in", "url": "https://github.com/acme/api/pull/4", "author": "me",
                "authorName": "Me Too", "updatedAt": "2026-10-27T15:53:26Z", "isDraft": false, "isDirect": false, "review": "CHANGES_REQUESTED",
                "ci": "FAILURE", "branch": "me/eng-401-login", "base": "main", "failedChecks": ["lint", "ci/circleci: test"]})
        );
        assert_eq!(github.mine[1]["ci"], Value::Null);
        assert_eq!(github.mine_nodes.len(), 2);
    }

    #[test]
    fn merged_prs_need_a_merge_commit_and_a_branch() {
        let github = parse(&answer()["data"]);
        assert_eq!(
            github.merged,
            [json!({"repo": "api", "slug": "acme/api", "url": "https://github.com/acme/api/pull/2", "number": 2, "branch": "me/eng-400-sso",
                "mergeSha": "abc123", "headSha": "def456", "mergedAt": "2026-10-20T09:00:00Z"})]
        );
    }

    #[test]
    fn the_query_scopes_my_prs_to_the_org_safely() {
        let q = query("acme\" evil", "2026-09-28");
        assert!(q.contains("author:@me org:acmeevil\""));
        assert!(q.contains("merged:>=2026-09-28"));
        assert!(!query("", "x").contains("org:"));
    }
}
