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
    "at": int(now.timestamp() * 1000),
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
    "sandboxes": [{"stage": "preview-quotas", "product": "web", "branch": "me/eng-413-quota-ui", "health": "Healthy", "expiresAt": fut(hours=2)}],
    "deployLinks": {"api": {"argo": "https://argocd.example.com/applications/api", "workflow": {"url": "https://argo.example.com/w/1", "label": "v2026.10.22"}}},
}
ci = {"at": int(now.timestamp() * 1000), "version": 2, "runs": [
    {"repo": "web", "branch": "me/eng-414-quota-copy", "runId": "r1", "createdAt": iso(minutes=14), "isRunning": False, "outcome": "failed",
     "workflowId": "w1", "done": 9, "total": 9, "failed": {"job": "lint", "jobId": "j1", "step": "eslint", "tests": []}, "dockerTag": "skipped"},
    {"repo": "web", "branch": "me/eng-413-quota-ui", "runId": "r2", "createdAt": iso(minutes=4), "isRunning": True, "outcome": "",
     "workflowId": "w2", "done": 5, "total": 9},
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

with open(os.path.join(cache, "inbox.json"), "w") as f: json.dump(inbox, f)
with open(os.path.join(cache, "ci.json"), "w") as f: json.dump(ci, f)
with open(os.path.join(cache, "meetings.json"), "w") as f: json.dump(meetings, f)
with open(os.path.join(conf, "muted.json"), "w") as f: json.dump(["infra"], f)
with open(os.path.join(conf, "config.json"), "w") as f:
    json.dump({"reposDir": "~/code", "github": {"enabled": True, "org": "acme"}, "linear": {"enabled": True, "teams": ["ENG"]},
               "backstage": {"enabled": True, "stagePrefix": "preview-", "sandboxArgoUrl": "https://argocd.example.com/applications/{stage}"}}, f)
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
