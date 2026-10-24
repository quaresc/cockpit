#!/usr/bin/env python3
"""Full-width, mouse-driven inbox for the herdr inbox space.

Reads the caches the cockpit Claude Code mod writes (~/.cache/cockpit) and acts through the
herdr CLI: detail popups, Claude sessions in new spaces, mutes, and rerun / redeploy / release
requests the mod carries out. A row shows its buttons only while hovered or selected.
"""

import json
import os
import re
import shlex
import subprocess
import time
from datetime import datetime, timezone

import cockpit_config
import view
from view import BOLD, DIM, GREEN, HERDR, ORANGE, RED, Button, Line, asking, confirmed, fit, note, width

HOME = os.path.expanduser("~")
CONFIG = cockpit_config.load()
REPOS = CONFIG["reposDir"]
CACHE = os.path.join(HOME, ".cache", "cockpit")
INBOX = os.path.join(CACHE, "inbox.json")
REFRESH = os.path.join(CACHE, "refresh-request")
CI = os.path.join(CACHE, "ci.json")
BRIEF = os.path.join(CACHE, "brief.json")
REQUESTS = os.path.join(CACHE, "requests")
MUTED = os.path.join(HOME, ".config", "cockpit", "muted.json")


def dashboard_url(preview):
    return cockpit_config.fill(CONFIG["platform"]["previewDashboardUrl"], preview=preview)


OLD_REVIEW_DAYS = 60
STALE_PR_DAYS = 30
PREVIEW_WARN_HOURS = 3
RED_ALERT_HOURS = 2

# Nerd Font glyphs present in FiraCode Nerd Font 3.0.
ICON = {
    "inbox": "", "review": "", "pr": "", "linear": "", "github": "",
    "circleci": "", "docker": "", "preview": "", "rocket": "", "fix": "",
    "rerun": "", "redeploy": "", "open": "", "mute": "", "plan": "",
    "continue": "", "done": "", "sure?": "", "released": "", "refresh": "", "alert": "", "this": "", "new": "", "cancel": "", "extend": "", "ArgoCD": "󰠳", "Dashboard": "\uf0e4", "deploy": "",
    "stop": "", "today": "", "calendar": "", "dismiss": "",
    "ok": "", "fail": "", "running": "", "approved": "", "changes": "",
    "waiting": "", "fold": "", "unfold": "", "user": "", "team": "",
    "work": "", "backlog": "", "progress": "",
}

# Column widths, shared by every row so the table lines up: glyphs, reference, title (flex),
# notes, age, then the buttons a hovered row shows.
LEAD, REF, NOTES, AGE, ACTIONS = 4, 28, 16, 6, 46


# ---------------------------------------------------------------------------
# Text helpers


def age_days(iso):
    return (datetime.now(timezone.utc) - datetime.fromisoformat(iso.replace("Z", "+00:00"))).total_seconds() / 86400


def ago(iso):
    days = age_days(iso)
    if days < 1 / 24:
        return f"{int(days * 1440)}m"
    if days < 2:
        return f"{int(days * 24)}h"
    return f"{int(days)}d"


def is_failing(pr):
    return pr.get("ci") in ("FAILURE", "ERROR")


def needs_me(pr):
    return is_failing(pr) or pr.get("review") == "CHANGES_REQUESTED"


def ci_glyph(pr):
    """CI: green check circle, red cross circle, orange hourglass while running, dot when none."""
    if is_failing(pr):
        return ICON["fail"], RED
    if pr.get("ci") == "SUCCESS":
        return ICON["ok"], GREEN
    return (ICON["running"], ORANGE) if pr.get("ci") else ("·", DIM)


def review_glyph(pr):
    """Review: check when approved, speech bubble when changes are requested, clock while waiting."""
    review = pr.get("review")
    if review == "APPROVED":
        return ICON["approved"], GREEN
    return (ICON["changes"], RED) if review == "CHANGES_REQUESTED" else (ICON["waiting"], DIM)


def first_name(pr):
    name = (pr.get("authorName") or "").strip()
    return name.split()[0] if name else pr["author"]


def short_status(ticket):
    return re.sub(r"\s*\(.*\)", "", ticket["status"]).lower()


def is_deploying(ticket):
    return ticket["statusType"] == "started" and bool(re.search("deploy", ticket["status"], re.I))



# ---------------------------------------------------------------------------
# Actions


def run(argv):
    return subprocess.run(argv, capture_output=True, text=True, timeout=15)


