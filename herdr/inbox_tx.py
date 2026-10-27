#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["textual>=8,<9"]
# ///
"""inbox.py on Textual, for the config's `apps: "full"`: the same caches, acting through inbox.py's own functions.

The inbox is one tree (review requests, then projects → tickets → PRs, stacked PRs under their base). The highlighted row's
actions sit in a panel under it, ctrl+p searches every row's actions, and an action that changes something asks first.
"""

import time
from functools import partial
from typing import Callable, NamedTuple, Optional

from rich.text import Text
from textual import on, work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.command import DiscoveryHit, Hit, Provider
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.theme import Theme
from textual.widgets import Button, Footer, OptionList, Static, Tree
from textual.widgets.option_list import Option

import cockpit_config
import inbox as ib
import view
from inbox import ICON

VESPER = Theme(name="vesper", primary="#FFC799", secondary="#99FFE4", accent="#FFC799", foreground="#D2D2D2",
               background="#101010", surface="#161616", panel="#1E1E22", success="#99FFE4", warning="#FFC799",
               error="#FF8080", dark=True)
ORANGE, GREEN, RED, DIM = "#FFC799", "#99FFE4", "#FF8080", "dim"
STYLE = {view.ORANGE: ORANGE, view.GREEN: GREEN, view.RED: RED, view.DIM: DIM, "": ""}
LEAD, REF, NOTES, AGE = 4, 24, 16, 5


class Act(NamedTuple):
    label: str
    key: Optional[str]
    run: Callable[[], object]
    main: bool = False
    confirm: Optional[str] = None  # the key inbox.py's `confirmed` checks: asked in a dialog first
    choices: tuple = ()  # (label, callable) to pick from in a dialog instead of running `run`


