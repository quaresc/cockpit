#!/usr/bin/env python3
"""Full-width, mouse-driven inbox for the herdr inbox space.

Reads the caches the cockpit Claude Code mod writes (~/.cache/cockpit) and acts through the
herdr CLI: detail popups, Claude sessions in new spaces, mutes, and rerun / redeploy / release
requests the mod carries out. A row shows its buttons only while hovered or selected.
"""

import json
import os
import re
import select
import shlex
import subprocess
import sys
import termios
import time
import tty
import unicodedata
from datetime import datetime, timezone

import cockpit_config

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
HERDR = os.environ.get("HERDR_BIN_PATH", "herdr")


def sandbox_url(stage):
    return cockpit_config.fill(CONFIG["backstage"]["sandboxArgoUrl"], stage=stage)


OLD_REVIEW_DAYS = 60
STALE_PR_DAYS = 30
SANDBOX_WARN_HOURS = 3
RED_ALERT_HOURS = 2

# Vesper colours: warm orange accent, teal for good, red for broken, the rest dim.
DIM, BOLD, RESET, INV = "\033[2m", "\033[1m", "\033[0m", "\033[7m"
GREEN, RED, ORANGE = "\033[38;2;153;255;228m", "\033[38;2;255;128;128m", "\033[38;2;255;199;153m"
HOVER = "\033[48;2;30;30;34m"
PILL_MAIN = ("\033[38;2;255;199;153m", "\033[48;2;255;199;153m\033[38;2;16;16;16m")
PILL = ("\033[38;2;44;44;48m", "\033[48;2;44;44;48m\033[38;2;210;210;210m")
PILL_FOCUS = ("\033[38;2;245;245;245m", "\033[48;2;245;245;245m\033[38;2;16;16;16m\033[1m")

# Nerd Font glyphs present in FiraCode Nerd Font 3.0.
ICON = {
    "inbox": "", "review": "", "pr": "", "linear": "", "github": "",
    "circleci": "", "docker": "", "sandbox": "", "rocket": "", "fix": "",
    "rerun": "", "redeploy": "", "open": "", "mute": "", "plan": "",
    "continue": "", "done": "", "sure?": "", "released": "", "refresh": "", "alert": "", "this": "", "new": "", "cancel": "", "extend": "", "argo": "󰠳", "deploy": "",
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


def width(text):
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 0 if unicodedata.combining(c) else 1 for c in text)


def fit(text, cells):
    """Cuts or pads text to exactly `cells` terminal cells."""
    if cells <= 0:
        return ""
    if width(text) <= cells:
        return text + " " * (cells - width(text))
    out = ""
    for c in text:
        if width(out + c) > cells - 1:
            break
        out += c
    return out + "…" + " " * (cells - 1 - width(out))


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


def load(path, default):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError):
        return default


# ---------------------------------------------------------------------------
# Actions


def run(argv):
    return subprocess.run(argv, capture_output=True, text=True, timeout=15)


def note(state, text):
    state["note"] = text
    state["note_at"] = time.time()


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


def confirmed(state, key):
    """True on the second click within 6 seconds; the first one turns the button into "sure?"."""
    pending = state.get("confirm")
    if pending and pending[0] == key and time.time() - pending[1] < 6:
        state["confirm"] = None
        return True
    state["confirm"] = (key, time.time())
    return False


def asking(state, key, label):
    """The label of a button waiting for its confirming click."""
    pending = state.get("confirm")
    return "sure?" if pending and pending[0] == key and time.time() - pending[1] < 6 else label


def request(state, payload, row, working):
    os.makedirs(REQUESTS, exist_ok=True)
    name = f"{int(time.time() * 1000)}-{payload['type']}.json"
    path = os.path.join(REQUESTS, name)
    with open(path + ".tmp", "w", encoding="utf-8") as handle:
        json.dump(payload, handle)
    os.rename(path + ".tmp", path)
    state["pending"][row] = {"file": name, "working": working, "label": payload["label"], "at": time.time()}