def open_url(state, url):
    cockpit_config.open_url(url)
    note(state, f"opened {url}")


def details(state, pr):
    result = run([HERDR, "plugin", "pane", "open", "--plugin", "cockpit", "--entrypoint", "peek",
                  "--placement", "popup", "--env", f"PEEK_URL={pr['url']}"])
    if result.returncode != 0:
        open_url(state, pr["url"])


def start_space(state, cwd, label, prompt):
    created = run([HERDR, "workspace", "create", "--cwd", cwd, "--label", label, "--focus"])
    try:
        pane = json.loads(created.stdout)["result"]["root_pane"]["pane_id"]
    except (ValueError, KeyError, TypeError):
        note(state, f"herdr workspace create failed: {created.stderr.strip()}")
        return
    run([HERDR, "pane", "run", pane, "claude " + shlex.quote(prompt)])
    note(state, f'started "{label}" in a new space')


def repo_dir(repo):
    path = os.path.join(REPOS, repo)
    return path if os.path.isdir(path) else REPOS


def review(state, pr):
    target = str(pr["number"]) if os.path.isdir(os.path.join(REPOS, pr["repo"])) else pr["url"]
    start_space(state, repo_dir(pr["repo"]), f"review {pr['repo']}#{pr['number']}", f"/code-review {target}")


def fix_ci(state, pr, ci_run, where):
    circle = ""
    if ci_run and ci_run.get("failed"):
        failed = ci_run["failed"]
        step = f', step "{failed["step"]}"' if failed.get("step") else ""
        circle = (f" The failing CircleCI job is {failed['job']} (job id {failed['jobId']}{step}): use the CircleCI MCP "
                  "(get_job, list_job_tests, then get_job_logs with a small tail).")
    checks = pr.get("failedChecks") or []
    github = f" Failing GitHub checks: {', '.join(checks)} (gh pr checks, gh run view --log-failed)." if checks else ""
    if where == "here":
        plan = f" The fix goes on this PR's branch, {pr.get('branch')}."
    else:
        plan = (f" Treat it as unrelated to this PR (flaky test or broken main): fix it on a new branch off an updated "
                f"local main and prepare a separate PR, leaving {pr.get('branch')} untouched.")
    prompt = f"CI is failing on {pr['url']}.{circle}{github}{plan} Explain the cause and propose a fix. Do not push."
    start_space(state, repo_dir(pr["repo"]), f"ci {pr['repo']}#{pr['number']}", prompt)
    state["choice"] = None


def ask_fix(state, pr):
    """fix ci first asks where the fix goes; the row's buttons become the choices."""
    state["choice"] = ("fix", pr["url"])
    state["btn"] = 0


def plan(state, ticket):
    prompt = (f"Pick up {ticket['id']} ({ticket['url']}): read the ticket, find the repos involved, "
              "and propose a plan before coding.")
    start_space(state, REPOS, ticket["id"].lower(), prompt)


def continue_ticket(state, ticket, prs):
    if prs:
        pr = prs[0]
        where = f" Its PR is {pr['url']} on branch {pr.get('branch')} in {pr['repo']}: check that branch out, read the PR's state, review comments and CI,"
        cwd = repo_dir(pr["repo"])
    else:
        where = f" Find its branch (named after {ticket['id'].lower()}) in the repo it touches, check it out,"
        cwd = REPOS
    prompt = f"Continue {ticket['id']} ({ticket['url']}).{where} and summarize where it stands before changing anything."
    start_space(state, cwd, ticket["id"].lower(), prompt)


def request(state, payload, row, working):
    name = f"{int(time.time() * 1000)}-{payload['type']}.json"
    view.write_json(os.path.join(REQUESTS, name), payload)
    state["pending"][row] = {"file": name, "working": working, "label": payload["label"], "at": time.time()}


# A request waits this long for a Claude session to take it, and this long in all for its outcome.
CLAIM_WAIT, ANSWER_WAIT = 120, 600


def settle(state):
    """Moves finished requests from pending to their outcome, read from the file the mod writes."""
    for row, job in list(state["pending"].items()):
        path = os.path.join(REQUESTS, "results", job["file"])
        outcome = view.load_json(path, None)
        if outcome is None:
            waited, is_claimed = time.time() - job["at"], os.path.exists(path + ".claimed")
            if waited > ANSWER_WAIT or (waited > CLAIM_WAIT and not is_claimed):
                message = "the session that took it never answered" if is_claimed else "no Claude session picked it up"
                state["outcome"][row] = {"ok": False, "message": message, "at": time.time()}
                del state["pending"][row]
            continue
        os.remove(path)
        del state["pending"][row]
        message = f"refused: {outcome['message']}" if outcome.get("state") == "refused" else outcome["message"]
        state["outcome"][row] = {**outcome, "message": message, "at": time.time()}
        note(state, f"{job['label']}: {message}")
    for row, outcome in list(state["outcome"].items()):
        if time.time() - outcome["at"] > 10:
            del state["outcome"][row]


