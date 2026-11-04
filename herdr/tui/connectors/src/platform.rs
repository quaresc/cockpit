//! Deploys and preview environments through the platform connector's mapping: the deployed apps, an app's latest deploy,
//! the previews, and the inbox's previews and deploys of my tickets.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::mapping::{pick, text};
use crate::tools::Tools;

/// A deployed app and the repo it builds from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct App {
    pub id: String,
    pub repo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
}

/// An app's deploy run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Deployment {
    pub workflow: String,
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// A preview environment, its apps and the platform's answer for it as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewEnv {
    pub id: String,
    pub expires_at: String,
    pub activated: Option<bool>,
    pub health: String,
    pub apps: Vec<PreviewApp>,
    pub raw: Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewApp {
    pub name: String,
    pub enabled: bool,
    pub branch: Option<String>,
}

/// The config's platform section with defaults for missing keys, ready to call: `previewDefaults` join the
/// `createPreview` arguments, and each `extendPreview.keep` field becomes a `{keep_<field>}` argument.
pub fn mapping(section: &Value) -> Value {
    let mut merged = json!({
        "enabled": false, "server": "", "previewPrefix": "", "previewDashboardUrl": "", "workflowUrl": "",
        "previewDefaults": {}, "tools": {}, "fields": {}
    });
    for (key, value) in section.as_object().into_iter().flatten() {
        merged[key] = value.clone();
    }
    let defaults = merged["previewDefaults"].as_object().cloned().unwrap_or_default();
    if let Some(args) = args_of(&mut merged, "createPreview") {
        args.extend(defaults);
    }
    let keep: Vec<String> = merged["tools"]["extendPreview"]["keep"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    if let Some(args) = args_of(&mut merged, "extendPreview") {
        for key in keep {
            args.insert(key.clone(), Value::String(format!("{{{}}}", keep_name(&key))));
        }
    }
    merged
}

fn args_of<'a>(section: &'a mut Value, verb: &str) -> Option<&'a mut Map<String, Value>> {
    let recipe = section["tools"].get_mut(verb)?.as_object_mut()?;
    let args = recipe.entry("args").or_insert_with(|| json!({}));
    if !args.is_object() {
        *args = json!({});
    }
    args.as_object_mut()
}

/// The placeholder that carries a kept field back to `extendPreview`.
fn keep_name(key: &str) -> String {
    let word: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    format!("keep_{word}")
}

fn field<'a>(section: &'a Value, name: &str) -> &'a str {
    section["fields"][name].as_str().unwrap_or("")
}

/// The list at a field's path, or none (an empty path is no list).
fn list<'a>(value: &'a Value, path: &str) -> Vec<&'a Value> {
    pick(value, path).and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default()
}

/// The deployed apps, with the repo each one builds from; none when the platform maps no `deployStatus`.
pub async fn deployed_apps(tools: &impl Tools, section: &Value) -> Result<Vec<App>, String> {
    if !tools.has("deployStatus") {
        return Ok(Vec::new());
    }
    let answer = tools.call("deployStatus", &json!({})).await?;
    let f = |name: &str| field(section, name);
    let skip: Vec<&str> = section["fields"]["skipKinds"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    Ok(list(&answer, f("apps"))
        .into_iter()
        .filter(|a| !skip.contains(&text(pick(a, f("appKind"))).unwrap_or_default().as_str()))
        .map(|a| App {
            id: text(pick(a, f("appId"))).unwrap_or_default(),
            repo: text(pick(a, f("appRepo"))).unwrap_or_default(),
            link: text(pick(a, f("appLink"))),
            tag: text(pick(a, f("appTag"))),
            sha: text(pick(a, f("appSha"))),
        })
        .collect())
}

/// The app a repo deploys as, its repo URL ending in `/<repo>` (case aside, `.git` dropped).
pub fn app_for<'a>(apps: &'a [App], repo: &str) -> Option<&'a App> {
    let suffix = format!("/{}", repo.to_lowercase());
    apps.iter().find(|a| {
        let url = a.repo.to_lowercase();
        url.strip_suffix(".git").unwrap_or(&url).ends_with(&suffix)
    })
}

/// An app's latest deploy, or none when there is none or the platform maps no `deployHistory`.
pub async fn latest_deploy(tools: &impl Tools, section: &Value, app_id: &str) -> Result<Option<Deployment>, String> {
    if !tools.has("deployHistory") {
        return Ok(None);
    }
    let answer = tools.call("deployHistory", &json!({"appId": app_id})).await?;
    let f = |name: &str| field(section, name);
    Ok(list(&answer, f("deployments")).first().map(|d| Deployment {
        workflow: text(pick(d, f("deployWorkflow"))).unwrap_or_default(),
        namespace: text(pick(d, f("deployNamespace"))).unwrap_or_default(),
        tag: text(pick(d, f("deployTag"))),
        branch: text(pick(d, f("deployBranch"))),
    }))
}