class Row:
    """A tree row: either aligned cells (lead glyphs, ref, title, notes, age) or one free text."""

    def __init__(self, key, text=None, cells=None, acts=(), primary=None, info=(), status=None, expand=True, name="", section=""):
        self.key, self.text, self.cells, self.acts, self.primary = key, text, cells, list(acts), primary
        self.info, self.status, self.expand, self.section = list(info), status, expand, section
        self.name = name or (text.plain.strip() if text is not None else "")
        self.children, self.path = [], key

    def add(self, row):
        self.children.append(row)
        return row

    def render(self, room, state):
        if self.cells is None:
            text = self.text.copy()
            text.truncate(max(1, room))
            return text
        lead, (ref, ref_style), (title, title_style), (notes, notes_style), age = self.cells
        out = Text()
        for glyph, style in lead:
            out.append(glyph, style)
        out.append(" " * max(0, LEAD - out.cell_len))
        # Narrow panes and deep rows give up ref cells first, so the title keeps some room and the age stays in view.
        avail = room - LEAD - NOTES - AGE - 1
        ref_room = min(REF, max(10, avail // 2))
        out.append(ib.fit(ref, ref_room - 1) + " ", ref_style)
        job, outcome = state["pending"].get(self.status), state["outcome"].get(self.status)
        if job:
            notes, notes_style, age = f"{ICON['refresh']} {job['working']}", ORANGE, ""
        elif outcome:
            notes, notes_style, age = outcome["message"], GREEN if outcome["ok"] else RED, ""
        title_room = max(4, avail - ref_room)
        out.append(ib.fit(title, title_room) + " ", title_style)
        out.append(ib.fit(notes, NOTES + (AGE if not age else 0)), notes_style)
        if age:
            out.append(ib.fit(age.rjust(AGE), AGE), DIM)
        return out


def heading(icon, title, extra=""):
    return Text.assemble((f"{ICON[icon]}  ", ORANGE), (title, f"bold {ORANGE}"), (f"   {extra}", DIM))


# ---------------------------------------------------------------------------
# The model: inbox.py's build, as rows of a tree instead of lines


def model(state):
    """The header, the error lines and the tree's top rows, from the caches."""
    data = view.load_cached(ib.INBOX, None)
    if data is None:
        return Text("No data yet: the cockpit mod in a Claude session fills ~/.cache/cockpit/inbox.json.", DIM), [], []
    if data.get("version") != ib.INBOX_VERSION:
        return Text("Refreshing: the inbox cache comes from another cockpit version.", DIM), [], []
    muted = set(view.load_cached(ib.MUTED, []))
    ci = view.load_cached(ib.CI, {"runs": []})
    runs = {(r["repo"], r["branch"]): r for r in ci.get("runs", [])}
    updated = time.strftime("%H:%M", time.localtime(data["at"] / 1000))
    head = Text.assemble((f"{ICON['inbox']}  ", ORANGE), ("Inbox", f"bold {ORANGE}"), (f"   updated {updated}", DIM))
    errors = [Text(e, RED) for e in data.get("errors", []) + ([ci["error"]] if ci.get("error") else [])]
    top = []
    open_ = lambda url: partial(ib.open_url, state, url)

    # The morning brief, until dismissed.
    brief = view.load_cached(ib.BRIEF, {})
    if brief.get("date") == time.strftime("%Y-%m-%d") and not brief.get("dismissed") and brief.get("lines"):
        written = time.strftime("%H:%M", time.localtime(brief["at"] / 1000))
        today = Row("today", heading("today", "Today", f"morning brief, {written}"), acts=[Act("dismiss", "x", partial(ib.dismiss_brief, state), True)])
        for n, entry in enumerate(brief["lines"]):
            style = {"bad": RED, "warn": ORANGE, "good": GREEN}.get(entry.get("tone"), "")
            acts = [Act("open", "o", open_(entry["url"]), True)] if entry.get("url") else []
            today.add(Row(f"brief:{n}", Text.assemble((f"{ICON.get(entry['icon'], '·')}  ", style or DIM), (entry["text"], style)), acts=acts,
                          primary=acts[0].run if acts else None, info=[Text(entry["text"], style)], name=entry["text"], section="Today"))
        top.append(today)

    # Alerts: previews expiring, unhealthy or running a merged branch, red CI, sessions left on merged branches.
    alerts = Row("alerts", heading("alert", "Alerts"))
    for preview in sorted({b["id"] for b in data.get("previews") or []}):
        box = next(b for b in data["previews"] if b["id"] == preview)
        days = ib.age_days(box["expiresAt"])
        if days is None:
            continue
        hours, url = -days * 24, ib.dashboard_url(preview)
        if hours <= ib.PREVIEW_WARN_HOURS:
            text = f"preview {preview} has expired" if hours <= 0 else f"preview {preview} expires in {hours:.1f} h"
            alerts.add(Row(f"alert:expire:{preview}", Text(f" {ICON['alert']}  {text}", RED if hours <= 0 else ORANGE), status=f"preview-{preview}",
                           acts=[Act("extend 2 days", "a", partial(ib.extend, state, preview), True, f"extend-{preview}"), Act("open", "o", open_(url))],
                           primary=open_(url), name=text, section="Alerts"))
        if box["health"] not in ("Healthy", "Progressing") and hours > 0:
            text = f"preview {preview} is {box['health'].lower()}"
            alerts.add(Row(f"alert:health:{preview}", Text(f" {ICON['alert']}  {text}", ORANGE), acts=[Act("open", "o", open_(url), True)],
                           primary=open_(url), name=text, section="Alerts"))
    for ci_run in ci.get("runs", []):
        days = ib.age_days(ci_run.get("createdAt"))
        if not ci_run["isRunning"] and ci_run["outcome"] == "failed" and days is not None and days * 24 <= ib.RED_ALERT_HOURS:
            job = f" · {ci_run['failed']['job']}" if ci_run.get("failed") else ""
            text = f"ci failed on {ci_run['repo']} {ci_run['branch']}{job}"
            alerts.add(Row(f"alert:ci:{ci_run['repo']}:{ci_run['branch']}", Text(f" {ICON['alert']}  {text}", RED), name=text, section="Alerts"))
    merged = {(m["repo"], m["branch"]) for m in data.get("merged") or []}
    for box in data.get("previews") or []:
        if box.get("activated") and (box["app"], box["branch"]) in merged:
            text, url = f"preview {box['id']} runs merged {box['app']} {box['branch']}", ib.dashboard_url(box["id"])
            alerts.add(Row(f"alert:merged:{box['id']}", Text(f" {ICON['alert']}  {text}", ORANGE), status=f"preview-{box['id']}",
                           acts=[Act("stop", "a", partial(ib.stop_preview, state, box["id"]), True, f"stop-{box['id']}"), Act("open", "o", open_(url))],
                           primary=open_(url), name=text, section="Alerts"))
    for space, (repo, branch) in ib.merged_sessions(merged, data["mine"]).items():
        label = f"merged {repo} {branch}"
        is_worktree = bool(((ib.linked_spaces().get(space) or {}).get("worktree") or {}).get("is_linked_worktree"))
        is_tab = space.startswith("tab:")
        verb = "close tab" if is_tab else "remove worktree" if is_worktree else "close space"
        go = [ib.HERDR, "tab", "focus", space[len("tab:"):]] if is_tab else [ib.HERDR, "workspace", "focus", space]
        alerts.add(Row(f"alert:session:{space}", Text(f" {ICON['alert']}  a session still works on {label}", DIM), name=f"session on {label}", section="Alerts",
                       acts=[Act(verb, "a", partial(ib.close_space, state, space, label, is_worktree), True, f"close-{space}"), Act("go", "o", partial(ib.run, go))]))
    if alerts.children:
        alerts.text.append(f"{len(alerts.children)}", DIM)
        top.append(alerts)

    # Review requests: direct ones first, then team ones; old ones and muted repos folded away.
    def review_row(pr):
        acts = [Act("review", "a", partial(ib.review, state, pr), True), Act("open", "o", open_(pr["url"]))]
        if not pr["isDirect"]:
            acts.append(Act("mute repo", "m", partial(ib.set_muted, state, pr["repo"], True)))
        info = [Text.assemble((f"{pr['repo']}#{pr['number']}  ", DIM), (pr["title"], "bold")),
                Text(f"{'asked of you' if pr['isDirect'] else 'asked of your team'} · by {pr.get('authorName') or pr['author']} · updated {ib.ago(pr['updatedAt'])} ago", DIM),
                Text(pr["url"], DIM)]
        return Row(pr["url"], cells=([(ICON["user"] if pr["isDirect"] else ICON["team"], ORANGE if pr["isDirect"] else DIM)], (f"{pr['repo']}#{pr['number']}", DIM),
                                     (pr["title"], ""), (ib.first_name(pr), DIM), ib.ago(pr["updatedAt"])),
                   acts=acts, primary=partial(ib.details, state, pr), info=info, status=pr["url"], name=f"{pr['repo']}#{pr['number']} {pr['title']}", section="Review")

    shown = [pr for pr in data["team"] if pr["repo"] not in muted]
    direct = [pr for pr in data["direct"] if (ib.age_days(pr["updatedAt"]) or 0) <= ib.OLD_REVIEW_DAYS]
    team = [pr for pr in shown if (ib.age_days(pr["updatedAt"]) or 0) <= ib.OLD_REVIEW_DAYS]
    old = [pr for pr in data["direct"] + shown if (ib.age_days(pr["updatedAt"]) or 0) > ib.OLD_REVIEW_DAYS]
    bots = [r for r in data.get("botRepos", []) if r not in muted]
    hidden = len(data["team"]) - len(shown) + len(data.get("botRepos", [])) - len(bots)
    reviews = Row("review", heading("review", "Review", f"{len(direct)} for you · {len(team)} team · {len(bots)} bots" + (f" · {hidden} muted" if hidden else "")))
    for pr in direct + team:
        reviews.add(review_row(pr))
    if old:
        group = reviews.add(Row("review:old", Text(f"{len(old)} older than {ib.OLD_REVIEW_DAYS} days", DIM), expand=False))
        for pr in old:
            group.add(review_row(pr))
    if muted:
        group = reviews.add(Row("review:muted", Text(f"{ICON['mute']} {len(muted)} muted repos", DIM), expand=False))
        for repo in sorted(muted):
            group.add(Row(f"muted:{repo}", Text(repo, DIM), acts=[Act("unmute", "a", partial(ib.set_muted, state, repo, False), True)], name=repo, section="Muted"))
    top.append(reviews)

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
        return [pr] + [p for kid in pr_children.get(pr_key(pr), []) for p in pr_subtree(kid)]

    def pr_row(pr, section):
        ci_run = runs.get(pr_key(pr))
        box = next((b for b in data.get("previews") or [] if b["app"] == pr["repo"] and b["branch"] == pr.get("branch")), None)
        image_ready = bool(ci_run and ci_run.get("dockerTag") == "succeeded")
        notes = []
        if ci_run and ci_run["isRunning"]:
            notes.append(f"{ICON['ci']} {ci_run['done']}/{ci_run['total']}")
        if image_ready:
            notes.append(f"{ICON['docker']} image")
        if box:
            notes.append(f"{ICON['preview']} preview")
        ci_g, ci_s = ib.ci_glyph(pr)
        rv_g, rv_s = ib.review_glyph(pr)
        acts = []
        if ib.is_failing(pr):
            acts.append(Act("fix ci", "a", None, True, choices=(("on this PR's branch", partial(ib.fix_ci, state, pr, ci_run, "here")),
                                                                 ("on a new PR off main", partial(ib.fix_ci, state, pr, ci_run, "new")))))
        if ci_run and ci_run.get("outcome") == "failed" and ci_run.get("workflowId") and not ci_run["isRunning"]:
            acts.append(Act("rerun", "u", partial(ib.rerun, state, pr, ci_run), False, f"rerun-{ci_run['workflowId']}"))
        if box and image_ready:
            acts.append(Act("redeploy", "d", partial(ib.redeploy, state, box, pr["url"]), not acts, f"redeploy-{box['id']}-{box['app']}"))
        if not box and image_ready and ib.CONFIG["platform"]["enabled"] and "createPreview" in ib.CONFIG["platform"]["tools"]:
            acts.append(Act("preview", "s", partial(ib.create_preview, state, pr), not acts, f"preview-{pr['repo']}-{pr.get('branch')}"))
        links = (data.get("deployLinks") or {}).get(pr["repo"]) or {}
        dashboard = (ib.dashboard_url(box["id"]) if box else None) or links.get("dashboard")
        if dashboard:
            acts.append(Act(cockpit_config.dashboard_name(dashboard), "A", open_(dashboard)))
        if links.get("workflow"):
            acts.append(Act("deploy", "W", open_(links["workflow"]["url"])))
        acts.append(Act("open", "o", open_(pr["url"]), not acts))
        review = (pr.get("review") or "no review").lower().replace("_", " ")
        ci_state = (f"running {ci_run['done']}/{ci_run['total']}" if ci_run and ci_run["isRunning"] else (pr.get("ci") or "none").lower())
        info = [Text.assemble((f"{pr['repo']}#{pr['number']}  ", DIM), (pr["title"], "bold")),
                Text(f"{pr.get('branch')} → {pr.get('base')}" + (" · draft" if pr.get("isDraft") else "") + f" · updated {ib.ago(pr['updatedAt'])} ago", DIM),
                Text.assemble(("review ", DIM), (review, STYLE[rv_s]), ("   ci ", DIM), (ci_state, STYLE[ci_s]),
                              (f"   preview {box['id']} · {box['health'].lower()}" if box else "", DIM))]
        row = Row(pr["url"], cells=([(ci_g, STYLE[ci_s]), (" ", ""), (rv_g, STYLE[rv_s])], (f"{pr['repo']}#{pr['number']}", DIM),
                                    (pr["title"], DIM if pr["isDraft"] else ""), ("  ".join(notes), ORANGE), ib.ago(pr["updatedAt"])),
                  acts=acts, primary=partial(ib.details, state, pr), info=info, status=pr["url"], name=f"{pr['repo']}#{pr['number']} {pr['title']}", section=section)
        if ci_run and ci_run.get("failed") and not ci_run["isRunning"]:
            failed = ci_run["failed"]
            why = f"{len(failed['tests'])} failing: {', '.join(failed['tests'])}" if failed.get("tests") else failed.get("step", "")
            row.add(Row(pr["url"] + ":ci", Text(f"{ICON['ci']} {failed['job']} · {why}", RED)))
            row.info.append(Text(f"{ICON['ci']} {failed['job']} · {why}", RED))
        shown_job = (ci_run.get("failed") or {}).get("job") if ci_run else None
        mirror = ib.CONFIG["ci"]["name"].lower() if ib.CONFIG["ci"]["server"] else None
        checks = [c for c in pr.get("failedChecks") or [] if c != shown_job and not (mirror and c.lower().startswith((f"ci/{mirror}", mirror)))]
        if checks:
            row.add(Row(pr["url"] + ":checks", Text(f"{ICON['github']} {', '.join(checks)}", RED)))
            row.info.append(Text(f"{ICON['github']} {', '.join(checks)}", RED))
        for kid in pr_children.get(pr_key(pr), []):
            row.add(pr_row(kid, section))
        return row

    # Work: each ticket with its sub-issues and its PRs (and theirs, stacked) underneath.
    tickets = data["tickets"]
    costs = ib.ticket_costs()
    ticket_ids = {t["id"] for t in tickets}
    t_children = {}
    for t in tickets:
        if t.get("parentId") in ticket_ids:
            t_children.setdefault(t["parentId"], []).append(t)
    t_roots = [t for t in tickets if t.get("parentId") not in ticket_ids]
    linked = lambda ticket: [pr for pr in mine if cockpit_config.has_ticket(pr.get("branch"), ticket["id"])]

    def linked_roots(ticket):
        prs = linked(ticket)
        keys = {pr_key(pr) for pr in prs}
        return [pr for pr in prs if pr_parent.get(pr_key(pr)) not in keys]

    claimed = set()
    for t in tickets:
        for root in linked_roots(t):
            claimed |= {pr_key(pr) for pr in pr_subtree(root)}

    def ticket_row(ticket, section):
        prs = linked(ticket)
        usd = costs.get(ticket["id"])
        deploying = ib.is_deploying(ticket)
        glyph, style = (ICON["rocket"], DIM) if deploying else (ICON["progress"], ORANGE) if ticket["statusType"] == "started" else (ICON["backlog"], DIM)
        deployed = (data.get("deploys") or {}).get(ticket["id"]) or []
        acts = []
        if deploying:
            if deployed and all(d["isLive"] for d in deployed):
                acts.append(Act("done", "a", partial(ib.mark_done, state, ticket), True, f"done-{ticket['id']}"))
        elif ticket["statusType"] == "started":
            acts.append(Act("continue", "a", partial(ib.continue_ticket, state, ticket, prs), True))
        else:
            acts.append(Act("plan", "a", partial(ib.plan, state, ticket), True))
        if deploying and deployed:
            links = (data.get("deployLinks") or {}).get(deployed[0]["repo"]) or {}
            if links.get("dashboard"):
                acts.append(Act(cockpit_config.dashboard_name(links["dashboard"]), "A", open_(links["dashboard"])))
            if links.get("workflow"):
                acts.append(Act("deploy", "W", open_(links["workflow"]["url"])))
        acts += [Act("talk", "t", partial(ib.talk, state, ticket, prs, data)), Act("open", "o", open_(ticket["url"]))]
        info = [Text.assemble((f"{ticket['id']}  ", DIM), (ticket["title"], "bold")),
                Text(f"{ticket['status']} · {ticket.get('project') or 'no project'} · {len(prs)} PRs" + (f" · ${usd:.2f} in Claude usage" if usd else ""), DIM)]
        row = Row(ticket["id"], cells=([(glyph, style)], (f"{ticket['id']:<10}{ib.short_status(ticket)}", DIM), (ticket["title"], ""),
                                       (f"${usd:.2f}" if usd else "", DIM), ""),
                  acts=acts, primary=open_(ticket["url"]), info=info, status=ticket["id"], name=f"{ticket['id']} {ticket['title']}", section=section)
        for n, done in enumerate(deployed):
            live = f"{ICON['rocket']} live in production" if done["isLive"] else f"{ICON['waiting']} not in production yet, prod runs {done['tag']}"
            row.add(Row(f"{ticket['id']}:deploy:{n}", Text(f"{done['repo']}#{done['number']}  {live}", GREEN if done["isLive"] else DIM)))
            row.info.append(Text(f"{done['repo']}#{done['number']}  {live}", GREEN if done["isLive"] else DIM))
        for kid in t_children.get(ticket["id"], []):
            row.add(ticket_row(kid, section))
        for root in linked_roots(ticket):
            row.add(pr_row(root, section))
        return row

    steps = ["progress", "review", "merge", "test", "deploy"]
    labels = ["in progress", "in review", "to merge", "to test", "to deploy", "todo", "backlog", "triage", "other"]

    def status_order(ticket):
        name = ticket["status"].lower()
        if ticket["statusType"] == "started":
            return next((i for i, step in enumerate(steps) if step in name), 0)
        return {"unstarted": 5, "backlog": 6, "triage": 7}.get(ticket["statusType"], 8)

    def tree_tickets(ticket):
        return [ticket] + [t for kid in t_children.get(ticket["id"], []) for t in tree_tickets(kid)]

    def tree_needs_me(ticket):
        return any(ib.needs_me(pr) for t in tree_tickets(ticket) for root in linked_roots(t) for pr in pr_subtree(root))

    def tree_order(ticket):
        return (0 if tree_needs_me(ticket) else 1, min(status_order(t) for t in tree_tickets(ticket)), ticket["id"])

    in_progress = sum(1 for t in tickets if t["statusType"] == "started" and not ib.is_deploying(t))
    work = Row("work", heading("work", "Work", f"{len(tickets)} tickets · {in_progress} in progress · {len(claimed)} PRs · by {state['work_mode']} (g)"))
    if state["work_mode"] == "feature":
        features = {}
        for root in t_roots:
            features.setdefault(root.get("project") or "No project", []).append(root)
        for name, roots in sorted(features.items(), key=lambda kv: (kv[0] == "No project", min(tree_order(r) for r in kv[1]))):
            marker = ("● ", RED) if any(tree_needs_me(r) for r in roots) else ("", "")
            project = work.add(Row(f"project:{name}", Text.assemble(marker, (name, "bold"), (f"  {sum(len(tree_tickets(r)) for r in roots)}", DIM))))
            for root in sorted(roots, key=tree_order):
                project.add(ticket_row(root, name))
    else:
        by_status = {}
        for root in t_roots:
            order = min(status_order(t) for t in tree_tickets(root))
            by_status.setdefault("needs you" if tree_needs_me(root) else labels[min(order, len(labels) - 1)], []).append(root)
        for label, roots in sorted(by_status.items(), key=lambda kv: min(tree_order(r) for r in kv[1])):
            group = work.add(Row(f"status:{label}", Text(label, RED if label == "needs you" else "bold"), expand=label != "to deploy"))
            for root in sorted(roots, key=tree_order):
                group.add(ticket_row(root, label))
    top.append(work)

    # PRs no assigned ticket claims, grouped by what they need.
    others = [pr for pr in mine if pr_key(pr) not in claimed and pr_parent.get(pr_key(pr)) not in claimed]
    groups = {0: [], 1: [], 2: [], 3: []}
    for root in [pr for pr in others if pr_parent.get(pr_key(pr)) is None]:
        tree = pr_subtree(root)
        rank = (3 if all((ib.age_days(p["updatedAt"]) or 0) > ib.STALE_PR_DAYS for p in tree) else 0 if any(ib.needs_me(p) for p in tree)
                else 2 if all(p.get("review") == "APPROVED" for p in tree) else 1)
        groups[rank].append(root)
    other = Row("other", heading("pr", "Other PRs", f"{len(others)} without an assigned ticket"))
    for rank, (label, style, is_open) in enumerate((("needs you", RED, True), ("waiting for review", DIM, True), ("ready to merge", DIM, False),
                                                     (f"stale, no update in {ib.STALE_PR_DAYS} days", DIM, False))):
        if groups[rank]:
            group = other.add(Row(f"other:{rank}", Text.assemble((label, style), (f"  {sum(len(pr_subtree(r)) for r in groups[rank])}", DIM)), expand=is_open))
            for root in groups[rank]:
                group.add(pr_row(root, "Other PRs"))
    top.append(other)
    return head, errors, top


# ---------------------------------------------------------------------------
# Dialogs


class Sure(ModalScreen):
    """Yes or no before an action that changes something, instead of pressing it twice."""

    BINDINGS = [Binding("y", "answer(True)", "yes"), Binding("n,escape", "answer(False)", "no")]

    def __init__(self, question):
        super().__init__()
        self.question = question

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog"):
            yield Static(self.question)
            with Horizontal(classes="dialog-buttons"):
                yield Button("No  n", id="no", compact=True)
                yield Button("Yes  y", id="yes", variant="warning", compact=True)

    @on(Button.Pressed)
    def pressed(self, event):
        self.dismiss(event.button.id == "yes")

    def action_answer(self, yes):
        self.dismiss(yes)


class Pick(ModalScreen):
    BINDINGS = [Binding("escape", "dismiss(None)", "cancel")]

    def __init__(self, question, choices):
        super().__init__()
        self.question, self.choices = question, choices

    def compose(self) -> ComposeResult:
        with Vertical(classes="dialog"):
            yield Static(self.question)
            yield OptionList(*[Option(label, id=str(n)) for n, (label, _) in enumerate(self.choices)])

    @on(OptionList.OptionSelected)
    def chosen(self, event):
        self.dismiss(self.choices[int(event.option.id)][1])


# ---------------------------------------------------------------------------
# The tree, the panel under it, the palette


class InboxTree(Tree):
    BINDINGS = [Binding("j", "cursor_down", show=False), Binding("k", "cursor_up", show=False)]

    def render_label(self, node, base_style, style):
        row = node.data
        if row is None:
            return Text(str(node.label))
        depth, parent = 0, node.parent
        while parent is not None and parent is not self.root:
            depth, parent = depth + 1, parent.parent
        prefix = (self.ICON_NODE_EXPANDED if node.is_expanded else self.ICON_NODE) if node.allow_expand else "  "
        label = row.render(self.scrollable_content_region.width - depth * self.guide_depth - 2, self.app.state)
        label.stylize(style)
        return Text.assemble((prefix, base_style + style), label)

    def on_key(self, event):
        row = self.cursor_node.data if self.cursor_node else None
        act = next((a for a in (row.acts if row else []) if a.key == event.character), None)
        if act:
            event.stop()
            event.prevent_default()
            self.app.act(act, row)


class RowCommands(Provider):
    def hits(self, main_only):
        app = self.app
        for row in app.rows():
            for act in row.acts:
                if not main_only or act.main:
                    yield f"{act.label}  {row.name}", partial(app.act, act, row), row.section
            if row.name and not main_only:
                yield f"go to  {row.name}", partial(app.go_to, row.path), row.section
        yield "refresh the inbox", partial(ib.request_refresh, app.state), "asks the mod for fresh data"
        yield f"group work by {'status' if app.state['work_mode'] == 'feature' else 'feature'}", app.action_regroup, "g"

    async def discover(self):
        for text, callback, help_ in self.hits(main_only=True):
            yield DiscoveryHit(text, callback, help=help_)

    async def search(self, query):
        matcher = self.matcher(query)
        for text, callback, help_ in self.hits(main_only=False):
            if (score := matcher.match(text)) > 0:
                yield Hit(score, matcher.highlight(text), callback, help=help_)


class InboxApp(App):
    TITLE = "inbox"
    COMMANDS = App.COMMANDS | {RowCommands}
    CSS = """
    Screen { background: $background; }
    #head { height: auto; padding: 0 1; }
    InboxTree { background: $background; padding: 0 1; }
    #panel { height: auto; max-height: 12; border-top: solid $panel; padding: 0 1; background: $background; }
    #actions { height: auto; margin-top: 1; }
    #actions Button { margin-right: 1; }
    Sure, Pick { align: center middle; }
    .dialog { width: 64; height: auto; border: round $primary; background: $surface; padding: 1 2; }
    .dialog-buttons { height: auto; margin-top: 1; }
    .dialog-buttons Button { margin-right: 2; }
    .dialog OptionList { height: auto; margin-top: 1; }
    """
    BINDINGS = [
        Binding("q", "quit", "quit"),
        Binding("g", "regroup", "group by"),
        Binding("R", "refresh_inbox", "refresh"),
        Binding("space", "toggle", "fold", show=True),
    ]

    def __init__(self):
        super().__init__()
        self.state = view.new_state(open=set(), work_mode="feature", choice=None, pending={}, outcome={}, seen=None)
        self.opened, self.closed, self.nodes, self.top = set(), set(), {}, []

    def compose(self) -> ComposeResult:
        yield Static(id="head")
        tree = InboxTree("inbox")
        tree.show_root, tree.guide_depth, tree.auto_expand = False, 3, False
        yield tree
        with Vertical(id="panel"):
            yield Static(id="info")
            yield Horizontal(id="actions")
        yield Footer()

    def on_mount(self):
        self.register_theme(VESPER)
        self.theme = "vesper"
        self.query_one(InboxTree).focus()
        self.reload(force=True)
        self.set_interval(1, self.reload)

    def reload(self, force=False):
        """Requests in flight every second; the caches changing, and every 30 s for the ages, rebuild the tree."""
        busy = bool(self.state["pending"] or self.state["outcome"])
        ib.settle(self.state)
        seen = (ib.stamp(), int(time.time() // 30), self.state["work_mode"])
        if seen != self.state["seen"] or force:
            self.state["seen"] = seen
            self.rebuild()
        elif busy:
            for node in self.nodes.values():
                node.refresh()

    def rebuild(self):
        head, errors, self.top = model(self.state)
        self.query_one("#head", Static).update(Text("\n").join([head, *errors]))
        tree = self.query_one(InboxTree)
        cursor = tree.cursor_node.data.path if tree.cursor_node and tree.cursor_node.data else None
        scroll = tree.scroll_y
        tree.clear()
        self.nodes = {}

        def add(parent, row, path):
            # A stacked PR shows under each ticket its stack belongs to, so a node is known by its path.
            row.path = f"{path}/{row.key}"
            is_open = row.path in self.opened or (row.expand and row.path not in self.closed)
            node = parent.add(row.key, data=row, expand=is_open) if row.children else parent.add_leaf(row.key, data=row)
            self.nodes[row.path] = node
            for kid in row.children:
                add(node, kid, row.path)

        for row in self.top:
            add(tree.root, row, "")
        tree.scroll_to(y=scroll, animate=False)
        self.call_after_refresh(lambda: tree.move_cursor(self.nodes.get(cursor) or (tree.root.children[0] if tree.root.children else None)))

    def rows(self):
        def walk(rows):
            for row in rows:
                yield row
                yield from walk(row.children)
        return walk(self.top)

    @on(Tree.NodeExpanded)
    def expanded(self, event):
        if event.node.data:
            self.opened.add(event.node.data.path)
            self.closed.discard(event.node.data.path)

    @on(Tree.NodeCollapsed)
    def collapsed(self, event):
        if event.node.data:
            self.closed.add(event.node.data.path)
            self.opened.discard(event.node.data.path)

    @on(Tree.NodeHighlighted)
    async def highlighted(self, event):
        row = event.node.data
        info = row.info if row and row.info else [row.text] if row and row.text is not None else [Text(row.name if row else "")]
        self.query_one("#info", Static).update(Text("\n").join(info))
        bar = self.query_one("#actions", Horizontal)
        await bar.remove_children()
        await bar.mount_all([Button(f"{a.label}  {a.key}" if a.key else a.label, name=str(n), variant="primary" if a.main else "default", compact=True)
                       for n, a in enumerate(row.acts if row else [])])

    @on(Tree.NodeSelected)
    def selected(self, event):
        row = event.node.data
        if row and row.primary:
            self.perform(row.primary)
        elif row and row.children:
            event.node.toggle()

    @on(Button.Pressed, "#actions Button")
    def pressed(self, event):
        row = self.query_one(InboxTree).cursor_node.data
        self.act(row.acts[int(event.button.name)], row)

    def act(self, act, row):
        """Runs a row's action: a choice asks which, a change asks first, the rest runs at once."""
        if act.choices:
            self.push_screen(Pick(f"{act.label}: {row.name}", act.choices), lambda run: run and self.perform(run))
        elif act.confirm:
            self.push_screen(Sure(f"{act.label}: {row.name}?"), lambda yes: yes and self.perform(partial(self.armed, act)))
        else:
            self.perform(act.run)

    def armed(self, act):
        # inbox.py's actions run on their second press within a few seconds; the dialog was the first.
        self.state["confirm"] = (act.confirm, time.time())
        act.run()

    @work(thread=True)
    def perform(self, run):
        """herdr and git calls take a moment: off the UI thread, then the note as a toast."""
        before = self.state["note_at"]
        run()
        if self.state["note_at"] != before:
            self.call_from_thread(self.notify, self.state["note"])
        self.call_from_thread(self.reload, True)

    def go_to(self, path):
        node = self.nodes.get(path)
        parent = node.parent if node else None
        while parent is not None:
            parent.expand()
            parent = parent.parent
        self.call_after_refresh(lambda: self.query_one(InboxTree).move_cursor(node))

    def action_regroup(self):
        self.state["work_mode"] = "status" if self.state["work_mode"] == "feature" else "feature"
        self.reload(force=True)

    def action_refresh_inbox(self):
        ib.request_refresh(self.state)
        self.notify(self.state["note"])

    def action_toggle(self):
        self.query_one(InboxTree).action_toggle_node()


if __name__ == "__main__":
    InboxApp().run()