def settle(state):
    """Moves finished requests from pending to their outcome, read from the file the mod writes."""
    for row, job in list(state["pending"].items()):
        path = os.path.join(REQUESTS, "results", job["file"])
        outcome = load(path, None)
        if outcome is None:
            if time.time() - job["at"] > 120:
                state["outcome"][row] = {"ok": False, "message": "no Claude session picked it up", "at": time.time()}
                del state["pending"][row]
            continue
        os.remove(path)
        del state["pending"][row]
        state["outcome"][row] = {**outcome, "at": time.time()}
        note(state, f"{job['label']}: {outcome['message']}")
    for row, outcome in list(state["outcome"].items()):
        if time.time() - outcome["at"] > 10:
            del state["outcome"][row]


def rerun(state, pr, ci_run):
    if confirmed(state, f"rerun-{ci_run['workflowId']}"):
        request(state, {"type": "rerun", "workflowId": ci_run["workflowId"], "label": f"rerun {pr['repo']}#{pr['number']}"},
                pr["url"], "rerunning failed jobs…")


def redeploy(state, box, row):
    if confirmed(state, f"redeploy-{box['stage']}-{box['product']}"):
        request(state, {"type": "redeploy", "stage": box["stage"], "product": box["product"],
                        "label": f"redeploy {box['product']} on {box['stage']}"}, row, "redeploying…")


def create_sandbox(state, pr):
    key = f"sandbox-{pr['repo']}-{pr.get('branch')}"
    if confirmed(state, key):
        request(state, {"type": "create-sandbox", "product": pr["repo"], "branch": pr.get("branch"),
                        "label": f"sandbox for {pr['repo']} {pr.get('branch')}"}, pr["url"], "creating…")


def stop_sandbox(state, stage):
    if confirmed(state, f"stop-{stage}"):
        request(state, {"type": "stop-sandbox", "stage": stage, "label": f"stop {stage}"}, f"sandbox-{stage}", "stopping…")


def extend(state, stage, days=2):
    if confirmed(state, f"extend-{stage}"):
        request(state, {"type": "extend", "stage": stage, "days": days, "label": f"extend {stage}"}, f"sandbox-{stage}", "extending…")


def mark_done(state, ticket):
    if confirmed(state, f"done-{ticket['id']}"):
        request(state, {"type": "release", "ticket": ticket["id"], "label": f"{ticket['id']}"}, ticket["id"], "moving to Done…")


def set_muted(state, repo, is_muted):
    muted = set(load(MUTED, []))
    muted = muted | {repo} if is_muted else muted - {repo}
    os.makedirs(os.path.dirname(MUTED), exist_ok=True)
    with open(MUTED, "w", encoding="utf-8") as handle:
        json.dump(sorted(muted), handle)
    note(state, f"{'muted' if is_muted else 'unmuted'} {repo}")


def dismiss_brief(state):
    """Hides today's brief; tomorrow's shows again."""
    brief = load(BRIEF, {})
    brief["dismissed"] = True
    with open(BRIEF + ".tmp", "w", encoding="utf-8") as handle:
        json.dump(brief, handle)
    os.replace(BRIEF + ".tmp", BRIEF)
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