/// The preview environments `values` select (`{"login": …}` for mine); none when the platform maps no `listPreviews`.
pub async fn list_previews(tools: &impl Tools, section: &Value, values: &Value) -> Result<Vec<PreviewEnv>, String> {
    if !tools.has("listPreviews") {
        return Ok(Vec::new());
    }
    let answer = tools.call("listPreviews", values).await?;
    let f = |name: &str| field(section, name);
    Ok(list(&answer, f("previews"))
        .into_iter()
        .map(|raw| PreviewEnv {
            id: text(pick(raw, f("previewId"))).unwrap_or_default(),
            expires_at: text(pick(raw, f("previewExpires"))).unwrap_or_default(),
            activated: pick(raw, f("previewActive")).and_then(Value::as_bool),
            health: text(pick(raw, f("previewHealth"))).unwrap_or_default(),
            apps: list(raw, f("previewApps"))
                .into_iter()
                .map(|a| PreviewApp {
                    name: text(pick(a, f("appName"))).unwrap_or_default(),
                    enabled: pick(a, f("appEnabled")) != Some(&Value::Bool(false)),
                    branch: text(pick(a, f("appBranch"))),
                })
                .collect(),
            raw: raw.clone(),
        })
        .collect())
}

/// An app's first URL on a preview; none when the platform maps no `previewStatus` or the preview has no such app.
pub async fn preview_url(tools: &impl Tools, section: &Value, preview: &str, app: &str) -> Result<Option<String>, String> {
    if !tools.has("previewStatus") {
        return Ok(None);
    }
    let status = tools.call("previewStatus", &json!({"preview": preview})).await?;
    let f = |name: &str| field(section, name);
    let item = list(&status, f("statusApps")).into_iter().find(|a| text(pick(a, f("appName"))).as_deref() == Some(app));
    Ok(item.and_then(|item| text(pick(item, f("statusUrl")))))
}

/// Starts a preview of an app's branch, with the config's `previewDefaults`; its id, empty when the answer names none.
pub async fn create_preview(tools: &impl Tools, section: &Value, app: &str, branch: &str) -> Result<String, String> {
    let created = tools.call("createPreview", &json!({"app": app, "branch": branch})).await?;
    Ok(text(pick(&created, field(section, "createdId"))).unwrap_or_default())
}

/// Adds days to a preview (expired or not) and switches it back on. The fields `keep` names go back as they are, so a
/// tool that replaces the whole setup does not reset them.
pub async fn extend_preview(tools: &impl Tools, section: &Value, preview: &str, days: u32) -> Result<(), String> {
    let envs = list_previews(tools, section, &json!({})).await?;
    let env = envs.iter().find(|p| p.id == preview).ok_or_else(|| format!("preview {preview} not found"))?;
    let mut values = json!({"preview": preview, "days": days});
    for key in section["tools"]["extendPreview"]["keep"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if let Some(kept) = env.raw.get(key) {
            values[keep_name(key)] = kept.clone();
        }
    }
    tools.call("extendPreview", &values).await?;
    if env.activated != Some(true) && tools.has("startPreview") {
        tools.call("startPreview", &json!({"preview": preview})).await?;
    }
    Ok(())
}

/// Rebuilds an app of a preview from its branch.
pub async fn redeploy_preview(tools: &impl Tools, preview: &str, app: &str) -> Result<Value, String> {
    tools.call("redeployPreview", &json!({"preview": preview, "app": app})).await
}

pub async fn stop_preview(tools: &impl Tools, preview: &str) -> Result<Value, String> {
    tools.call("stopPreview", &json!({"preview": preview})).await
}

pub async fn start_preview(tools: &impl Tools, preview: &str) -> Result<Value, String> {
    tools.call("startPreview", &json!({"preview": preview})).await
}

