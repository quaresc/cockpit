//! The Herdr app's Home: what landed this week and how fast, the PRs ready to land and the ones to fix, what the agents
//! cost lately, and the agents a restart stopped. The mod lays the agents themselves out from the `agents` view.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use cockpit_core::{files, resume};
use serde_json::{Value, json};

pub fn dirs() -> Vec<PathBuf> {
    vec![resume::dir(), files::cache().join("costs")]
}

pub fn view(_args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let or_file = |name: &str| {
        let v = doc(name);
        if v.is_null() { files::load(&files::cache().join(format!("{name}.json"))).unwrap_or(Value::Null) } else { v }
    };
    let mut model = model(&or_file("inbox"), &or_file("ci"), Local::now());
    model["lost"] = json!(resume::lost());
    model["spend"] = spend(&files::cache().join("costs"));
    model
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

fn local(at: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(at).ok().map(|d| d.with_timezone(&Local))
}

/// The ticket key a branch names (`me/eng-412-quotas` → `ENG-412`).
fn ticket_of(branch: &str) -> Option<String> {
    let tail = branch.rsplit('/').next().unwrap_or(branch);
    let mut parts = tail.split('-');
    let (key, num) = (parts.next()?, parts.next()?);
    (key.chars().all(|c| c.is_ascii_alphabetic()) && !key.is_empty() && !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("{}-{num}", key.to_uppercase()))
}

fn pr_card(pr: &Value, ci: &Value) -> Value {
    let branch = s(pr, "branch");
    let run = list(&ci["runs"]).iter().filter(|r| s(r, "repo") == s(pr, "repo") && s(r, "branch") == branch).max_by_key(|r| s(r, "createdAt").to_string());
    json!({
        "key": s(pr, "url"),
        "repo": s(pr, "repo"),
        "number": pr["number"],
        "title": s(pr, "title"),
        "url": s(pr, "url"),
        "branch": branch,
        "base": s(pr, "base"),
        "ticket": ticket_of(branch),
        "review": pr["review"],
        "ci": pr["ci"],
        "failed": pr["failedChecks"],
        "draft": pr["isDraft"] == true,
        "updated": s(pr, "updatedAt"),
        "running": run.is_some_and(|r| r["isRunning"] == true),
        "checks": run.map(|r| json!({"done": r["done"], "total": r["total"]})).unwrap_or(Value::Null),
    })
}

/// The model at `now`: my PRs ready to land (approved and green) and to fix (failing or changes asked), merges per day
/// over the week, and this week against the last.
pub fn model(inbox: &Value, ci: &Value, now: DateTime<Local>) -> Value {
    let mine = list(&inbox["mine"]);
    let ready: Vec<Value> = mine
        .iter()
        .filter(|p| p["review"] == "APPROVED" && p["ci"] == "SUCCESS" && p["isDraft"] != true)
        .map(|p| pr_card(p, ci))
        .collect();
    let fixing: Vec<Value> = mine
        .iter()
        .filter(|p| p["isDraft"] != true && (p["ci"] == "FAILURE" || p["ci"] == "ERROR" || p["review"] == "CHANGES_REQUESTED"))
        .map(|p| pr_card(p, ci))
        .collect();
    let waiting: Vec<Value> = mine
        .iter()
        .filter(|p| p["isDraft"] != true && p["review"] != "APPROVED" && p["review"] != "CHANGES_REQUESTED" && p["ci"] != "FAILURE" && p["ci"] != "ERROR")
        .map(|p| pr_card(p, ci))
        .collect();

    let today = now.date_naive();
    let merged: Vec<(NaiveDate, Option<f64>, &Value)> = list(&inbox["merged"])
        .iter()
        .filter_map(|m| {
            let at = local(s(m, "mergedAt"))?;
            let hours = local(s(m, "createdAt")).map(|c| (at - c).num_minutes() as f64 / 60.0);
            Some((at.date_naive(), hours, m))
        })
        .collect();
    let on = |d: NaiveDate| merged.iter().filter(|(day, _, _)| *day == d).count();
    let days: Vec<Value> = (0..7)
        .rev()
        .map(|ago| {
            let d = today - Duration::days(ago);
            json!({"day": d.format("%a").to_string(), "date": d.to_string(), "count": on(d)})
        })
        .collect();
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let in_week = |from: NaiveDate| merged.iter().filter(|(d, _, _)| *d >= from && *d < from + Duration::days(7)).collect::<Vec<_>>();
    let this_week = in_week(monday);
    let last_week = in_week(monday - Duration::days(7));
    let last_to_date = last_week.iter().filter(|(d, _, _)| *d <= today - Duration::days(7)).count();
    let recent: Vec<Value> = (0..14)
        .rev()
        .map(|ago| {
            let d = today - Duration::days(ago);
            json!({"day": d.format("%a").to_string(), "date": d.format("%b %-d").to_string(), "count": on(d), "today": ago == 0, "weekend": d.weekday().number_from_monday() > 5})
        })
        .collect();
    let by_day: Vec<Value> = (0..7)
        .map(|i| {
            let d = monday + Duration::days(i);
            json!({"day": d.format("%a").to_string(), "count": on(d), "future": d > today})
        })
        .collect();
    let weeks: Vec<Value> = (0..4)
        .rev()
        .map(|ago| {
            let from = monday - Duration::days(7 * ago);
            json!({"label": if ago == 0 { "now".to_string() } else { from.format("%b %-d").to_string() }, "count": in_week(from).len()})
        })
        .collect();
    let mut hours: Vec<f64> = merged.iter().filter(|(d, _, _)| *d > today - Duration::days(30)).filter_map(|(_, h, _)| *h).collect();
    hours.sort_by(f64::total_cmp);
    let median = (!hours.is_empty()).then(|| if hours.len() % 2 == 1 { hours[hours.len() / 2] } else { (hours[hours.len() / 2 - 1] + hours[hours.len() / 2]) / 2.0 });
    let repos = {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for (_, _, m) in &this_week {
            match counts.iter_mut().find(|(r, _)| r == s(m, "repo")) {
                Some((_, n)) => *n += 1,
                None => counts.push((s(m, "repo").to_string(), 1)),
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        counts.into_iter().map(|(repo, count)| json!({"repo": repo, "count": count})).collect::<Vec<_>>()
    };
    let landed_today: Vec<Value> = merged
        .iter()
        .filter(|(d, _, _)| *d == today)
        .map(|(_, _, m)| json!({"repo": s(m, "repo"), "number": m["number"], "url": s(m, "url"), "branch": s(m, "branch")}))
        .collect();

    json!({
        "ready": ready,
        "fixing": fixing,
        "waiting": waiting,
        "merged": list(&inbox["merged"]).iter().map(|m| json!({"repo": s(m, "repo"), "branch": s(m, "branch"), "url": s(m, "url"), "number": m["number"], "at": s(m, "mergedAt")})).collect::<Vec<_>>(),
        "landed": {"today": on(today), "days": days, "prs": landed_today},
        "week": {
            "merged": this_week.len(),
            "last": last_week.len(),
            "last_to_date": last_to_date,
            "recent": recent,
            "by_day": by_day,
            "weeks": weeks,
            "median_hours": median,
            "repos": repos,
            "open": mine.iter().filter(|p| p["isDraft"] != true).count(),
            "reviews": list(&inbox["direct"]).len(),
        },
    })
}

/// What the Claude sessions cost over the last 7 days and today, from the cost books they wrote then (a book counts
/// for the day it was last written).
fn spend(dir: &std::path::Path) -> Value {
    let now = SystemTime::now();
    let midnight = Local.from_local_datetime(&Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap_or_default()).single();
    let midnight = midnight.map(|m| UNIX_EPOCH + std::time::Duration::from_secs(m.timestamp().max(0) as u64)).unwrap_or(now);
    let (mut week, mut today) = (0.0, 0.0);
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else { continue };
        if now.duration_since(modified).is_ok_and(|d| d.as_secs() > 7 * 86_400) {
            continue;
        }
        let total: f64 = files::load(&entry.path()).and_then(|b| b.as_object().map(|o| o.values().filter_map(Value::as_f64).sum())).unwrap_or(0.0);
        week += total;
        if modified >= midnight {
            today += total;
        }
    }
    json!({"week": (week * 100.0).round() / 100.0, "today": (today * 100.0).round() / 100.0})
}

/// Home's actions: `resume [handle]` (every lost agent without one), `dismiss [handle]`.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let handle = args.first().and_then(Value::as_str).unwrap_or_default();
    Some(match action {
        "resume" if handle.is_empty() => resume::resume_all(),
        "resume" => resume::resume(handle),
        "dismiss" => {
            resume::dismiss(handle);
            Ok(String::new())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(n: u64, review: &str, ci: &str, branch: &str) -> Value {
        json!({"repo": "api", "number": n, "title": format!("PR {n}"), "url": format!("u{n}"), "branch": branch, "review": review, "ci": ci, "isDraft": false})
    }

    #[test]
    fn ready_fixing_and_a_week_of_merges() {
        let now = Local.with_ymd_and_hms(2026, 10, 28, 15, 0, 0).unwrap();
        let at = |d: u32, h: u32| Local.with_ymd_and_hms(2026, 10, d, h, 0, 0).unwrap().to_rfc3339();
        let inbox = json!({
            "mine": [pr(1, "APPROVED", "SUCCESS", "me/eng-412-quotas"), pr(2, "REVIEW_REQUIRED", "FAILURE", "me/x"), pr(3, "REVIEW_REQUIRED", "PENDING", "me/y")],
            "merged": [
                {"repo": "api", "url": "m1", "number": 9, "branch": "a", "mergedAt": at(28, 10), "createdAt": at(28, 6)},
                {"repo": "web", "url": "m2", "number": 8, "branch": "b", "mergedAt": at(27, 10), "createdAt": at(26, 10)},
                {"repo": "api", "url": "m3", "number": 7, "branch": "c", "mergedAt": at(21, 10)},
            ],
            "direct": [{}],
        });
        let ci = json!({"runs": [{"repo": "api", "branch": "me/eng-412-quotas", "createdAt": "z", "isRunning": false, "done": 7, "total": 7}]});
        let m = model(&inbox, &ci, now);
        assert_eq!(m["ready"][0]["ticket"], "ENG-412");
        assert_eq!(m["ready"][0]["checks"]["total"], 7);
        assert_eq!(m["fixing"][0]["number"], 2);
        assert_eq!(m["waiting"][0]["number"], 3);
        assert_eq!(m["landed"]["today"], 1);
        assert_eq!(m["landed"]["days"].as_array().unwrap().len(), 7);
        assert_eq!(m["week"]["merged"], 2);
        assert_eq!(m["week"]["last"], 1);
        assert_eq!(m["week"]["last_to_date"], 1);
        assert_eq!(m["week"]["recent"].as_array().unwrap().len(), 14);
        assert_eq!(m["week"]["recent"][13]["count"], 1);
        assert_eq!(m["week"]["by_day"][2]["count"], 1);
        assert_eq!(m["week"]["by_day"][6]["future"], true);
        assert_eq!(m["week"]["median_hours"], 14.0);
        assert_eq!(m["week"]["repos"][0]["repo"], "api");
        assert_eq!(m["week"]["reviews"], 1);
    }
}