class Line:
    def __init__(self):
        self.parts = []  # (text, style)
        self.spans = []  # (start, end, action)
        self.buttons = []  # (label, action, key, is_main)
        self.button_spans = []
        self.status = None  # (text, style): a request in flight or its outcome, shown instead of the buttons
        self.primary = None

    def add(self, text, style="", action=None):
        start = sum(width(t) for t, _ in self.parts)
        self.parts.append((text, style))
        if action:
            self.spans.append((start, start + width(text), action))
        return self

    def keys(self):
        return {key: fn for _, fn, key, _ in self.buttons}

    def render(self, cols, is_active, focus=-1):
        # The buttons (or a request's status) sit at the right edge and cover the end of the row
        # while it is active, so no column is reserved for them the rest of the time.
        tail = []
        if self.status:
            tail = [self.status]
        elif is_active and self.buttons:
            for n, (label, fn, _, is_main) in enumerate(self.buttons):
                cap, body = PILL_FOCUS if n == focus else PILL_MAIN if is_main else PILL
                text = f"{ICON.get(label.split()[0], ICON['open'])} {label}"
                tail += [("\ue0b6", cap, fn), (text, body, fn), ("\ue0b4", cap, fn), (" ", "", None)]
        tail_width = sum(width(t[0]) for t in tail)
        room_for_parts = cols - tail_width - (2 if tail else 0)
        out, used = "", 0
        for text, style in self.parts:
            room = room_for_parts - used
            if room <= 0:
                break
            piece = text if width(text) <= room else fit(text, room)
            out += style + piece + RESET
            used += width(piece)
        out += " " * max(0, cols - tail_width - used)
        self.button_spans = []
        position = max(used, cols - tail_width)
        for text, style, *action in tail:
            out += style + text + RESET
            if action and action[0]:
                self.button_spans.append((position, position + width(text), action[0]))
            position += width(text)
        if is_active and self.primary:
            out = HOVER + out.replace(RESET, RESET + HOVER) + RESET
        return out