/// What a dashboard link is, from its host, for the buttons and chips that open it.
pub fn dashboard_name(url: &str) -> &'static str {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or("");
    let host = rest.split('/').next().unwrap_or("");
    let labels: Vec<&str> = host.split('.').collect();
    // A label "argo" or "argocd" followed by a dot, or starting with "argo-" or "argocd-".
    let is_argo = labels.iter().enumerate().any(|(i, label)| {
        let after = label.strip_prefix("argocd").or_else(|| label.strip_prefix("argo"));
        after.is_some_and(|rest| rest.starts_with('-') || (rest.is_empty() && i + 1 < labels.len()))
    });
    match host {
        _ if is_argo => "ArgoCD",
        h if h.ends_with("vercel.com") => "Vercel",
        h if h.ends_with("netlify.app") || h.ends_with("netlify.com") => "Netlify",
        h if h.ends_with("render.com") => "Render",
        h if h.ends_with("railway.app") => "Railway",
        h if h.ends_with("fly.io") => "Fly",
        _ => "Dashboard",
    }
}

/// My previews, one per app running a branch other than main, in the inbox's `Preview` shape; none without a login.
pub async fn previews_for(tools: &impl Tools, section: &Value, login: &str) -> Result<Vec<Value>, String> {
    if login.is_empty() || !tools.has("listPreviews") {
        return Ok(Vec::new());
    }
    let envs = list_previews(tools, section, &json!({"login": login})).await?;
    let is_work = |branch: &Option<String>| branch.as_deref().is_some_and(|b| !b.is_empty() && b != "main" && b != "master");
    Ok(envs
        .iter()
        .flat_map(|env| {
            env.apps.iter().filter(|a| a.enabled && is_work(&a.branch)).map(|a| {
                let mut preview = json!({
                    "id": env.id, "app": a.name, "branch": a.branch.clone().unwrap_or_default(), "health": env.health, "expiresAt": env.expires_at
                });
                if let Some(activated) = env.activated {
                    preview["activated"] = Value::Bool(activated);
                }
                preview
            })
        })
        .collect())
}

/// For each ticket waiting for a deploy, whether each of its merged PRs is in the image its app runs, as the inbox's
/// `deploys`; `is_contained(slug, merge_sha, ref)` says whether a commit is in a ref (GitHub's compare).
pub async fn deploys_for(
    tools: &impl Tools,
    section: &Value,
    tickets: &[Value],
    merged: &[Value],
    mut is_contained: impl FnMut(&str, &str, &str) -> bool,
) -> Result<Map<String, Value>, String> {
    let is_waiting = |t: &&Value| t["statusType"] == "started" && t["status"].as_str().is_some_and(|s| s.to_lowercase().contains("deploy"));
    let waiting: Vec<&Value> = tickets.iter().filter(is_waiting).collect();
    if waiting.is_empty() || !tools.has("deployStatus") {
        return Ok(Map::new());
    }
    let apps = deployed_apps(tools, section).await?;
    let mut out = Map::new();
    for ticket in waiting {
        let id = ticket["id"].as_str().unwrap_or("");
        let mut prs = Vec::new();
        for pr in merged.iter().filter(|pr| has_ticket(pr["branch"].as_str().unwrap_or(""), id)) {
            let repo = pr["repo"].as_str().unwrap_or("");
            let reference = app_for(&apps, repo).and_then(|a| a.tag.clone().or_else(|| a.sha.clone()));
            let is_live = match reference.as_deref() {
                Some(r) if !r.is_empty() => is_contained(pr["slug"].as_str().unwrap_or(""), pr["mergeSha"].as_str().unwrap_or(""), r),
                _ => false,
            };
            prs.push(json!({"repo": repo, "number": pr["number"], "tag": reference, "isLive": is_live}));
        }
        out.insert(id.to_string(), Value::Array(prs));
    }
    Ok(out)
}

