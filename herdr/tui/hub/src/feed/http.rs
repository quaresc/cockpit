//! JSON over HTTPS for the feed, each API's base URL overridable by env (tests serve a fake one), and failures told
//! apart: a rate limit with when to try again, anything else with a short message. Tokens only travel in headers.

use std::sync::LazyLock;
use std::time::Duration;

use serde_json::{Value, json};

use super::now_secs;

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// Rate limited: no try before this time, in seconds since the epoch.
    Limited(u64),
    Other(String),
}

pub type Fetched<T = Value> = Result<T, Failure>;

/// An API's base URL: the env variable's when set, else the real one; never with a trailing slash.
pub fn base(var: &str, default: &str) -> String {
    std::env::var(var).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string()).trim_end_matches('/').to_string()
}

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent("cockpit-hub")
        .build()
        .into()
});

pub fn get(url: &str, headers: &[(&str, &str)]) -> Fetched {
    let mut request = AGENT.get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    finish(request.call())
}

pub fn post(url: &str, headers: &[(&str, &str)], body: &Value) -> Fetched {
    let mut request = AGENT.post(url).header("Content-Type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    finish(request.send(body.to_string()))
}

fn finish(result: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Fetched {
    let mut response = result.map_err(|err| Failure::Other(err.to_string()))?;
    let status = response.status().as_u16();
    let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let retry = Retry {
        after: header("retry-after"),
        remaining: header("x-ratelimit-remaining").or_else(|| header("x-ratelimit-requests-remaining")),
        reset: header("x-ratelimit-reset"),
        reset_ms: header("x-ratelimit-requests-reset"),
    };
    let text = response.body_mut().read_to_string().unwrap_or_default();
    let body = serde_json::from_str(&text).unwrap_or_else(|_| json!({"message": text.chars().take(160).collect::<String>()}));
    judge(status, &retry, body)
}

/// What an answer says about the rate limit, as its headers put it.
#[derive(Default)]
pub struct Retry {
    pub after: Option<String>,
    pub remaining: Option<String>,
    /// GitHub's, in seconds since the epoch.
    pub reset: Option<String>,
    /// Linear's, in milliseconds since the epoch.
    pub reset_ms: Option<String>,
}

impl Retry {
    /// When to try again: Retry-After first, else the reset of an exhausted quota, else in a minute.
    fn until(&self) -> u64 {
        let number = |v: &Option<String>| v.as_deref().and_then(|v| v.trim().parse::<u64>().ok());
        if let Some(seconds) = number(&self.after) {
            return now_secs() + seconds.max(1);
        }
        number(&self.reset).or_else(|| number(&self.reset_ms).map(|ms| ms / 1000)).filter(|at| *at > now_secs()).unwrap_or_else(|| now_secs() + 60)
    }

    fn is_exhausted(&self) -> bool {
        self.remaining.as_deref().map(str::trim) == Some("0")
    }
}

/// The answer's body, or why not: a 429, a 403 with an exhausted quota or a GraphQL rate-limit error read as a limit.
pub fn judge(status: u16, retry: &Retry, body: Value) -> Fetched {
    let errors = body["errors"].as_array().cloned().unwrap_or_default();
    let is_limit_error =
        errors.iter().any(|e| e["type"] == "RATE_LIMITED" || e["extensions"]["code"] == "RATELIMITED" || e["extensions"]["type"] == "ratelimited");
    if status == 429 || (status == 403 && (retry.is_exhausted() || retry.after.is_some())) || is_limit_error {
        return Err(Failure::Limited(retry.until()));
    }
    if status >= 400 {
        let reason = if status == 401 { "token refused".to_string() } else { message(&body) };
        return Err(Failure::Other(format!("HTTP {status}: {reason}")));
    }
    Ok(body)
}

/// The first message an error body carries, short.
pub fn message(body: &Value) -> String {
    let text = body["message"].as_str().or_else(|| body["errors"][0]["message"].as_str()).unwrap_or("no message");
    text.lines().next().unwrap_or_default().chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retry(remaining: &str, reset: u64) -> Retry {
        Retry { remaining: Some(remaining.into()), reset: Some(reset.to_string()), ..Retry::default() }
    }

    #[test]
    fn a_rate_limit_waits_for_the_reset() {
        let reset = now_secs() + 600;
        assert_eq!(judge(403, &retry("0", reset), json!({"message": "API rate limit exceeded"})), Err(Failure::Limited(reset)));
        assert_eq!(judge(429, &Retry { after: Some("30".into()), ..Retry::default() }, json!({})), Err(Failure::Limited(now_secs() + 30)));
    }

    #[test]
    fn a_graphql_rate_limit_error_counts_too() {
        let reset = now_secs() + 120;
        let body = json!({"errors": [{"type": "RATE_LIMITED", "message": "API rate limit exceeded"}]});
        assert_eq!(judge(200, &retry("0", reset), body), Err(Failure::Limited(reset)));
        let linear = json!({"errors": [{"message": "Rate limit exceeded", "extensions": {"code": "RATELIMITED"}}]});
        assert!(matches!(judge(400, &Retry::default(), linear), Err(Failure::Limited(at)) if at >= now_secs() + 59));
    }

    #[test]
    fn other_failures_say_why_in_one_short_line() {
        assert_eq!(judge(401, &Retry::default(), json!({"message": "Bad credentials"})), Err(Failure::Other("HTTP 401: token refused".into())));
        assert_eq!(judge(403, &retry("4000", 0), json!({"message": "Resource protected\nby SAML"})), Err(Failure::Other("HTTP 403: Resource protected".into())));
        assert_eq!(judge(200, &Retry::default(), json!({"data": 1})), Ok(json!({"data": 1})));
    }
}