def build(state, cols):
    data = load(INBOX, None)
    muted = set(load(MUTED, []))
    ci = load(CI, {"runs": []})
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
    head.buttons = [("refresh", lambda: request_refresh(state), "R", False)]
    head.primary = lambda: request_refresh(state)
    for error in data.get("errors", []) + ([ci["error"]] if ci.get("error") else []):
        line().add(error, RED)

    # The morning brief the mod writes once a day, until dismissed.
    brief = load(BRIEF, {})
    if brief.get("date") == datetime.now().strftime("%Y-%m-%d") and not brief.get("dismissed") and brief.get("lines"):
        line()
        written = datetime.fromtimestamp(brief["at"] / 1000).strftime("%H:%M")
        top = line().add(f"{ICON['today']}  ", ORANGE).add("Today", BOLD + ORANGE).add(f"   morning brief, {written}", DIM)
        top.buttons = [("dismiss", lambda: dismiss_brief(state), "x", False)]
        top.primary = lambda: dismiss_brief(state)
        for entry in brief["lines"]:
            style = {"bad": RED, "warn": ORANGE, "good": GREEN}.get(entry.get("tone"), "")
            item = line().add(" " * LEAD).add(f"{ICON.get(entry['icon'], '·')}  ", style or DIM).add(entry["text"], style)
            if entry.get("url"):
                item.buttons = [("open", lambda u=entry["url"]: open_url(state, u), "o", True)]
                item.primary = lambda u=entry["url"]: open_url(state, u)

    alerts = []  # (text, style, url, stage to extend)
    for stage in sorted({b["stage"] for b in data.get("sandboxes") or []}):
        box = next(b for b in data["sandboxes"] if b["stage"] == stage)
        hours = -age_days(box["expiresAt"]) * 24
        url = sandbox_url(stage)
        if hours <= 0:
            alerts.append((f"sandbox {stage} has expired", RED, url, stage))
        elif hours <= SANDBOX_WARN_HOURS:
            left = f"{int(hours * 60)} min" if hours < 1 else f"{hours:.1f} h"
            alerts.append((f"sandbox {stage} expires in {left}", ORANGE, url, stage))
        if box["health"] not in ("Healthy", "Progressing") and hours > 0:
            alerts.append((f"sandbox {stage} is {box['health'].lower()}", ORANGE, url, None))
    for ci_run in ci.get("runs", []):
        if not ci_run["isRunning"] and ci_run["outcome"] == "failed" and age_days(ci_run["createdAt"]) * 24 <= RED_ALERT_HOURS:
            job = f" · {ci_run['failed']['job']}" if ci_run.get("failed") else ""
            alerts.append((f"ci failed on {ci_run['repo']} {ci_run['branch']}{job}", RED, None, None))
    # A running sandbox whose branch has merged: it can stop, keeping its setup.
    merged = {(m["repo"], m["branch"]) for m in data.get("merged") or []}
    for box in data.get("sandboxes") or []:
        if box.get("activated") and (box["product"], box["branch"]) in merged:
            alerts.append((f"sandbox {box['stage']} runs merged {box['product']} {box['branch']}", ORANGE, sandbox_url(box["stage"]), ("stop", box["stage"])))
    if alerts:
        line()
        for text, style, url, stage in alerts:
            item = line().add(f" {ICON['alert']}  ", style).add(text, style)
            if isinstance(stage, tuple):
                item.buttons.append((asking(state, f"stop-{stage[1]}", "stop"), lambda s=stage[1]: stop_sandbox(state, s), "a", True))
                row_status(item, f"sandbox-{stage[1]}")
            elif stage:
                item.buttons.append((asking(state, f"extend-{stage}", "extend 2 days"), lambda stage=stage: extend(state, stage), "a", True))
                row_status(item, f"sandbox-{stage}")
            if url:
                item.buttons.append(("open", lambda url=url: open_url(state, url), "o", False))
                item.primary = lambda url=url: open_url(state, url)
    line()

    # Review requests: direct ones first, then team ones, muted repos and old ones folded away.
    def review_row(pr):
        item = line()
        item.add(fit(ICON["user"] if pr["isDirect"] else ICON["team"], LEAD), ORANGE if pr["isDirect"] else DIM)
        item.add(fit(f"{pr['repo']}#{pr['number']}", REF), DIM, lambda: details(state, pr))
        item.add(fit(pr["title"], title_room(LEAD + REF + 2)) + "  ", "", lambda: details(state, pr))
        item.add(fit(first_name(pr), NOTES), DIM).add(fit(ago(pr["updatedAt"]), AGE), DIM)
        item.buttons = [("review", lambda: review(state, pr), "a", True), ("open", lambda: open_url(state, pr["url"]), "o", False)]
        if not pr["isDirect"]:
            item.buttons.append(("mute", lambda: set_muted(state, pr["repo"], True), "m", False))
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
        box = next((b for b in data.get("sandboxes") or [] if b["product"] == pr["repo"] and b["branch"] == pr.get("branch")), None)
        image_ready = bool(ci_run and ci_run.get("dockerTag") == "succeeded")
        notes = []
        if ci_run and ci_run["isRunning"]:
            notes.append(f"{ICON['circleci']} {ci_run['done']}/{ci_run['total']}")
        if image_ready:
            notes.append(f"{ICON['docker']} image")
        if box:
            notes.append(f"{ICON['sandbox']} sbx")
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
                ("this PR", lambda: fix_ci(state, pr, ci_run, "here"), "a", True),
                ("new PR", lambda: fix_ci(state, pr, ci_run, "new"), "n", False),
                ("cancel", lambda: state.update(choice=None, btn=-1), "c", False),
            ]
        else:
            if is_failing(pr):
                item.buttons.append(("fix ci", lambda: ask_fix(state, pr), "a", True))
            if ci_run and ci_run.get("outcome") == "failed" and ci_run.get("workflowId") and not ci_run["isRunning"]:
                item.buttons.append((asking(state, f"rerun-{ci_run['workflowId']}", "rerun"), lambda: rerun(state, pr, ci_run), "u", False))
            if box and image_ready:
                item.buttons.append((asking(state, f"redeploy-{box['stage']}-{box['product']}", "redeploy"), lambda: redeploy(state, box, pr["url"]),
                                     "d", not item.buttons))
            # A green image with no sandbox running it yet: one click creates one with the configured defaults.
            if not box and image_ready and CONFIG["backstage"]["enabled"]:
                item.buttons.append((asking(state, f"sandbox-{pr['repo']}-{pr.get('branch')}", "sandbox"), lambda: create_sandbox(state, pr),
                                     "s", not item.buttons))
            # ArgoCD: the sandbox running this branch, else the production app; then its latest prod deploy.
            links = (data.get("deployLinks") or {}).get(pr["repo"]) or {}
            argo = (sandbox_url(box["stage"]) if box else None) or links.get("argo")
            if argo:
                item.buttons.append(("argo", lambda: open_url(state, argo), "A", False))
            if links.get("workflow"):
                item.buttons.append(("deploy", lambda: open_url(state, links["workflow"]["url"]), "W", False))
            item.buttons.append(("open", lambda: open_url(state, pr["url"]), "o", False))
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
                item.buttons.append((asking(state, f"done-{ticket['id']}", "done"), lambda: mark_done(state, ticket), "a", True))
        elif ticket["statusType"] == "started":
            item.buttons.append(("continue", lambda: continue_ticket(state, ticket, prs), "a", True))
        else:
            item.buttons.append(("plan", lambda: plan(state, ticket), "a", True))
        if deploying and deployed:
            links = (data.get("deployLinks") or {}).get(deployed[0]["repo"]) or {}
            if links.get("argo"):
                item.buttons.append(("argo", lambda: open_url(state, links["argo"]), "A", False))
            if links.get("workflow"):
                item.buttons.append(("deploy", lambda: open_url(state, links["workflow"]["url"]), "W", False))
        item.buttons.append(("open", lambda: open_url(state, ticket["url"]), "o", False))
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
    if state["note"] and time.time() - state["note_at"] < 8:
        line().add(state["note"], ORANGE)
    else:
        line().add("hover a row for its buttons · click for details · j/k rows · h/l or tab buttons · enter press · a main action · o open · g group by · R refresh", DIM)
    return lines


