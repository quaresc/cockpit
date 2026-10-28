"""Writes a fictional cockpit state under a fake HOME, for the README screenshots."""
import json, os, sys
from datetime import datetime, timedelta, timezone

home = sys.argv[1]
now = datetime.now(timezone.utc)
iso = lambda **d: (now - timedelta(**d)).isoformat().replace("+00:00", "Z")
fut = lambda **d: (now + timedelta(**d)).isoformat().replace("+00:00", "Z")
cache = os.path.join(home, ".cache", "cockpit"); conf = os.path.join(home, ".config", "cockpit")
os.makedirs(cache, exist_ok=True); os.makedirs(os.path.join(conf, "runbooks"), exist_ok=True)
os.makedirs(os.path.join(home, "code"), exist_ok=True)

def pr(repo, n, title, author, mins, review="REVIEW_REQUIRED", ci="SUCCESS", branch="", base="main", direct=False, draft=False, failed=()):
    return {"repo": repo, "number": n, "title": title, "url": f"https://github.com/acme/{repo}/pull/{n}", "author": author,
            "authorName": author.title(), "updatedAt": iso(minutes=mins), "isDraft": draft, "isDirect": direct,
            "review": review, "ci": ci, "branch": branch or f"{author}/{n}", "base": base, "failedChecks": list(failed)}

def t(i, title, status, stype, project=None, parent=None):
    return {"id": i, "title": title, "url": f"https://linear.app/acme/issue/{i}", "status": status, "statusType": stype,
            "parentId": parent, "project": project}

inbox = {
    "version": 1, "at": int(now.timestamp() * 1000),
    "direct": [pr("api", 2210, "Rate-limit the public search endpoint", "maya", 40, direct=True),
               pr("mobile", 512, "Offline mode for saved jobs", "jonas", 300, review="APPROVED", direct=True)],
    "team": [pr("web", 1840, "Move the settings page to the new form kit", "lin", 95),
             pr("billing", 377, "Prorate seat changes mid-cycle", "sam", 1500),
             pr("infra", 961, "Bump the Postgres minor version", "noor", 2900)],
    "botRepos": ["web", "api", "api"],
    "mine": [pr("api", 2198, "Daily quotas: store and enforce per-account limits", "me", 25, "APPROVED", "SUCCESS", "me/eng-412-daily-quotas"),
             pr("web", 1836, "Daily quotas: banner and disabled state", "me", 30, "REVIEW_REQUIRED", "PENDING", "me/eng-413-quota-ui", base="me/eng-412-daily-quotas"),
             pr("web", 1838, "Daily quotas: settings page copy", "me", 12, "REVIEW_REQUIRED", "FAILURE", "me/eng-414-quota-copy", failed=["lint"]),
             pr("billing", 375, "Export invoices as CSV", "me", 200, "CHANGES_REQUESTED", "SUCCESS", "me/eng-398-invoice-csv"),
             pr("infra", 955, "Preview environments: shorter default lifetime", "me", 4000, "REVIEW_REQUIRED", "SUCCESS", "me/infra-ttl")],
    "tickets": [t("ENG-410", "Daily quotas", "In Progress", "started", "Quotas"),
                t("ENG-412", "Store and enforce daily quotas", "To Deploy", "started", "Quotas", "ENG-410"),
                t("ENG-413", "Quota banner and disabled state", "In Review", "started", "Quotas", "ENG-410"),
                t("ENG-414", "Settings page copy for quotas", "In Progress", "started", "Quotas", "ENG-410"),
                t("ENG-398", "Invoice CSV export", "In Review", "started", "Billing exports"),
                t("ENG-405", "Flaky checkout e2e test", "Todo", "unstarted"),
                t("ENG-371", "Search ranking experiments", "Backlog", "backlog", "Search")],
    "errors": [],
    "merged": [{"repo": "api", "slug": "acme/api", "url": "https://github.com/acme/api/pull/2190", "number": 2190,
                "branch": "me/eng-412-quota-model", "mergeSha": "abc123", "mergedAt": iso(hours=20)}],
    "deploys": {"ENG-412": [{"repo": "api", "number": 2190, "tag": "v2026.10.22", "isLive": True}]},
    "previews": [{"id": "pv-quotas", "app": "api", "branch": "me/eng-412-quota-model", "health": "Healthy", "expiresAt": fut(hours=30), "activated": True},
                  {"id": "pv-billing", "app": "billing", "branch": "me/eng-398-invoice-csv", "health": "Healthy", "expiresAt": fut(hours=2), "activated": True}],
    "deployLinks": {"api": {"dashboard": "https://argocd.example.com/applications/api", "workflow": {"url": "https://argo.example.com/w/1", "label": "v2026.10.22"}}},
}
ci = {"at": int(now.timestamp() * 1000), "version": 3, "runs": [
    {"repo": "web", "branch": "me/eng-414-quota-copy", "runId": "r1", "createdAt": iso(minutes=14), "isRunning": False, "outcome": "failed",
     "workflowId": "w1", "done": 9, "total": 9, "failed": {"job": "lint", "jobId": "j1", "step": "eslint", "tests": [], "logs": "gh run view --job 1 --log-failed -R acme/web"}, "dockerTag": "skipped"},
    {"repo": "web", "branch": "me/eng-413-quota-ui", "runId": "r2", "createdAt": iso(minutes=4), "isRunning": False, "outcome": "succeeded",
     "workflowId": "w2", "done": 9, "total": 9, "dockerTag": "succeeded"},
    {"repo": "api", "branch": "me/eng-412-daily-quotas", "runId": "r3", "createdAt": iso(hours=1), "isRunning": False, "outcome": "succeeded",
     "workflowId": "w3", "done": 7, "total": 7, "dockerTag": "succeeded"}]}