def rerun(state, pr, ci_run):
    if confirmed(state, f"rerun-{ci_run['workflowId']}"):
        request(state, {"type": "rerun", "workflowId": ci_run["workflowId"], "label": f"rerun {pr['repo']}#{pr['number']}"},
                pr["url"], "rerunning failed jobs…")


def redeploy(state, box, row):
    if confirmed(state, f"redeploy-{box['id']}-{box['app']}"):
        request(state, {"type": "redeploy", "preview": box["id"], "app": box["app"],
                        "label": f"redeploy {box['app']} on {box['id']}"}, row, "redeploying…")


def create_preview(state, pr):
    key = f"preview-{pr['repo']}-{pr.get('branch')}"
    if confirmed(state, key):
        request(state, {"type": "create-preview", "app": pr["repo"], "branch": pr.get("branch"),
                        "label": f"preview for {pr['repo']} {pr.get('branch')}"}, pr["url"], "creating…")


def stop_preview(state, preview):
    if confirmed(state, f"stop-{preview}"):
        request(state, {"type": "stop-preview", "preview": preview, "label": f"stop {preview}"}, f"preview-{preview}", "stopping…")


def extend(state, preview, days=2):
    if confirmed(state, f"extend-{preview}"):
        request(state, {"type": "extend", "preview": preview, "days": days, "label": f"extend {preview}"}, f"preview-{preview}", "extending…")


def mark_done(state, ticket):
    if confirmed(state, f"done-{ticket['id']}"):
        request(state, {"type": "release", "ticket": ticket["id"], "label": f"{ticket['id']}"}, ticket["id"], "moving to Done…")


def set_muted(state, repo, is_muted):
    muted = set(view.load_json(MUTED, []))
    muted = muted | {repo} if is_muted else muted - {repo}
    os.makedirs(os.path.dirname(MUTED), exist_ok=True)
    with open(MUTED, "w", encoding="utf-8") as handle:
        json.dump(sorted(muted), handle)
    note(state, f"{'muted' if is_muted else 'unmuted'} {repo}")


def dismiss_brief(state):
    """Hides today's brief; tomorrow's shows again."""
    brief = view.load_json(BRIEF, {})
    brief["dismissed"] = True
    view.write_json(BRIEF, brief)
    note(state, "brief dismissed until tomorrow")


def request_refresh(state):
    os.makedirs(CACHE, exist_ok=True)
    with open(REFRESH, "w", encoding="utf-8") as handle:
        handle.write(str(time.time()))
    note(state, "refresh requested, it lands within a minute")


def toggle(state, group):
    state["open"] ^= {group}


# ---------------------------------------------------------------------------
# Layout: lines with clickable spans, and buttons drawn only on the active line