/// Whether a text names a ticket: its id, not inside a longer word or number.
fn has_ticket(text: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let (text, id) = (text.to_lowercase(), id.to_lowercase());
    text.match_indices(&id).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + id.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use crate::tools::Fake;

    fn section() -> Value {
        mapping(&json!({
            "enabled": true,
            "server": "backstage",
            "previewDefaults": {"monitoringEnabled": false},
            "tools": {
                "deployStatus": {"tool": "ship_get-deploy-status", "args": {"environment": "production"}},
                "deployHistory": {"tool": "ship_list-deployments", "args": {"app": "{appId}"}},
                "listPreviews": {"tool": "list-sandboxes", "args": {"owner": "{login}"}},
                "previewStatus": {"tool": "get-sandbox-status", "args": {"stage": "{preview}"}},
                "createPreview": {"tool": "create-sandbox", "args": {"product": "{app}", "branch": "{branch}"}},
                "extendPreview": {"tool": "update-sandbox", "args": {"stage": "{preview}", "ttlDays": "{days}"}, "keep": ["products", "alias"]},
                "startPreview": {"tool": "activate-sandbox", "args": {"stage": "{preview}"}}
            },
            "fields": {
                "apps": "apps", "appId": "id", "appRepo": "repository", "appLink": "argoUrl", "appTag": "tag", "appSha": "sha", "appKind": "kind",
                "skipKinds": ["cronjob"], "deployments": "deployments", "deployWorkflow": "workflow", "deployNamespace": "namespace",
                "deployTag": "tag", "previews": "sandboxes", "previewId": "stage", "previewExpires": "expiresAt", "previewActive": "activated",
                "previewHealth": "status.health", "previewApps": "products", "appName": "name", "appEnabled": "enabled", "appBranch": "branch",
                "statusApps": "products", "statusUrl": "urls.0.url", "createdId": "sandbox.stage"
            }
        }))
    }

    fn status() -> Value {
        json!({"apps": [
            {"id": "sourcing", "repository": "https://github.com/acme/sourcing.git", "argoUrl": "https://argocd.acme.io/applications/sourcing", "tag": "v1.4.0", "kind": "web"},
            {"id": "sourcing-cron", "repository": "https://github.com/acme/sourcing", "kind": "cronjob"},
            {"id": "front", "repository": "https://github.com/acme/Sourcing-Front", "sha": "abc123", "kind": "web"}
        ]})
    }

    fn sandboxes() -> Value {
        json!({"sandboxes": [
            {"stage": "preprod-sou-quotas", "expiresAt": "2026-11-10T00:00:00Z", "activated": false, "status": {"health": "Healthy"}, "alias": "quotas",
             "products": [{"name": "sourcing", "enabled": true, "branch": "cq/sou-2909-quotas"}, {"name": "front", "enabled": true, "branch": "main"},
                          {"name": "api", "enabled": false, "branch": "cq/off"}]},
            {"stage": "preprod-other", "expiresAt": "2026-11-12T00:00:00Z", "activated": true, "status": {"health": "Progressing"},
             "products": [{"name": "front", "branch": "cq/roi-1-x"}]}
        ]})
    }

    #[test]
    fn deployed_apps_skip_kinds_and_match_repos() {
        let fake = Fake::default().with("deployStatus", status());
        let apps = block_on(deployed_apps(&fake, &section())).unwrap();
        assert_eq!(apps.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["sourcing", "front"]);
        assert_eq!(apps[0].link.as_deref(), Some("https://argocd.acme.io/applications/sourcing"));
        assert_eq!(app_for(&apps, "sourcing").map(|a| a.id.as_str()), Some("sourcing"));
        assert_eq!(app_for(&apps, "sourcing-front").and_then(|a| a.sha.as_deref()), Some("abc123"));
        assert!(app_for(&apps, "front").is_none());
        assert!(block_on(deployed_apps(&Fake::default(), &section())).unwrap().is_empty(), "no deployStatus tool");
    }

    #[test]
    fn latest_deploy_is_the_first() {
        let fake =
            Fake::default().with("deployHistory", json!({"deployments": [{"workflow": "deploy-1", "namespace": "prod", "tag": "v2"}, {"workflow": "old"}]}));
        let latest = block_on(latest_deploy(&fake, &section(), "sourcing")).unwrap().unwrap();
        assert_eq!(latest, Deployment { workflow: "deploy-1".into(), namespace: "prod".into(), tag: Some("v2".into()), branch: None });
        assert_eq!(fake.calls.borrow()[0].1, json!({"appId": "sourcing"}));
    }

    #[test]
    fn my_previews_are_the_enabled_branches_of_my_login() {
        let fake = Fake::default().with("listPreviews", sandboxes());
        let previews = block_on(previews_for(&fake, &section(), "quaresc")).unwrap();
        assert_eq!(
            previews,
            [
                json!({"id": "preprod-sou-quotas", "app": "sourcing", "branch": "cq/sou-2909-quotas", "health": "Healthy", "expiresAt": "2026-11-10T00:00:00Z", "activated": false}),
                json!({"id": "preprod-other", "app": "front", "branch": "cq/roi-1-x", "health": "Progressing", "expiresAt": "2026-11-12T00:00:00Z", "activated": true}),
            ]
        );
        assert_eq!(fake.calls.borrow()[0], ("listPreviews".to_string(), json!({"login": "quaresc"})));
        assert!(block_on(previews_for(&fake, &section(), "")).unwrap().is_empty(), "no login");
    }

    #[test]
    fn preview_url_is_the_apps_first_url() {
        let answer = json!({"products": [
            {"name": "front", "urls": [{"url": "https://front.quotas.test"}]},
            {"name": "sourcing", "urls": [{"url": "https://sourcing.quotas.test"}, {"url": "x"}]}
        ]});
        let fake = Fake::default().with("previewStatus", answer);
        assert_eq!(block_on(preview_url(&fake, &section(), "preprod-sou-quotas", "sourcing")).unwrap().as_deref(), Some("https://sourcing.quotas.test"));
        assert_eq!(block_on(preview_url(&fake, &section(), "preprod-sou-quotas", "api")).unwrap(), None);
    }

    #[test]
    fn create_sends_the_defaults_and_extend_keeps_fields() {
        let s = section();
        let values = json!({"app": "sourcing", "branch": "cq/x"});
        let filled = crate::mapping::fill_args(&s["tools"]["createPreview"]["args"], values.as_object().unwrap()).unwrap();
        assert_eq!(filled, json!({"product": "sourcing", "branch": "cq/x", "monitoringEnabled": false}));
        let fake = Fake::default().with("createPreview", json!({"sandbox": {"stage": "preprod-cq-x"}}));
        assert_eq!(block_on(create_preview(&fake, &s, "sourcing", "cq/x")).unwrap(), "preprod-cq-x");

        let fake = Fake::default().with("listPreviews", sandboxes()).with("extendPreview", json!({})).with("startPreview", json!({}));
        block_on(extend_preview(&fake, &s, "preprod-sou-quotas", 7)).unwrap();
        {
            let calls = fake.calls.borrow();
            let (verb, values) = &calls[1];
            assert_eq!(verb, "extendPreview");
            let args = crate::mapping::fill_args(&s["tools"]["extendPreview"]["args"], values.as_object().unwrap()).unwrap();
            assert_eq!(args["stage"], "preprod-sou-quotas");
            assert_eq!(args["ttlDays"], 7);
            assert_eq!(args["alias"], "quotas");
            assert_eq!(args["products"].as_array().map(Vec::len), Some(3));
            assert_eq!(calls[2], ("startPreview".to_string(), json!({"preview": "preprod-sou-quotas"})), "an inactive preview is switched back on");
        }
        assert!(block_on(extend_preview(&fake, &s, "nope", 1)).is_err());
    }

    #[test]
    fn deploys_say_whether_each_pr_is_live() {
        let tickets = [
            json!({"id": "SOU-2909", "status": "To Deploy", "statusType": "started"}),
            json!({"id": "SOU-1", "status": "In Progress", "statusType": "started"}),
        ];
        let merged = [
            json!({"repo": "sourcing", "slug": "acme/sourcing", "number": 12, "branch": "cq/sou-2909-quotas", "mergeSha": "m1"}),
            json!({"repo": "sourcing-front", "slug": "acme/sourcing-front", "number": 7, "branch": "cq/SOU-2909-front", "mergeSha": "m2"}),
            json!({"repo": "sourcing", "slug": "acme/sourcing", "number": 13, "branch": "cq/sou-29090-other", "mergeSha": "m3"}),
        ];
        let fake = Fake::default().with("deployStatus", status());
        let mut asked = Vec::new();
        let deploys = block_on(deploys_for(&fake, &section(), &tickets, &merged, |slug, sha, r| {
            asked.push(format!("{slug} {sha} {r}"));
            sha == "m1"
        }))
        .unwrap();
        assert_eq!(
            Value::Object(deploys),
            json!({"SOU-2909": [
                {"repo": "sourcing", "number": 12, "tag": "v1.4.0", "isLive": true},
                {"repo": "sourcing-front", "number": 7, "tag": "abc123", "isLive": false}
            ]})
        );
        assert_eq!(asked, ["acme/sourcing m1 v1.4.0", "acme/sourcing-front m2 abc123"]);
    }

    #[test]
    fn dashboards_are_named_by_host() {
        assert_eq!(dashboard_name("https://argocd.preprod.acme.internal/applications/x"), "ArgoCD");
        assert_eq!(dashboard_name("https://argo-cd.acme.io"), "ArgoCD");
        assert_eq!(dashboard_name("https://ci.argo.acme.io"), "ArgoCD");
        assert_eq!(dashboard_name("https://argonaut.io"), "Dashboard");
        assert_eq!(dashboard_name("https://vercel.com/acme/app"), "Vercel");
        assert_eq!(dashboard_name("https://acme.netlify.app"), "Netlify");
        assert_eq!(dashboard_name("not a url"), "Dashboard");
    }
}