local = datetime.now().astimezone()
def at(day, h, m):
    return (local.replace(hour=h, minute=m, second=0, microsecond=0) + timedelta(days=day)).isoformat()
meetings = {"at": int(now.timestamp() * 1000), "notified": [], "meetings": [
    {"id": f"m{i}", "title": title, "start": at(d, h, m), "end": at(d, h2, m2), "url": "https://calendar.example.com", "join": "https://meet.example.com/x",
     "response": resp, "attendees": n}
    for i, (title, d, h, m, h2, m2, resp, n) in enumerate([
        ("Team daily", 1, 9, 30, 9, 45, "accepted", 8), ("Quotas design review", 1, 11, 0, 12, 0, "accepted", 5),
        ("1:1 with Maya", 1, 14, 0, 14, 30, "accepted", 2), ("Billing sync", 1, 16, 0, 16, 30, "tentative", 6),
        ("Team daily", 2, 9, 30, 9, 45, "accepted", 8), ("Sprint review", 2, 15, 0, 16, 0, "needsAction", 14)])]}

os.makedirs(os.path.join(cache, "activity"), exist_ok=True)
with open(os.path.join(cache, "activity", "t7.jsonl"), "w") as f:
    for mins, kind, text in [(48, "edit", "src/settings/QuotaBanner.tsx"), (46, "edit", "src/settings/SearchForm.tsx"),
                             (44, "command", "yarn test src/settings"), (41, "edit", "src/settings/QuotaBanner.test.tsx"),
                             (38, "command", "yarn test src/settings"), (35, "turn", "The banner reads dailyLimit and usedToday and disables New search at the limit."),
                             (20, "command", "yarn lint --fix"), (18, "command", "git commit -m \"Quota banner and disabled state\""),
                             (17, "command", "git push"), (16, "turn", "Pushed to me/eng-413-quota-ui; CI is green and the image is built."),
                             (5, "turn", "Should the banner also show on the mobile layout? The frame only covers desktop.")]:
        f.write(json.dumps({"at": int((now - timedelta(minutes=mins)).timestamp() * 1000), "kind": kind, "text": text}) + "\n")
# The monitor: every app of production, those released today judged on their numbers before and after.
def release(i, tag, actor, mins, state, summary, mine=False, tickets=(), before=None, after=None, new=()):
    return {"id": i, "tag": tag, "actor": actor, "isMine": mine, "phase": "Succeeded", "startedAt": iso(minutes=mins + 4), "finishedAt": iso(minutes=mins),
            "tickets": [{"id": t, "url": f"https://linear.app/acme/issue/{t}"} for t in tickets], "prs": [],
            "verdict": {"state": state, "summary": summary, "before": before, "after": after, "newIssues": list(new), "until": iso(minutes=mins - 30)}}