def build(state, cols):
    data = view.load_json(INBOX, None)
    muted = set(view.load_json(MUTED, []))
    ci = view.load_json(CI, {"runs": []})
    runs = {(r["repo"], r["branch"]): r for r in ci.get("runs", [])}
    lines = []

    def line():
        lines.append(Line())
        return lines[-1]

    def heading(icon, title, extra=""):
        line().add(f"{ICON[icon]}  ", ORANGE).add(title, BOLD + ORANGE).add(f"   {extra}", DIM)

    def sub(label, style=DIM):
        line().add(" " * LEAD).add(label, style)

    def fold(group, count, label, rows):
        if count == 0:
            return
        is_open = group in state["open"]
        item = line().add(" " * LEAD).add(f"{ICON['unfold'] if is_open else ICON['fold']} {count} {label}", DIM, lambda: toggle(state, group))
        item.primary = item.spans[0][2]
        if is_open:
            rows()

    def row_status(item, row):
        """A request in flight from this row, or its outcome for a few seconds."""
        job = state["pending"].get(row)
        if job:
            item.status = (f"{ICON['refresh']} {job['working']}", ORANGE)
        elif row in state["outcome"]:
            outcome = state["outcome"][row]
            item.status = (f"{ICON['approved'] if outcome['ok'] else ICON['fail']} {outcome['message']}", GREEN if outcome["ok"] else RED)

    def detail(cont, text, style=DIM):
        line().add(" " * (LEAD + REF)).add(cont, DIM).add(text, style)

    title_room = lambda used: max(10, cols - used - NOTES - AGE - 1)

    if data is None:
        line().add("No data yet: the cockpit mod in a Claude session fills ~/.cache/cockpit/inbox.json.", DIM)
        return lines

    # Header, then what needs attention now.
    updated = datetime.fromtimestamp(data["at"] / 1000).strftime("%H:%M")
    head = line().add(f"{ICON['inbox']}  ", ORANGE).add("Inbox", BOLD + ORANGE).add(f"   updated {updated}", DIM)
    head.buttons = [Button("refresh", lambda: request_refresh(state), "R", False)]
    head.primary = lambda: request_refresh(state)
    for error in data.get("errors", []) + ([ci["error"]] if ci.get("error") else []):
        line().add(error, RED)

    # The morning brief the mod writes once a day, until dismissed.
    brief = view.load_json(BRIEF, {})
    if brief.get("date") == datetime.now().strftime("%Y-%m-%d") and not brief.get("dismissed") and brief.get("lines"):
        line()
        written = datetime.fromtimestamp(brief["at"] / 1000).strftime("%H:%M")
        top = line().add(f"{ICON['today']}  ", ORANGE).add("Today", BOLD + ORANGE).add(f"   morning brief, {written}", DIM)
        top.buttons = [Button("dismiss", lambda: dismiss_brief(state), "x", False)]
        top.primary = lambda: dismiss_brief(state)
        for entry in brief["lines"]:
            style = {"bad": RED, "warn": ORANGE, "good": GREEN}.get(entry.get("tone"), "")
            item = line().add(" " * LEAD).add(f"{ICON.get(entry['icon'], '·')}  ", style or DIM).add(entry["text"], style)
            if entry.get("url"):
                item.buttons = [Button("open", lambda u=entry["url"]: open_url(state, u), "o", True)]
                item.primary = lambda u=entry["url"]: open_url(state, u)

    alerts = []  # (text, style, url, preview to extend)
    for preview in sorted({b["id"] for b in data.get("previews") or []}):
        box = next(b for b in data["previews"] if b["id"] == preview)
        hours = -age_days(box["expiresAt"]) * 24
        url = dashboard_url(preview)
        if hours <= 0:
            alerts.append((f"preview {preview} has expired", RED, url, preview))
        elif hours <= PREVIEW_WARN_HOURS:
            left = f"{int(hours * 60)} min" if hours < 1 else f"{hours:.1f} h"
            alerts.append((f"preview {preview} expires in {left}", ORANGE, url, preview))
        if box["health"] not in ("Healthy", "Progressing") and hours > 0:
            alerts.append((f"preview {preview} is {box['health'].lower()}", ORANGE, url, None))
    for ci_run in ci.get("runs", []):
        if not ci_run["isRunning"] and ci_run["outcome"] == "failed" and age_days(ci_run["createdAt"]) * 24 <= RED_ALERT_HOURS:
            job = f" · {ci_run['failed']['job']}" if ci_run.get("failed") else ""
            alerts.append((f"ci failed on {ci_run['repo']} {ci_run['branch']}{job}", RED, None, None))
    # A running preview whose branch has merged: it can stop, keeping its setup.
    merged = {(m["repo"], m["branch"]) for m in data.get("merged") or []}
    for box in data.get("previews") or []:
        if box.get("activated") and (box["app"], box["branch"]) in merged:
            alerts.append((f"preview {box['id']} runs merged {box['app']} {box['branch']}", ORANGE, dashboard_url(box["id"]), ("stop", box["id"])))
    if alerts:
        line()
        for text, style, url, target in alerts:
            item = line().add(f" {ICON['alert']}  ", style).add(text, style)
            if isinstance(target, tuple):
                item.buttons.append(Button(asking(state, f"stop-{target[1]}", "stop"), lambda p=target[1]: stop_preview(state, p), "a", True))
                row_status(item, f"preview-{target[1]}")
            elif target:
                item.buttons.append(Button(asking(state, f"extend-{target}", "extend 2 days"), lambda p=target: extend(state, p), "a", True))
                row_status(item, f"preview-{target}")
            if url:
                item.buttons.append(Button("open", lambda url=url: open_url(state, url), "o", False))
                item.primary = lambda url=url: open_url(state, url)
    line()

    # Review requests: direct ones first, then team ones, muted repos and old ones folded away.
    def review_row(pr):
        item = line()
        item.add(fit(ICON["user"] if pr["isDirect"] else ICON["team"], LEAD), ORANGE if pr["isDirect"] else DIM)
        item.add(fit(f"{pr['repo']}#{pr['number']}", REF), DIM, lambda: details(state, pr))
        item.add(fit(pr["title"], title_room(LEAD + REF + 2)) + "  ", "", lambda: details(state, pr))
        item.add(fit(first_name(pr), NOTES), DIM).add(fit(ago(pr["updatedAt"]), AGE), DIM)
        item.buttons = [Button("review", lambda: review(state, pr), "a", True), Button("open", lambda: open_url(state, pr["url"]), "o", False)]
        if not pr["isDirect"]:
            item.buttons.append(Button("mute", lambda: set_muted(state, pr["repo"], True), "m", False))
        item.primary = lambda: details(state, pr)

    shown = [pr for pr in data["team"] if pr["repo"] not in muted]
    direct = [pr for pr in data["direct"] if age_days(pr["updatedAt"]) <= OLD_REVIEW_DAYS]
    team = [pr for pr in shown if age_days(pr["updatedAt"]) <= OLD_REVIEW_DAYS]
    old = [pr for pr in data["direct"] + shown if age_days(pr["updatedAt"]) > OLD_REVIEW_DAYS]
    bots = [r for r in data.get("botRepos", []) if r not in muted]
    hidden = len(data["team"]) - len(shown) + len(data.get("botRepos", [])) - len(bots)
    heading("review", "Review", f"{len(direct)} for you · {len(team)} team · {len(bots)} bots" + (f" · {hidden} muted" if hidden else ""))
    for pr in direct:
        review_row(pr)
    for pr in team if "team" in state["open"] else team[:5]:
        review_row(pr)
    if len(team) > 5:
        label = f"{ICON['unfold']} show less" if "team" in state["open"] else f"{ICON['fold']} {len(team) - 5} more team requests"
        item = line().add(" " * LEAD).add(label, DIM, lambda: toggle(state, "team"))
        item.primary = item.spans[-1][2]
    fold("old", len(old), f"older than {OLD_REVIEW_DAYS} days", lambda: [review_row(pr) for pr in old])
    if muted:
        item = line().add(" " * LEAD).add(f"{ICON['mute']} muted  ", DIM)
        for repo in sorted(muted):
            item.add(f"{repo} ×", DIM, lambda repo=repo: set_muted(state, repo, False)).add("   ")
    line()

    # The PR forest: a PR based on another PR's branch hangs under it.
    mine = data["mine"]
    pr_key = lambda pr: (pr["repo"], pr.get("branch"))
    by_key = {pr_key(pr): pr for pr in mine}
    pr_children = {}
    for pr in mine:
        parent = (pr["repo"], pr.get("base"))
        if parent in by_key and parent != pr_key(pr):
            pr_children.setdefault(parent, []).append(pr)
    pr_parent = {pr_key(kid): parent for parent, kids in pr_children.items() for kid in kids}

    def pr_subtree(pr):
        out = [pr]
        for kid in pr_children.get(pr_key(pr), []):
            out += pr_subtree(kid)
        return out

    def pr_row(pr, prefix, cont):
        ci_run = runs.get((pr["repo"], pr.get("branch")))
        box = next((b for b in data.get("previews") or [] if b["app"] == pr["repo"] and b["branch"] == pr.get("branch")), None)
        image_ready = bool(ci_run and ci_run.get("dockerTag") == "succeeded")
        notes = []
        if ci_run and ci_run["isRunning"]:
            notes.append(f"{ICON['circleci']} {ci_run['done']}/{ci_run['total']}")
        if image_ready:
            notes.append(f"{ICON['docker']} image")
        if box:
            notes.append(f"{ICON['preview']} preview")
        ci_g, ci_s = ci_glyph(pr)
        rv_g, rv_s = review_glyph(pr)
        item = line()
        item.add(ci_g, ci_s).add(" ").add(rv_g, rv_s).add(" ")
        item.add(fit(f"{pr['repo']}#{pr['number']}", REF), DIM, lambda: details(state, pr))
        item.add(prefix, DIM)
        item.add(fit(pr["title"], title_room(LEAD + REF + width(prefix) + 2)) + "  ", DIM if pr["isDraft"] else "", lambda: details(state, pr))
        item.add(fit("  ".join(notes), NOTES), ORANGE).add(fit(ago(pr["updatedAt"]), AGE), DIM)
        if state["choice"] == ("fix", pr["url"]):
            item.buttons = [
                Button("this PR", lambda: fix_ci(state, pr, ci_run, "here"), "a", True),
                Button("new PR", lambda: fix_ci(state, pr, ci_run, "new"), "n", False),
                Button("cancel", lambda: state.update(choice=None, btn=-1), "c", False),
            ]
        else:
            if is_failing(pr):
                item.buttons.append(Button("fix ci", lambda: ask_fix(state, pr), "a", True))
            if ci_run and ci_run.get("outcome") == "failed" and ci_run.get("workflowId") and not ci_run["isRunning"]:
                item.buttons.append(Button(asking(state, f"rerun-{ci_run['workflowId']}", "rerun"), lambda: rerun(state, pr, ci_run), "u", False))
            if box and image_ready:
                item.buttons.append(Button(asking(state, f"redeploy-{box['id']}-{box['app']}", "redeploy"), lambda: redeploy(state, box, pr["url"]),
                                     "d", not item.buttons))
            # A green image with no preview running it yet: one click creates one with the configured defaults.
            if not box and image_ready and CONFIG["platform"]["enabled"] and "createPreview" in CONFIG["platform"]["tools"]:
                item.buttons.append(Button(asking(state, f"preview-{pr['repo']}-{pr.get('branch')}", "preview"), lambda: create_preview(state, pr),
                                     "s", not item.buttons))
            # The dashboard of the preview running this branch, else of the production app; then its latest prod deploy.
            links = (data.get("deployLinks") or {}).get(pr["repo"]) or {}
            dashboard = (dashboard_url(box["id"]) if box else None) or links.get("dashboard")
            if dashboard:
                item.buttons.append(Button(cockpit_config.dashboard_name(dashboard), lambda: open_url(state, dashboard), "A", False))
            if links.get("workflow"):
                item.buttons.append(Button("deploy", lambda: open_url(state, links["workflow"]["url"]), "W", False))
            item.buttons.append(Button("open", lambda: open_url(state, pr["url"]), "o", False))
        item.primary = lambda: details(state, pr)
        row_status(item, pr["url"])
        if ci_run and ci_run.get("failed") and not ci_run["isRunning"]:
            failed = ci_run["failed"]
            why = f"{len(failed['tests'])} failing: {', '.join(failed['tests'])}" if failed.get("tests") else failed.get("step", "")
            detail(cont, fit(f"{ICON['circleci']} {failed['job']} · {why}", title_room(LEAD + REF + width(cont))), DIM + RED)
        checks = [c for c in pr.get("failedChecks") or [] if not c.lower().startswith(("ci/circleci", "circleci"))]
        if checks:
            detail(cont, fit(f"{ICON['github']} {', '.join(checks)}", title_room(LEAD + REF + width(cont))), DIM + RED)

    def render_pr(pr, prefix="", cont=""):
        pr_row(pr, prefix, cont)
        kids = pr_children.get(pr_key(pr), [])
        for i, kid in enumerate(kids):
            last = i == len(kids) - 1
            render_pr(kid, cont + ("└─ " if last else "├─ "), cont + ("   " if last else "│  "))

    # Work: each ticket with its sub-issues and its PRs (and theirs, stacked) underneath.
    tickets = data["tickets"]
    ticket_ids = {t["id"] for t in tickets}
    t_children = {}
    for t in tickets:
        if t.get("parentId") in ticket_ids:
            t_children.setdefault(t["parentId"], []).append(t)
    t_roots = [t for t in tickets if t.get("parentId") not in ticket_ids]

    def linked(ticket):
        return [pr for pr in mine if ticket["id"].lower() in (pr.get("branch") or "").lower()]

    def linked_roots(ticket):
        """Linked PRs whose parent PR is not linked to the same ticket; their stacks follow them."""
        prs = linked(ticket)
        keys = {pr_key(pr) for pr in prs}
        return [pr for pr in prs if pr_parent.get(pr_key(pr)) not in keys]

    claimed = set()
    for t in tickets:
        for root in linked_roots(t):
            claimed |= {pr_key(pr) for pr in pr_subtree(root)}

    def ticket_row(ticket, prefix, cont):
        prs = linked(ticket)
        deploying = is_deploying(ticket)
        if deploying:
            glyph, style = ICON["rocket"], DIM
        elif ticket["statusType"] == "started":
            glyph, style = ICON["progress"], ORANGE
        else:
            glyph, style = ICON["backlog"], DIM
        item = line()
        item.add(fit(glyph, LEAD), style)
        item.add(fit(ticket["id"], 10), "", lambda: open_url(state, ticket["url"]))
        item.add(fit(short_status(ticket), REF - 10), DIM, lambda: open_url(state, ticket["url"]))
        item.add(prefix, DIM)
        item.add(fit(ticket["title"], title_room(LEAD + REF + width(prefix) + 2)) + "  ", "", lambda: open_url(state, ticket["url"]))
        item.add(" " * (NOTES + AGE))
        deployed = (data.get("deploys") or {}).get(ticket["id"]) or []
        if deploying:
            if deployed and all(d["isLive"] for d in deployed):
                item.buttons.append(Button(asking(state, f"done-{ticket['id']}", "done"), lambda: mark_done(state, ticket), "a", True))
        elif ticket["statusType"] == "started":
            item.buttons.append(Button("continue", lambda: continue_ticket(state, ticket, prs), "a", True))
        else:
            item.buttons.append(Button("plan", lambda: plan(state, ticket), "a", True))
        if deploying and deployed:
            links = (data.get("deployLinks") or {}).get(deployed[0]["repo"]) or {}
            if links.get("dashboard"):
                item.buttons.append(Button(cockpit_config.dashboard_name(links["dashboard"]), lambda: open_url(state, links["dashboard"]), "A", False))
            if links.get("workflow"):
                item.buttons.append(Button("deploy", lambda: open_url(state, links["workflow"]["url"]), "W", False))
        item.buttons.append(Button("open", lambda: open_url(state, ticket["url"]), "o", False))
        item.primary = lambda: open_url(state, ticket["url"])
        row_status(item, ticket["id"])
        for done in deployed:
            live = f"{ICON['rocket']} live in production" if done["isLive"] else f"{ICON['waiting']} not in production yet, prod runs {done['tag']}"
            detail(cont + ("│  " if t_children.get(ticket["id"]) or linked_roots(ticket) else "   "),
                   f"{done['repo']}#{done['number']}  {live}", GREEN if done["isLive"] else DIM)

    def render_ticket(ticket, prefix="", cont=""):
        ticket_row(ticket, prefix, cont)
        kids = [("ticket", t) for t in t_children.get(ticket["id"], [])] + [("pr", pr) for pr in linked_roots(ticket)]
        for i, (kind, item) in enumerate(kids):
            last = i == len(kids) - 1
            branch, more = cont + ("└─ " if last else "├─ "), cont + ("   " if last else "│  ")
            (render_ticket if kind == "ticket" else render_pr)(item, branch, more)

    STATUS_STEPS = ["progress", "review", "merge", "test", "deploy"]
    STATUS_LABELS = ["in progress", "in review", "to merge", "to test", "to deploy", "todo", "backlog", "triage", "other"]

    def status_order(ticket):
        """In progress, review, merge, test, deploy, then todo, backlog, triage."""
        name = ticket["status"].lower()
        if ticket["statusType"] == "started":
            return next((i for i, step in enumerate(STATUS_STEPS) if step in name), 0)
        return {"unstarted": 5, "backlog": 6, "triage": 7}.get(ticket["statusType"], 8)

    def tree_tickets(ticket):
        return [ticket] + [t for kid in t_children.get(ticket["id"], []) for t in tree_tickets(kid)]

    def tree_needs_me(ticket):
        return any(needs_me(pr) for t in tree_tickets(ticket) for root in linked_roots(t) for pr in pr_subtree(root))

    def tree_order(ticket):
        """A tree sorts by its most urgent ticket: one whose PR needs me, then the least advanced status."""
        return (0 if tree_needs_me(ticket) else 1, min(status_order(t) for t in tree_tickets(ticket)), ticket["id"])

    in_progress = sum(1 for t in tickets if t["statusType"] == "started" and not is_deploying(t))
    head = line().add(f"{ICON['work']}  ", ORANGE).add("Work", BOLD + ORANGE)
    head.add(f"   {len(tickets)} tickets · {in_progress} in progress · {len(claimed)} PRs", DIM)
    for mode in ("feature", "status"):
        is_on = state["work_mode"] == mode
        head.add("   ").add(f"by {mode}", (BOLD + ORANGE) if is_on else DIM, lambda mode=mode: state.update(work_mode=mode))

    if state["work_mode"] == "feature":
        features = {}
        for root in t_roots:
            features.setdefault(root.get("project") or "No project", []).append(root)
        ordered = sorted(features.items(), key=lambda kv: (kv[0] == "No project", min(tree_order(r) for r in kv[1])))
        for n, (name, roots) in enumerate(ordered):
            count = sum(len(tree_tickets(r)) for r in roots)
            if n:
                line()
            marker = ("● ", RED) if any(tree_needs_me(r) for r in roots) else ("  ", "")
            line().add(" " * (LEAD - 2)).add(*marker).add(name, BOLD).add(f"  {count}", DIM)
            for root in sorted(roots, key=tree_order):
                render_ticket(root)
    else:
        by_status = {}
        for root in t_roots:
            order = min(status_order(t) for t in tree_tickets(root))
            label = "needs you" if tree_needs_me(root) else STATUS_LABELS[min(order, len(STATUS_LABELS) - 1)]
            by_status.setdefault(label, []).append(root)
        for label, roots in sorted(by_status.items(), key=lambda kv: min(tree_order(r) for r in kv[1])):
            if label == "to deploy":
                fold("deploy", sum(len(tree_tickets(r)) for r in roots), "to deploy", lambda roots=roots: [render_ticket(r) for r in roots])
                continue
            sub(label, RED if label == "needs you" else DIM)
            for root in sorted(roots, key=tree_order):
                render_ticket(root)
    line()

    # PRs no assigned ticket claims, grouped by what they need.
    others = [pr for pr in mine if pr_key(pr) not in claimed and pr_parent.get(pr_key(pr)) not in claimed]
    roots = [pr for pr in others if pr_parent.get(pr_key(pr)) is None]

    def pr_rank(pr):
        tree = pr_subtree(pr)
        if all(age_days(p["updatedAt"]) > STALE_PR_DAYS for p in tree):
            return 3
        if any(needs_me(p) for p in tree):
            return 0
        return 2 if all(p.get("review") == "APPROVED" for p in tree) else 1

    pr_groups = {0: [], 1: [], 2: [], 3: []}
    for root in roots:
        pr_groups[pr_rank(root)].append(root)

    def pr_rows(group):
        for root in pr_groups[group]:
            render_pr(root)

    heading("pr", "Other PRs", f"{len(others)} without an assigned ticket")
    if pr_groups[0]:
        sub("needs you", RED)
        pr_rows(0)
    if pr_groups[1]:
        sub("waiting for review")
        pr_rows(1)
    fold("ready", sum(len(pr_subtree(r)) for r in pr_groups[2]), "ready to merge", lambda: pr_rows(2))
    fold("stale", sum(len(pr_subtree(r)) for r in pr_groups[3]), f"stale, no update in {STALE_PR_DAYS} days", lambda: pr_rows(3))

    line()
    if view.noted(state, 8):
        line().add(state["note"], ORANGE)
    else:
        line().add("hover a row for its buttons · click for details · j/k rows · h/l or tab buttons · enter press · a main action · o open · g group by · R refresh", DIM)
    return lines