# ---------------------------------------------------------------------------
# Terminal loop: alternate screen, SGR mouse with motion, redraw on input or file change


def size():
    try:
        return os.get_terminal_size()
    except OSError:
        return os.terminal_size((120, 40))


def active_index(state):
    if state["has_keyboard"] and state["selectable"]:
        return state["selectable"][state["sel"]]
    return state["hover"]


def draw(state):
    cols, rows = size()
    lines = build(state, cols)
    state["lines"] = lines
    selectable = [i for i, l in enumerate(lines) if l.primary]
    state["selectable"] = selectable
    if selectable:
        state["sel"] = min(state["sel"], len(selectable) - 1)
        if state["has_keyboard"]:
            current = selectable[state["sel"]]
            state["top"] = min(max(state["top"], current - rows + 1), current)
    state["top"] = max(0, min(state["top"], max(0, len(lines) - rows)))
    active = active_index(state)
    out = ["\033[H"]
    for row in range(rows):
        index = state["top"] + row
        line = lines[index] if index < len(lines) else None
        focus = state["btn"] if state["has_keyboard"] and index == active and line else -1
        if line and focus >= len(line.buttons):
            focus = state["btn"] = len(line.buttons) - 1
        text = line.render(cols, index == active, focus) if line else " " * cols
        out.append(text + ("\r\n" if row < rows - 1 else ""))
    sys.stdout.write("".join(out))
    sys.stdout.flush()


def click(state, x, y):
    index = state["top"] + y - 1
    if 0 <= index < len(state["lines"]):
        line = state["lines"][index]
        for start, end, action in line.button_spans + line.spans:
            if start <= x - 1 < end:
                action()
                return