def app(name, tag, mins, health="Healthy", watched=True, monitors=None, errors=(), p95=(), issues=(), releases=(), pinned=False):
    return {"id": f"{name}-production", "name": name, "service": name, "repo": name, "health": health, "tag": tag, "deployedAt": iso(minutes=mins), "behind": 0,
            "argoUrl": f"https://argocd.example.com/applications/{name}", "datadogUrl": f"https://app.datadoghq.eu/apm/entity/service%3A{name}",
            "watched": watched, "pinned": pinned, "monitors": monitors, "series": {"errors": list(errors), "p95": list(p95)} if watched else None,
            "issues": list(issues), "releases": list(releases)}

flat = [2, 3, 2, 2, 3, 2, 2, 2, 3, 2, 2, 3, 2, 2, 2, 3, 2, 2, 3, 2]
monitor = {"version": 1, "at": int(now.timestamp() * 1000), "env": "production", "errors": [], "apps": [
    app("api", "v2026.10.28.2", 50, monitors={"ok": 14, "warn": 0, "alert": 1, "nodata": 2, "alerting": [
            {"id": 1, "name": "[api] Search - API Availability", "status": "Alert", "url": "https://app.datadoghq.eu/monitors/1"}]},
        errors=flat[:16] + [3, 9, 14, 11], p95=[0.03] * 16 + [0.05, 0.12, 0.16, 0.15],
        issues=[{"id": "e1", "type": "QuotaExceeded", "message": "daily quota checked before the account loaded", "count": 140,
                 "firstSeen": iso(minutes=45), "firstVersion": "main-v2026.10.28.2", "url": "https://app.datadoghq.eu/error-tracking/issue/e1", "isNew": True}],
        releases=[release(31, "v2026.10.28.2", "me", 50, "moved", "errors 0.13% → 1.9% · p95 33 → 161 ms · 1 new error", True, ["ENG-412"],
                          {"errors": 2, "hits": 1512, "p95": 0.033}, {"errors": 31, "hits": 1630, "p95": 0.161}, ["e1"]),
                  release(30, "v2026.10.28.1", "jonas", 160, "ok", "errors 0.12% → 0.13% · p95 31 → 33 ms",
                          before={"errors": 2, "hits": 1650, "p95": 0.031}, after={"errors": 2, "hits": 1512, "p95": 0.033})], pinned=True),
    app("web", "v2026.10.28.3", 12, health="Progressing", monitors={"ok": 6, "warn": 0, "alert": 0, "nodata": 0, "alerting": []},
        errors=flat, p95=[0.21] * 18 + [0.22, 0.21],
        releases=[release(32, "v2026.10.28.3", "sam", 12, "watching", "watching", before={"errors": 1, "hits": 4210, "p95": 0.21})]),
    app("billing", "v2026.10.28", 300, monitors={"ok": 9, "warn": 0, "alert": 0, "nodata": 1, "alerting": []}, errors=flat, p95=[0.09] * 20,
        releases=[release(29, "v2026.10.28", "lin", 300, "ok", "errors 0.04% → 0.03% · p95 92 → 88 ms")]),
    app("mobile-api", "v2026.10.27", 1600, watched=False), app("search", "v2026.10.26", 3100, watched=False), app("infra-jobs", "v1.4.0", 9000, watched=False)]}
with open(os.path.join(cache, "monitor.json"), "w") as f: json.dump(monitor, f)

# Past Claude sessions, for the palette's search by words and by meaning.
def session(project, sid, title, mins, lines):
    folder = os.path.join(home, ".claude", "projects", project); os.makedirs(folder, exist_ok=True)
    with open(os.path.join(folder, f"{sid}.jsonl"), "w") as f:
        f.write(json.dumps({"type": "summary", "summary": title}) + "\n")
        for n, (role, text) in enumerate(lines):
            f.write(json.dumps({"type": role, "cwd": os.path.join(home, "code", project.split("-")[-1]), "sessionId": sid, "timestamp": iso(minutes=mins - n),
                                "message": {"role": role, "content": [{"type": "text", "text": text}]}}) + "\n")
