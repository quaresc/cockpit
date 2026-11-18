"""Writes a fictional cockpit state under a fake HOME, for the README screenshots."""
import json, os, pwd, sys
from datetime import datetime, timedelta, timezone

home = sys.argv[1] if len(sys.argv) > 1 else sys.exit("usage: demo_data.py <fake home>")
# It overwrites cockpit's caches and config under that folder, so never in the account's own home, whatever HOME says.
if os.path.realpath(home) == os.path.realpath(pwd.getpwuid(os.getuid()).pw_dir):
    sys.exit(f"demo_data.py: {home} is your real home; give it a fake one, such as docs/screenshots/home")
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
            "argoUrl": f"https://argocd.example.com/applications/{name}", "datadogUrl": f"https://app.datadoghq.com/apm/entity/service%3A{name}",
            "watched": watched, "pinned": pinned, "monitors": monitors, "series": {"errors": list(errors), "p95": list(p95)} if watched else None,
            "issues": list(issues), "releases": list(releases)}

flat = [2, 3, 2, 2, 3, 2, 2, 2, 3, 2, 2, 3, 2, 2, 2, 3, 2, 2, 3, 2]
monitor = {"version": 1, "at": int(now.timestamp() * 1000), "env": "production", "errors": [], "apps": [
    app("api", "v2026.10.28.2", 50, monitors={"ok": 14, "warn": 0, "alert": 1, "nodata": 2, "alerting": [
            {"id": 1, "name": "[api] Search - API Availability", "status": "Alert", "url": "https://app.datadoghq.com/monitors/1"}]},
        errors=flat[:16] + [3, 9, 14, 11], p95=[0.03] * 16 + [0.05, 0.12, 0.16, 0.15],
        issues=[{"id": "e1", "type": "QuotaExceeded", "message": "daily quota checked before the account loaded", "count": 140,
                 "firstSeen": iso(minutes=45), "firstVersion": "main-v2026.10.28.2", "url": "https://app.datadoghq.com/error-tracking/issue/e1", "isNew": True}],
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

# Routines: their files, and the runs the hub recorded with what each said.
os.makedirs(os.path.join(conf, "routines"), exist_ok=True)
ms = lambda **d: int((now - timedelta(**d)).timestamp() * 1000)
for slug, head, body in [
    ("morning-sweep", "name: Morning sweep\nwhen: weekdays 09:00\nmodel: sonnet\ntools: Read, Bash(gh pr list:*)\nbudget: 0.50", "Sum up my open PRs: which wait for review, which fail CI, which can merge."),
    ("fix-lint", "name: Fix lint on my PRs\nwhen: on ci-failed lint\nmode: acceptEdits\ntools: Read, Edit, Bash(yarn lint:*), Bash(git:*)\nnotify: failed", "CI failed on lint: fix it on the branch and push."),
    ("notes-pull", "name: Pull my notes\nwhen: every 30m\nrun: shell\ncwd: ~/code\nnotify: never", "git -C ~/code/api fetch --quiet && echo fetched"),
    ("weekly-digest", "name: Weekly digest\nwhen: fri 16:00\nmodel: opus\npaused: true", "Write my week's digest from my merged PRs and tickets.")]:
    with open(os.path.join(conf, "routines", f"{slug}.md"), "w") as f: f.write(f"---\n{head}\n---\n\n{body}\n")
def run(mins, took, ok, why, summary, cost=None, session="", code=0):
    return {"id": str(ms(minutes=mins)), "started": ms(minutes=mins), "ended": ms(minutes=mins) + took * 1000, "ok": ok, "code": code, "why": why,
            "session": session, "cost": cost, "summary": summary}
runs = {
    "morning-sweep": [run(70, 72, True, "scheduled for 09:00", "3 PRs wait for review, CI green on all of them", 0.18, "s-sweep-1"),
                      dict(run(2950, 0, False, "scheduled for 09:00", "missed Fri 09:00: cockpit was not running then", code=None), missed=True),
                      run(1510, 58, True, "scheduled for 09:00", "2 PRs wait for review, billing#375 fails lint", 0.15, "s-sweep-0")],
    "fix-lint": [dict(run(130, 210, True, "🔴 CI failed: billing me/eng-398-invoice-csv · lint", "Fixed 2 lint errors in billing#375 and pushed", 0.31, "s-lint-0"),
                      worktree={"path": os.path.join(home, ".herdr/worktrees/billing/auto/fix-lint-1"), "branch": "auto/fix-lint-1", "repo": os.path.join(home, "code/billing"), "base": "0" * 40, "commits": 1, "dirty": False})],
    "notes-pull": [run(12, 2, True, "every 30 min", "fetched"), run(42, 2, False, "every 30 min", "fatal: unable to access github.com", code=128)]}
for slug, rs in runs.items():
    os.makedirs(os.path.join(cache, "routines", slug), exist_ok=True)
with open(os.path.join(cache, "routines", "morning-sweep", runs["morning-sweep"][0]["id"] + ".txt"), "w") as f:
    f.write("3 PRs wait for review, CI green on all of them\n\n## Waiting for review\n- api#412 Daily quotas · 2 days, no reviewer yet\n"
            "- web#413 Quota banner · Maya asked for changes yesterday, answered\n- billing#375 Invoice CSV export · approved by Lin, needs a second review\n\n"
            "## Can merge\n- api#409 Search rate limit · approved, CI green\n")
with open(os.path.join(cache, "routines.json"), "w") as f:
    json.dump({"version": 1, "at": ms(minutes=1), "cursor": ms(minutes=1), "inboxFrom": ms(minutes=180), "routines": [], "running": {}, "queued": [], "runs": runs, "marks": {
        slug: {"seen": ms(days=3), "fired": ms(minutes=70) if slug == "morning-sweep" else ms(minutes=12)} for slug in runs}}, f)

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

# The conversations the chat screen reads: the demo agents' own sessions, under the folders their cwd names.
def conversation(cwd, sid, turns):
    folder = os.path.join(home, ".claude", "projects", cwd.replace("/", "-"))
    os.makedirs(folder, exist_ok=True)
    with open(os.path.join(folder, f"{sid}.jsonl"), "w") as f:
        n = len(turns)
        for i, turn in enumerate(turns):
            role = turn["role"]
            line = {"type": role, "cwd": cwd, "sessionId": sid, "gitBranch": turn.get("branch", "me/eng-412-quotas"),
                    "timestamp": iso(minutes=(n - i) * 3), "message": {"role": role, "content": turn["content"]}}
            if role == "assistant":
                line["message"]["model"] = "claude-opus-4-5"
                line["message"]["usage"] = {"input_tokens": 900, "output_tokens": 300, "cache_read_input_tokens": 62000, "cache_creation_input_tokens": 0}
            f.write(json.dumps(line) + "\n")

def says(text): return [{"type": "text", "text": text}]
def calls(*pairs): return [{"type": "tool_use", "id": f"t{i}", "name": name, "input": args} for i, (name, args) in enumerate(pairs)]
def results(*ids): return [{"type": "tool_result", "tool_use_id": i, "is_error": False, "content": "ok"} for i in ids]

conversation("/Users/me/code/api", "demo-412", [
    {"role": "user", "content": says("Exports and searches need a daily quota per account. Count them, answer 429 over the limit, and keep the payload the front needs.")},
    {"role": "assistant", "content": says("I'll read the limiter and the export job first.")},
    {"role": "assistant", "content": calls(("Read", {"file_path": "/Users/me/code/api/lib/quotas/daily.ex"}),
                                           ("Grep", {"pattern": "Quotas.Daily"}))},
    {"role": "user", "content": results("t0", "t1")},
    {"role": "assistant", "content": says("There is a counter per account already, but it is reset on deploy because it lives in the process. It has to move to Redis, keyed by account and day.")},
    {"role": "assistant", "content": calls(("Edit", {"file_path": "/Users/me/code/api/lib/quotas/daily.ex"}),
                                           ("Edit", {"file_path": "/Users/me/code/api/lib/quotas/plug.ex"}),
                                           ("Edit", {"file_path": "/Users/me/code/api/test/quotas/daily_test.exs"}))},
    {"role": "user", "content": results("t0", "t1", "t2")},
    {"role": "assistant", "content": calls(("Bash", {"command": "mix test test/quotas"}),
                                           ("Bash", {"command": "mix credo --strict"}))},
    {"role": "user", "content": results("t0", "t1")},
    {"role": "assistant", "content": says(
        "Done. The counter is a Redis key per account and day, and it expires at midnight UTC so nothing has to clean it up.\n\n"
        "```elixir\ndef consume(account, kind) do\n  key = \"quota:#{account.id}:#{kind}:#{Date.utc_today()}\"\n  case Redix.command(:quotas, [\"INCR\", key]) do\n    {:ok, n} when n > limit(account, kind) -> {:error, :over_quota}\n    {:ok, n} -> {:ok, n}\n  end\nend\n```\n\n"
        "Over the limit the plug answers 429 with the payload the banner reads:\n\n"
        "| field | what it holds |\n| --- | --- |\n| `limit` | the account's daily limit |\n| `used` | what it has spent today |\n| `resets_at` | midnight UTC, as ISO 8601 |\n\n"
        "All 34 tests pass and credo is clean.")},
    {"role": "user", "content": says("Exports were counted twice when the CSV job retried.")},
    {"role": "assistant", "content": calls(("Grep", {"pattern": "consume(.*:export"}), ("Edit", {"file_path": "/Users/me/code/api/lib/billing/export_job.ex"}))},
    {"role": "user", "content": results("t0", "t1")},
    {"role": "assistant", "content": says("Fixed: the counter increments once per job id, and a retry reuses the one it already took. The test covers the retry.")},
])

conversation("/Users/me/code/web", "demo-413", [
    {"role": "user", "content": says("Show the quota banner when the API answers 429, with what resets and when.")},
    {"role": "assistant", "content": calls(("Read", {"file_path": "/Users/me/code/web/src/api/client.ts"}))},
    {"role": "user", "content": results("t0")},
    {"role": "assistant", "content": says("The client throws on 429 without keeping the body, so the banner has nothing to read. I'll keep the payload on the error.")},
])

# A turn the person stopped: Claude's transcript ends on its interrupt marker, which the chat draws as "Stopped by you".
conversation("/Users/me/code/billing", "demo-419", [
    {"role": "user", "content": says("Prorate plan changes made mid-cycle: charge the difference for the days left.")},
    {"role": "assistant", "content": says("I'll read how invoices are built, then add the proration.")},
    {"role": "assistant", "content": calls(("Read", {"file_path": "/Users/me/code/billing/lib/invoices/build.ex"}))},
    {"role": "user", "content": results("t0")},
    {"role": "assistant", "content": calls(("Edit", {"file_path": "/Users/me/code/billing/test/invoices/proration_test.exs"}))},
    {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t0", "is_error": True, "content": "The user doesn't want to proceed with this tool use."}]},
    {"role": "user", "content": says("[Request interrupted by user for tool use]")},
])

with open(os.path.join(cache, "inbox.json"), "w") as f: json.dump(inbox, f)
with open(os.path.join(cache, "ci.json"), "w") as f: json.dump(ci, f)
with open(os.path.join(cache, "meetings.json"), "w") as f: json.dump(meetings, f)
# The mail space: two messages that need me above the updates, and Maya's thread already read whole.
def mail(i, sender, name, subject, snippet, mins, unread=True, labels=()):
    return {"id": i, "threadId": i, "from": sender, "fromName": name, "subject": subject, "snippet": snippet, "date": iso(minutes=mins),
            "url": f"https://mail.example.com/#all/{i}", "unread": unread, "labels": ["INBOX", *(["UNREAD"] if unread else []), *labels]}
mails = [mail("t-maya", "maya@example.com", "Maya", "Search rate limit: which tiers?", "Before I merge api#2210, should the free tier get 30/min or 60/min?", 18, labels=["IMPORTANT"]),
         mail("t-priya", "priya@example.com", "Priya", "Quotas announcement draft", "Here is the draft for Thursday, can you check the numbers?", 95),
         mail("t-gh", "notifications@github.example.com", "GitHub", "[acme/web] Daily quotas: banner (#1836)", "Leo approved these changes.", 30, labels=["CATEGORY_UPDATES"]),
         mail("t-ci", "noreply@ci.example.com", "CI", "billing: main passed", "All 412 tests passed in 6m 12s.", 140, unread=False, labels=["CATEGORY_UPDATES"]),
         mail("t-news", "digest@news.example.com", "Weekly digest", "This week in platform", "Postgres 17, faster previews and more.", 1500, labels=["CATEGORY_PROMOTIONS"])]
with open(os.path.join(cache, "mail.json"), "w") as f: json.dump({"at": int(now.timestamp() * 1000), "by": "hub", "messages": mails, "notified": [m["id"] for m in mails]}, f)
os.makedirs(os.path.join(cache, "mail"), exist_ok=True)
with open(os.path.join(cache, "mail", "t-maya.json"), "w") as f:
    json.dump({"at": int(now.timestamp() * 1000), "id": "t-maya", "messages": [
        {"id": "m1", "from": "me@acme.example", "fromName": "me@acme.example", "date": iso(minutes=60), "html": "",
         "text": "Maya, the rate limit is ready for review. Ping me with questions."},
        {"id": "m2", "from": "maya@example.com", "fromName": "Maya", "date": iso(minutes=18), "text": "",
         "html": "<p>Hi,</p><p>Before I merge <a href=\"https://github.com/acme/api/pull/2210\">api#2210</a>, should the <b>free tier</b> get 30/min or 60/min?</p>"
                 "<ul><li>30/min matches the docs</li><li>60/min matches what Sam promised</li></ul><p>Thanks,<br>Maya</p>"}]}, f)
# What the sessions cost, per ticket: the tickets' cost reads these books.
os.makedirs(os.path.join(cache, "costs"), exist_ok=True)
for session, spent in {"demo-412": {"ENG-412": 1.84}, "demo-413": {"ENG-413": 0.92}, "demo-398": {"ENG-398": 2.31}, "demo-405": {"ENG-405": 1.12}}.items():
    with open(os.path.join(cache, "costs", f"{session}.json"), "w") as f: json.dump(spent, f)
with open(os.path.join(conf, "muted.json"), "w") as f: json.dump(["infra"], f)
with open(os.path.join(conf, "config.json"), "w") as f:
    json.dump({"reposDir": "~/code", "github": {"enabled": True, "org": "acme"}, "tickets": {"enabled": True, "name": "Linear", "keys": ["ENG"]}, "ci": {"name": "GitHub", "imageJob": "docker"},
               "spaces": ["inbox", "terminal", "actions", "to-do", "runbook", "meetings", "monitor", "routines", "mail"],
               "mail": {"enabled": True, "name": "Mail", "server": "mail", "tools": {"archive": {"tool": "archive_thread"}, "markRead": {"tool": "read_thread"}}},
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

# parity:g — routines with a precheck (one that skips, one that runs), a time zone, runs that say how they went, and one
# routine whose `when` does not read: what needs a look in Needs, on Home and in the inbox's Today.
for slug, head, body in [
    ("review-digest", "name: Review digest\nwhen: cron */15 8-18 * * 1-5\ntz: Europe/London\nmodel: haiku\ntools: Bash(gh pr list:*)\n"
     "precheck: test -s ~/code/review-queue.txt || { echo nothing waits for review; exit 1; }\nnotify: failed", "List the PRs waiting for my review, oldest first."),
    ("release-check", "name: Release check\nwhen: every 2h\nrun: shell\ncwd: ~/code\nprecheck: test -d ~/code\nnotify: failed",
     "echo \"api and web run the same release\""),
    ("standup", "name: Standup notes\nwhen: manual\nmodel: sonnet\ntools: Read\nnotify: failed", "Draft my standup from yesterday's merged PRs."),
    ("weekly-report", "name: Weekly report\nwhen: every monday 9\nmodel: haiku\ntools: Bash(gh search prs:*)", "Sum up what I merged this week.")]:
    with open(os.path.join(conf, "routines", f"{slug}.md"), "w") as f: f.write(f"---\n{head}\n---\n\n{body}\n")
with open(os.path.join(cache, "routines.json")) as f: routines_doc = json.load(f)
routines_doc["runs"].update({
    "review-digest": [dict(run(14, 0, True, "scheduled for 09:30", "nothing waits for review", code=1),
                           skipped="precheck", kind="skipped", reason="Skipped: its precheck found nothing to do (exit 1).", read=True,
                           times=6, since=ms(minutes=90)),
                      dict(run(1480, 31, True, "scheduled for 08:00", "2 PRs wait for you: api#412, web#413", 0.02, "s-digest-0"), checked=True)],
    "release-check": [dict(run(35, 1, True, "every 2 h", "api and web run the same release"), checked=True)],
    "standup": [dict(run(50, 3, False, "run by hand", "Claude Code is not logged in: sign in to it in a terminal, then run the routine again.", code=1),
                     kind="logged_out", reason="Claude Code is not logged in: sign in to it in a terminal, then run the routine again.")]})
for slug in ("review-digest", "release-check", "standup"):
    os.makedirs(os.path.join(cache, "routines", slug), exist_ok=True)
    routines_doc["marks"][slug] = {"seen": ms(days=3), "fired": ms(minutes=14 if slug == "review-digest" else 35)}
with open(os.path.join(cache, "routines", "review-digest", routines_doc["runs"]["review-digest"][0]["id"] + ".txt"), "w") as f:
    f.write("nothing waits for review\n")
with open(os.path.join(cache, "routines.json"), "w") as f: json.dump(routines_doc, f)

# parity:f — how fresh each source is, as the hub records it in the documents it fills (`hub.fresh`): every source read
# a minute ago and the calendar a little stale, or with COCKPIT_DEMO_SOURCES=failing Linear refusing its token, CI
# offline, mail rate-limited and the calendar two hours old. The intervals keep the ages steady through a session of shots.
failing = os.environ.get("COCKPIT_DEMO_SOURCES") == "failing"
def fresh(source, label, read_mins, every_s, error=None, next_mins=None, login=None):
    entry = {"source": source, "label": label, "at": ms(minutes=read_mins), "every": every_s * 1000,
             "next": ms(minutes=-(next_mins if next_mins is not None else 1)), "state": "failing" if error else "fresh", "login": login}
    entry["error"] = {"kind": error[0], "message": error[1], "since": ms(minutes=error[2])} if error else None
    return entry
def stamp(name, record, errors=()):
    path = os.path.join(cache, f"{name}.json")
    doc = json.load(open(path))
    doc.setdefault("hub", {})["fresh"] = record
    if errors:
        doc["hub"]["errors"] = list(errors)
        doc["errors"] = list(doc.get("errors", [])) + list(errors)
    json.dump(doc, open(path, "w"))
if failing:
    stamp("inbox", {
        "github": fresh("github", "GitHub", 1, 900, login="gh auth login"),
        "tickets": fresh("tickets", "Linear", 27, 60, ("auth", "HTTP 401: token refused", 25), login="cockpit connector login Linear"),
        "platform": fresh("platform", "Deploys", 1, 900, login="cockpit setup"),
    }, errors=["Linear: HTTP 401: token refused"])
    stamp("ci", {"ci": fresh("ci", "CI", 41, 30, ("network", "unreachable: io: Connection refused (os error 61)", 40), next_mins=2, login="gh auth login")})
    ci_doc = json.load(open(os.path.join(cache, "ci.json"))); ci_doc["error"] = "CI unreachable: io: Connection refused (os error 61)"
    json.dump(ci_doc, open(os.path.join(cache, "ci.json"), "w"))
    stamp("meetings", {"calendar": fresh("calendar", "Calendar", 125, 300, next_mins=-120, login="cockpit setup")})
    stamp("mail", {"mail": fresh("mail", "Mail", 6, 60, ("rate", "rate limited: read again once the limit resets", 4), next_mins=26, login="cockpit setup")})
    # Signing in again to Linear goes through a server of cockpit's own, on an address that answers nothing.
    config_path = os.path.join(conf, "config.json")
    config = json.load(open(config_path))
    config["tickets"]["server"] = "Linear"
    config.setdefault("servers", {})["Linear"] = {"url": "https://mcp.example.com/linear"}
    json.dump(config, open(config_path, "w"))
else:
    stamp("inbox", {
        "github": fresh("github", "GitHub", 1, 900, login="gh auth login"),
        "tickets": fresh("tickets", "Linear", 1, 900, login="cockpit setup"),
        "platform": fresh("platform", "Deploys", 1, 900, login="cockpit setup"),
    })
    stamp("ci", {"ci": fresh("ci", "CI", 1, 900, login="gh auth login")})
    # Due 20 minutes ago and not read since: stale once two intervals have gone by.
    stamp("meetings", {"calendar": fresh("calendar", "Calendar", 25, 300, next_mins=-20, login="cockpit setup")})
    stamp("mail", {"mail": fresh("mail", "Mail", 1, 900, login="cockpit setup")})
stamp("monitor", {"monitor": fresh("monitor", "Datadog", 1, 1800, login="cockpit setup")})
# parity:d — billing#375's two open review threads, which "address review" hands to its agent.
with open(os.path.join(cache, "inbox.json")) as f: inbox_doc = json.load(f)
for p in inbox_doc["mine"]:
    if p["repo"] == "billing" and p["number"] == 375: p["threads"] = 2
with open(os.path.join(cache, "inbox.json"), "w") as f: json.dump(inbox_doc, f)

# parity:d — every agent's limits and the week's tokens: Claude fresh from its status line, Codex stale from its rollout,
# the Claude account cockpit shows (address and organization only), and token use spread over the last 7 days.
with open(os.path.join(cache, "limits.json"), "w") as f:
    json.dump({"at": ms(minutes=2), "limits": [{"name": "5h", "percent": 64, "resetsAt": fut(minutes=95)},
                                               {"name": "7d", "percent": 38, "resetsAt": fut(minutes=5800)}]}, f)
with open(os.path.join(home, ".claude.json"), "w") as f:
    json.dump({"oauthAccount": {"emailAddress": "me@acme.example", "organizationName": "Acme"}}, f)
# One rollout only: a run of this script on another day would leave its own beside it, counted twice.
import shutil
shutil.rmtree(os.path.join(home, ".codex", "sessions"), ignore_errors=True)
rollouts = os.path.join(home, ".codex", "sessions", local.strftime("%Y"), local.strftime("%m"), local.strftime("%d"))
os.makedirs(rollouts, exist_ok=True)
with open(os.path.join(rollouts, "rollout-%s-demo-codex.jsonl" % local.strftime("%Y-%m-%dT%H-%M-%S")), "w") as f:
    f.write(json.dumps({"timestamp": iso(days=5), "type": "session_meta", "payload": {"id": "demo-codex", "cwd": os.path.join(home, "code", "web")}}) + "\n")
    f.write(json.dumps({"timestamp": iso(days=5), "type": "turn_context", "payload": {"model": "gpt-5-codex", "cwd": os.path.join(home, "code", "web")}}) + "\n")
    total = [0, 0, 0]
    for day, (fresh, cached, out) in zip([5, 4, 2, 0], [(180_000, 90_000, 21_000), (240_000, 150_000, 30_000), (90_000, 40_000, 12_000), (310_000, 220_000, 41_000)]):
        total = [total[0] + fresh + cached, total[1] + cached, total[2] + out]
        last = day == 0
        f.write(json.dumps({"timestamp": iso(days=day, minutes=190 if last else 0), "type": "event_msg", "payload": {"type": "token_count",
            "info": {"total_token_usage": {"input_tokens": total[0], "cached_input_tokens": total[1], "output_tokens": total[2], "total_tokens": total[0] + total[2]}},
            "rate_limits": {"limit_id": "codex", "primary": {"used_percent": 12.0, "window_minutes": 10080, "resets_at": int((now + timedelta(minutes=4300)).timestamp())},
                            "secondary": None, "credits": {"has_credits": False, "unlimited": False, "balance": "0"}, "plan_type": "plus"} if last else None}}) + "\n")
helper = os.path.join(home, ".claude", "projects", "-code-api", "s-quotas", "subagents")
os.makedirs(helper, exist_ok=True)
with open(os.path.join(helper, "agent-usage.jsonl"), "w") as f:
    for day, calls in enumerate([6, 9, 4, 12, 7, 3, 8]):
        for n in range(calls):
            f.write(json.dumps({"type": "assistant", "isSidechain": True, "requestId": f"req-{day}-{n}", "timestamp": iso(days=day, minutes=30 + n),
                                "message": {"id": f"msg-{day}-{n}", "role": "assistant", "model": "claude-opus-5-5", "content": [{"type": "text", "text": "Checked the quota tests."}],
                                            "usage": {"input_tokens": 1200, "output_tokens": 900, "cache_read_input_tokens": 48_000, "cache_creation_input_tokens": 6_000}}}) + "\n")

# parity:y5 — names: the tabs cockpit named itself (all but "flaky e2e", which the person typed), and a session Claude Code
# titled.
os.makedirs(os.path.join(cache, "names"), exist_ok=True)
with open(os.path.join(cache, "names", "given.json"), "w") as f:
    json.dump({"tb-412": "quotas", "tb-413": "banner", "tb-414": "copy", "tb-398": "csv", "tb-ttl": "preview ttl", "tb-410": "model",
               "tb-417": "export retries", "tb-419": "proration"}, f)
with open(os.path.join(home, ".claude", "projects", "-Users-me-code-web", "demo-413.jsonl"), "a") as f:
    f.write(json.dumps({"type": "ai-title", "aiTitle": "Quota banner on 429", "sessionId": "demo-413"}) + "\n")

# parity:y5 — next steps: the settings-copy agent's finished turn, and the suggestions the hub keeps for it.
copy_answer = ("Done. The settings page says what the daily limit counts, that it resets at midnight UTC, and how much is left today. "
               "Over the limit, exports and searches pause until the reset and the banner links here. Lint still fails on an unused "
               "import in the strings file.")
conversation("/Users/me/code/web-copy", "demo-414", [
    {"role": "user", "content": says("Write the copy for the quota settings page: what the daily limit counts, when it resets, and what happens over it.")},
    {"role": "assistant", "content": [{"type": "tool_use", "id": "t0", "name": "Read", "input": {"file_path": "/Users/me/code/web-copy/src/settings/quotas.tsx"}},
                                      {"type": "tool_use", "id": "t1", "name": "Edit", "input": {"file_path": "/Users/me/code/web-copy/src/i18n/en.json"}}]},
    {"role": "user", "content": results("t0", "t1")},
    {"role": "assistant", "content": says(copy_answer)},
])
def fnv(text):
    h = 0xcbf29ce484222325
    for b in text.encode():
        h = ((h ^ b) * 0x100000001b3) & 0xffffffffffffffff
    return f"{h:016x}"
os.makedirs(os.path.join(cache, "agents", "next"), exist_ok=True)
with open(os.path.join(cache, "agents", "next", "demo-414.json"), "w") as f:
    json.dump({"turn": fnv(copy_answer), "at": ms(minutes=8), "items": [
        {"label": "Fix the lint failure", "prompt": "Fix the lint failure on the unused import in the strings file, then run lint again."},
        {"label": "Run the settings page tests", "prompt": "Run the settings page tests and fix what the new copy broke."},
        {"label": "Update web#1838", "prompt": "Push the branch and add the new copy to the description of web#1838."}]}, f)