# ---------------------------------------------------------------------------
# The loop


def on_key(state, data):
    """Moving off a row drops its "fix ci" choice; Esc cancels it; R refreshes, g switches how tickets group."""
    if data in ("j", "k", view.DOWN, view.UP, view.PAGE_DOWN, view.PAGE_UP):
        state["choice"] = None
        return False
    if data == "\033" and state["choice"]:
        state["choice"], state["btn"] = None, -1
    elif data == "R":
        request_refresh(state)
    elif data == "g":
        state["work_mode"] = "status" if state["work_mode"] == "feature" else "feature"
    else:
        return False
    return True


def stamp():
    return tuple(os.path.getmtime(p) if os.path.exists(p) else 0 for p in (INBOX, MUTED, CI, BRIEF))


def poll(state):
    """Requests in flight are checked every second; the caches changing, and every 30 s for the ages, redraw."""
    busy = bool(state["pending"] or state["outcome"] or state["confirm"])
    settle(state)
    seen = (stamp(), int(time.time() // 30))
    if seen == state["seen"]:
        return busy
    state["seen"] = seen
    return True


def main():
    state = view.new_state(open=set(), work_mode="feature", choice=None, pending={}, outcome={}, seen=None)
    view.App("inbox", state, build, ICON, poll=poll, on_key=on_key).run()


if __name__ == "__main__":
    main()