session("-code-api", "s-quotas", "Daily quotas for ENG-412", 1500, [
    ("user", "Store and enforce the daily quotas for ENG-412: count exports and searches per account per day"),
    ("assistant", "The quota model lives in Quotas::Daily; the 429 answer carries the quota payload so the front can show the banner."),
    ("user", "Exports were counted twice when the CSV job retried"),
    ("assistant", "Fixed: the export counter increments once per job id, and the retry reuses it. The test covers the retry.")])
session("-code-web", "s-dark", "Dark mode tokens", 4300, [
    ("user", "Add dark mode tokens to the settings page"),
    ("assistant", "The tokens are in tokens.css; the settings page reads them and the contrast passes on both themes.")])
session("-code-billing", "s-csv", "Invoice CSV export", 7000, [
    ("user", "Export invoices as CSV for ENG-398"),
    ("assistant", "The export streams rows by month; amounts are written in cents so spreadsheets don't round them.")])

with open(os.path.join(cache, "inbox.json"), "w") as f: json.dump(inbox, f)
with open(os.path.join(cache, "ci.json"), "w") as f: json.dump(ci, f)
with open(os.path.join(cache, "meetings.json"), "w") as f: json.dump(meetings, f)
with open(os.path.join(conf, "muted.json"), "w") as f: json.dump(["infra"], f)
with open(os.path.join(conf, "config.json"), "w") as f:
    json.dump({"reposDir": "~/code", "github": {"enabled": True, "org": "acme"}, "tickets": {"enabled": True, "name": "Linear", "keys": ["ENG"]}, "ci": {"name": "GitHub", "imageJob": "docker"},
               "spaces": ["inbox", "terminal", "actions", "to-do", "runbook", "meetings", "monitor"],
               "platform": {"enabled": True, "previewPrefix": "pv-", "previewDashboardUrl": "https://argocd.example.com/applications/{preview}", "tools": {"createPreview": {"tool": "previews_create"}}}}, f)
with open(os.path.join(conf, "todo.md"), "w") as f:
    f.write("# To-do\n\n- [ ] Answer Maya on the search rate limit ENG-405\n- [ ] Pair with Jonas on offline mode\n"
            "- [ ] Write the quotas announcement https://notion.so/acme/quotas\n- [x] Review the billing proration PR (done 2026-10-22)\n")
with open(os.path.join(conf, "runbooks", "eng-398.md"), "w") as f:
    f.write("# ENG-398 · Invoice CSV export\nupdated: %s\n\n| # | Step | Where | Status | Command |\n|---|---|---|---|---|\n"
            "| 1 | Merge CSV export | billing#375 | ❌ changes requested | |\n| 2 | Deploy billing | prod | ⬜ to do | |\n" % local.strftime("%Y-%m-%d %H:%M"))

import time; time.sleep(1.1)
with open(os.path.join(conf, "runbooks", "eng-410.md"), "w") as f:
    f.write("""# ENG-410 · Daily quotas
updated: %s

| # | Step | Where | Status | Command |
|---|---|---|---|---|
| 1 | Merge quota model | api#2190 | ✅ merged yesterday | |
| 2 | Deploy quota model | prod | 🚀 live v2026.10.22 | |
| 3 | Backfill quotas for existing accounts | prod | ⬜ to do | `bin/rails quotas:backfill` |
| 4 | Merge enforcement | api#2198 | ⏳ approved, CI green | |
| 5 | Merge banner | web#1836 | 🟡 CI running | |
| 6 | Merge settings copy | web#1838 | ❌ lint failing | |
| 7 | Turn the quotas flag on | prod | ⬜ to do | |
| 8 | Watch rejections | logs | ⬜ to do | `service:api "quota exceeded"` |

## Notes
- Deploy api before web: the banner reads the new quota fields.
- Backfill before the flag, or every account starts at zero.
""" % local.strftime("%Y-%m-%d %H:%M"))