def key(state, data):
    selectable = state["selectable"]
    current = state["lines"][selectable[state["sel"]]] if selectable else None
    state["has_keyboard"] = True
    buttons = current.buttons if current else []
    if data in ("j", "k", "\033[B", "\033[A", "\033[6~", "\033[5~", " "):
        state["btn"], state["choice"] = -1, None
    if data in ("j", "\033[B"):
        state["sel"] = min(state["sel"] + 1, max(0, len(selectable) - 1))
    elif data in ("k", "\033[A"):
        state["sel"] = max(state["sel"] - 1, 0)
    elif data in ("\033[6~", " "):
        state["sel"] = min(state["sel"] + 10, max(0, len(selectable) - 1))
    elif data == "\033[5~":
        state["sel"] = max(state["sel"] - 10, 0)
    elif data in ("l", "\033[C", "\t") and buttons:
        state["btn"] = min(state["btn"] + 1, len(buttons) - 1)
    elif data in ("h", "\033[D", "\033[Z"):
        state["btn"] = max(state["btn"] - 1, -1)
    elif data in ("\r", "\n") and current:
        if 0 <= state["btn"] < len(buttons):
            buttons[state["btn"]][1]()
        elif current.primary:
            current.primary()
    elif data == "\033" and state["choice"]:
        state["choice"], state["btn"] = None, -1
    elif data == "R":
        request_refresh(state)
    elif data == "g":
        state["work_mode"] = "status" if state["work_mode"] == "feature" else "feature"
    elif current and data in current.keys():
        current.keys()[data]()


MOUSE = re.compile(r"\033\[<(\d+);(\d+);(\d+)([Mm])")


def handle(state, data):
    """Returns whether the screen needs drawing again."""
    position, changed = 0, False
    for match in MOUSE.finditer(data):
        button, x, y, kind = int(match[1]), int(match[2]), int(match[3]), match[4]
        if button == 64:
            state["top"] = max(0, state["top"] - 3)
            changed = True
        elif button == 65:
            state["top"] += 3
            changed = True
        elif button & 32:
            index = state["top"] + y - 1
            if index != state["hover"] or state["has_keyboard"]:
                state["hover"], state["has_keyboard"], changed = index, False, True
        elif button == 0 and kind == "M":
            state["hover"], state["has_keyboard"] = state["top"] + y - 1, False
            click(state, x, y)
            changed = True
        position = match.end()
    rest = data[position:] if position else data
    if rest and not rest.startswith("\033[<"):
        key(state, rest)
        changed = True
    return changed


def stamp():
    return tuple(os.path.getmtime(p) if os.path.exists(p) else 0 for p in (INBOX, MUTED, CI, BRIEF))


def main():
    state = {"open": set(), "sel": 0, "top": 0, "hover": -1, "note": "", "note_at": 0, "work_mode": "feature", "btn": -1, "choice": None, "pending": {}, "outcome": {},
             "lines": [], "selectable": [], "has_keyboard": False}
    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    # 1003 reports every mouse move, which is what shows the hovered row's buttons.
    sys.stdout.write("\033[?1049h\033[?25l\033[?1003h\033[?1006h\033]2;inbox\007")
    try:
        tty.setraw(fd)
        seen, last_size, last_tick = None, None, None
        while True:
            now_stamp, now_size, tick = stamp(), size(), int(time.time() // 30)
            # Requests in flight are checked every second, and a pending "sure?" expires on screen.
            busy = bool(state["pending"] or state["outcome"] or state.get("confirm"))
            settle(state)
            if busy or (now_stamp, now_size, tick) != (seen, last_size, last_tick):
                seen, last_size, last_tick = now_stamp, now_size, tick
                if state.get("confirm") and time.time() - state["confirm"][1] >= 6:
                    state["confirm"] = None
                draw(state)
            ready, _, _ = select.select([fd], [], [], 1.0)
            if ready:
                data = os.read(fd, 4096).decode("utf-8", "replace")
                if data == "\x03":
                    break
                if handle(state, data):
                    draw(state)
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)
        sys.stdout.write("\033[?1003l\033[?1006l\033[?25h\033[?1049l")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
