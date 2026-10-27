#!/usr/bin/env python3
"""Runbooks for the herdr runbook space: ~/.config/cockpit/runbooks/*.md, written by /runbook.

Each file is a title, an `updated:` line, a `| # | Step | Where | Status | Command |` table and optional
notes. The newest runbook is open, the others folded. PRs open on click, and a row shows its buttons
while hovered or selected, like the inbox. A step's command shows under it; a click or copy puts it on the clipboard.
"""

import json
import os
import re
import shutil
import subprocess
import textwrap
import time

import cockpit_config
import inbox
import view
from view import BOLD, CAP_CLOSE, CAP_OPEN, DIM, GREEN, HERDR, LINK, ORANGE, PILL_MAIN, RED, Button, Line, fit, note, width

HOME = os.path.expanduser("~")
DIR = os.path.join(HOME, ".config", "cockpit", "runbooks")
INBOX = os.path.join(HOME, ".cache", "cockpit", "inbox.json")
CONFIG = cockpit_config.load()
REPOS = CONFIG["reposDir"]
FOCUS = os.path.join(HOME, ".cache", "cockpit", "runbook-focus.json")
# One file per refresh the mod asks for, and one per runbook naming the session that owns it.
REQUESTS = os.path.join(HOME, ".cache", "cockpit", "runbook-refresh")
OWNERS = os.path.join(HOME, ".cache", "cockpit", "runbook-owners")
# Steps handed to a session, and the mod's tool that session reports their outcome with.
HANDOFFS = os.path.join(HOME, ".cache", "cockpit", "handoffs")
REPORT_TOOL = "mcp__cockpit__report_step"
# A handoff no session took after SENT_WAIT seconds says so; a success shows under its step for DONE_SHOWN seconds.
SENT_WAIT, DONE_SHOWN = 900, 1800
NOTIFY = cockpit_config.script("notify.sh")
# A step that turns done plays for FLASH seconds, each one CASCADE seconds after the previous.
FLASH, CASCADE = 2.6, 0.3
SPINNER = "◐◓◑◒"

ICON = {"done": "", "talk": "\uf086", "run": "", "runbook": "", "copy": "", "open": "", "update": "", "delete": "",
        "fold": "", "unfold": "", "merged": "", "note": ""}
ICON["session"] = "\uf120"
OPEN = ("⬜", "⏳", "🟡", "❌")
NUM, STATUS, WHERE = 4, 22, 30
# Of the width the # column leaves, the status and where columns take these shares and the step the rest.
STATUS_SHARE, WHERE_SHARE = 0.28, 0.24
PR_REF = re.compile(r"^([\w.-]+)#(\d+)$")


# ---------------------------------------------------------------------------
# Files


def parse(path):
    with open(path, encoding="utf-8") as handle:
        lines = handle.read().splitlines()
    book = {"path": path, "slug": os.path.basename(path)[:-3], "title": "", "updated": "", "steps": [], "notes": []}
    in_notes = False
    for raw in lines:
        line = raw.strip()
        if line.startswith("# ") and not book["title"]:
            book["title"] = line[2:]
        elif line.lower().startswith("updated:"):
            book["updated"] = line.split(":", 1)[1].strip()
        elif line.startswith("## "):
            in_notes = "note" in line.lower()
        elif line.startswith("|"):
            cells = [c.strip() for c in line.strip("|").split("|")]
            if len(cells) >= 4 and not set("".join(cells)) <= set("-: ") and cells[0] != "#":
                cells += [""] * (5 - len(cells))
                command = cells[4].strip("`")
                book["steps"].append({"n": cells[0], "step": cells[1], "where": cells[2], "status": cells[3], "command": command})
        elif in_notes and line.startswith(("-", "*", "•")):
            book["notes"].append(line.lstrip("-*• ").strip())
    return book


def read_books():
    try:
        paths = [os.path.join(DIR, f) for f in os.listdir(DIR) if f.endswith(".md")]
    except OSError:
        return []
    paths.sort(key=os.path.getmtime, reverse=True)
    books = []
    for path in paths:
        try:
            books.append(parse(path))
        except (OSError, ValueError):  # unreadable, or not UTF-8: the other runbooks still show
            continue
    return books


def done_steps(book):
    return [s["n"] for s in book["steps"] if not s["status"].startswith(OPEN)]


def play(state, slug, steps):
    """Starts the done animation of these steps, one after the other."""
    start = time.time()
    for i, n in enumerate(steps):
        state["flash"][(slug, n)] = start + i * CASCADE


def reload(state):
    """Reads the runbooks again; a step that was open and is now done plays its animation."""
    books, before = read_books(), state.get("done")
    state["books"], state["done"] = books, {b["slug"]: done_steps(b) for b in books}
    state["handoffs"] = read_handoffs()
    if before is None:
        return
    for book in books:
        if book["slug"] in before:
            newly = [n for n in state["done"][book["slug"]] if n not in before[book["slug"]]]
            if newly:
                play(state, book["slug"], newly)
                # A refresh can write its file more than once: its notification counts every step it turned done.
                state["newly"][book["slug"]] = state["newly"].get(book["slug"], []) + newly


def take_focus(state):
    """A click on a runbook notification: that runbook opened and scrolled to, its steps played as done."""
    try:
        at = os.path.getmtime(FOCUS)
    except OSError:
        return False
    if at <= state["focus_at"]:
        return False
    state["focus_at"] = at
    request = view.load_json(FOCUS, {})
    slugs = [b["slug"] for b in state["books"]]
    slug = request.get("slug")
    if slug not in slugs:
        return False
    if slugs.index(slug) == 0:
        state["toggled"].discard(slug)
    else:
        state["toggled"].add(slug)
    book = state["books"][slugs.index(slug)]
    play(state, slug, request.get("steps") or done_steps(book))
    state["scroll_to"], state["has_keyboard"], state["hover"] = slug, False, -1
    return True


def take_requests(state):
    """Refreshes the mod asks for, e.g. when a ticket goes live: each one announces itself, with its reason, when done."""
    try:
        names = sorted(f for f in os.listdir(REQUESTS) if f.endswith(".json"))
    except OSError:
        return
    requests = []
    for name in names:
        # Renamed before reading, so a request the mod writes meanwhile waits for the next poll instead of being lost.
        taken = os.path.join(REQUESTS, name + ".taken")
        try:
            os.rename(os.path.join(REQUESTS, name), taken)
        except OSError:
            continue
        requests.append(view.load_json(taken, {}))
        os.remove(taken)
    for request in requests:
        book = next((b for b in state["books"] if b["slug"] == request.get("slug")), None)
        if book:
            update(state, book)
            state["reasons"][book["slug"]] = request.get("reason", "")


def is_in_view():
    """Whether the runbook space is the one herdr shows now."""
    try:
        spaces = json.loads(subprocess.run([HERDR, "workspace", "list"], capture_output=True, text=True, timeout=5).stdout)
        return any(w["focused"] and w["label"].endswith("  runbook") for w in spaces["result"]["workspaces"])
    except (ValueError, KeyError, OSError, subprocess.SubprocessError):
        return False


def announce(state, slug):
    """A finished background refresh: a notification whose click opens the runbook and plays its new done steps."""
    book = next((b for b in state["books"] if b["slug"] == slug), None)
    if not book or is_in_view():
        return
    newly = state["newly"].pop(slug, [])
    todo = sum(1 for s in book["steps"] if s["status"].startswith(OPEN))
    body = f"{len(newly)} step{'s' if len(newly) != 1 else ''} done · {todo} to do" if newly else f"no change · {todo} to do"
    reason = state["reasons"].pop(slug, "")
    body = f"{reason} · {body}" if reason else body
    action = f"runbook:{slug}" + (f":{','.join(newly)}" if newly else "")
    subprocess.Popen(["sh", NOTIFY, f"📋 {book['title'] or slug}", body, action], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     start_new_session=True)


def stamp():
    try:
        books = tuple(sorted((f, os.path.getmtime(os.path.join(DIR, f))) for f in os.listdir(DIR)))
    except OSError:
        books = ()
    try:
        books += (os.path.getmtime(INBOX),)
    except OSError:
        pass
    try:
        handoffs = tuple(sorted((f, os.path.getmtime(os.path.join(HANDOFFS, f))) for f in os.listdir(HANDOFFS)))
    except OSError:
        handoffs = ()
    return books, handoffs


def report(books):
    """The count on the runbook space's sidebar row."""
    left = sum(1 for b in books for s in b["steps"] if s["status"].startswith(OPEN))
    view.report("cockpit-runbook", "runbook", f"{ICON['runbook']}  {len(books)} runbooks · {left} to do" if books else f"{ICON['runbook']}  no runbook")


# ---------------------------------------------------------------------------
# Actions


def copy(state, command):
    if not cockpit_config.copy(command):
        note(state, "no clipboard tool: install wl-copy or xclip")
        return
    note(state, f"copied: {command}")


def open_pr(state, repo, number):
    url = cockpit_config.pr_url(repo, number, CONFIG)
    if not url:
        note(state, f"no URL known for {repo}#{number}: set github.org in the cockpit config")
        return
    cockpit_config.open_url(url)
    note(state, f"opened {url}")


CLAUDE = shutil.which("claude") or os.path.join(HOME, ".claude", "local", "claude")
# Claude Code keeps a project's memory under its path with every / and . turned into -.
MEMORY = os.path.join(HOME, ".claude", "projects", re.sub(r"[/.]", "-", REPOS), "memory")
LOGS = os.path.join(HOME, ".cache", "cockpit")
TOOLS = ["Bash(gh:*)", "Bash(git:*)", "Bash(jq:*)", "Read", "Write", "Edit", "Grep", "Glob",
         *[cockpit_config.tool_prefix(CONFIG[source]["server"]) for source in ("tickets", "ci", "platform") if CONFIG[source]["enabled"] and CONFIG[source]["server"]],
         *CONFIG["runbookTools"]]
DEPLOY_FACTS = (f"the platform's {CONFIG['platform']['tools']['deployStatus']['tool']} tool, then gh compare of the merge commit against the deployed tag"
                if CONFIG["platform"]["enabled"] and "deployStatus" in CONFIG["platform"]["tools"]
                else "from the deploy tooling the connected tools reach, then gh compare of the merge commit against the deployed ref")


def refresh_prompt(book):
    return "\n".join([
        f"Update the runbook at {book['path']}: read it first, keep its step order and numbering, refresh every status from live facts,"
        " add the steps that appeared and mark the ones finished.",
        "Gather the facts, never from memory alone: every PR across repos (gh: state, review, CI), whether merged ones are live in"
        f" production ({DEPLOY_FACTS}), the ticket"
        " status, and deploy order, migrations and post-deploy commands from the PRs, the memory notes and the code.",
        "Keep the file's format: `# <TICKET> · <feature>`, `updated: <YYYY-MM-DD HH:MM>`, one `| # | Step | Where | Status | Command |`"
        " table, statuses from ✅ done, 🚀 live, 🟡 in progress, ⏳ waiting, ⬜ to do, ❌ blocked, optional `## Notes` of three bullets.",
        "Do not run any production command or deploy. Write the file, nothing else.",
        f"It is now {time.strftime('%Y-%m-%d %H:%M')} local time: use it for `updated:` and any \"as of\" in a status.",
    ])


def update(state, book):
    """Refreshes the runbook in the background, like the inbox: a headless Claude forked from the
    conversation that wrote it (its context, without touching it), else a fresh one."""
    slug = book["slug"]
    running = state["refreshing"].get(slug)
    if running and running.poll() is None:
        note(state, f"{slug} is already refreshing")
        return
    _, session = owner_of(book)
    dirs = ["--add-dir", DIR] + (["--add-dir", MEMORY] if os.path.isdir(MEMORY) else [])
    command = [CLAUDE, "-p", refresh_prompt(book), "--model", "sonnet", "--permission-mode", "acceptEdits", *dirs, "--allowedTools", *TOOLS]
    if session and session != "-":
        command[1:1] = ["--resume", session, "--fork-session"]
    # The exit status lands beside the log, for an app that only knows the pid (the Rust one) to tell a failure.
    status = os.path.join(LOGS, f"runbook-refresh-{slug}.exit")
    if os.path.exists(status):
        os.remove(status)
    command = ["sh", "-c", '"$@"; code=$?; echo "$code" > "$0"; exit "$code"', status, *command]
    with open(os.path.join(LOGS, f"runbook-refresh-{slug}.log"), "w", encoding="utf-8") as log:
        state["refreshing"][slug] = subprocess.Popen(command, cwd=REPOS, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                                                     start_new_session=True)
    state["refresh_started"][slug] = time.time()
    state["announced"].discard(slug)
    state["newly"].pop(slug, None)
    note(state, f"refreshing {slug} in the background")


def refresh_status(state, book):
    """What the header says about a background refresh: running for how long, failed, or nothing."""
    proc = state["refreshing"].get(book["slug"])
    if not proc:
        return None
    if proc.poll() is None:
        return f"refreshing… {int(time.time() - state['refresh_started'][book['slug']])}s", ORANGE
    if proc.returncode != 0:
        return f"refresh failed, see ~/.cache/cockpit/runbook-refresh-{book['slug']}.log", RED
    return None


def talk(state, book):
    """Talks the runbook's ticket over in the actions space, forking the session that wrote the runbook."""
    data = view.load_json(INBOX, {})
    key = book["slug"].upper() if re.fullmatch(r"[a-z]+-\d+", book["slug"]) else book["slug"]
    ticket = next((t for t in data.get("tickets") or [] if t["id"] == key), None) or {
        "id": key, "title": book["title"], "url": cockpit_config.ticket_url(key, CONFIG, data) if key != book["slug"] else ""}
    prs = [pr for pr in data.get("mine") or [] if cockpit_config.has_ticket(pr.get("branch"), key)]
    _, session = owner_of(book)
    inbox.talk(state, ticket, prs, data, None if session == "-" else session)


def owner_of(book):
    owner = view.load_json(os.path.join(OWNERS, f"{book['slug']}.json"), {})
    return owner.get("pane", "-"), owner.get("session", "-")


def mark_done(state, book, step):
    """Marks a step done by hand: its status cell becomes "✅ done <day>", which plays the done animation on reload."""
    with open(book["path"], encoding="utf-8") as handle:
        lines = handle.read().split("\n")
    for i, line in enumerate(lines):
        cells = line.split("|")
        if line.strip().startswith("|") and len(cells) > 5 and cells[1].strip() == step["n"]:
            cells[4] = f" ✅ done {time.strftime('%d %b')} "
            lines[i] = "|".join(cells)
            break
    else:
        note(state, f"step {step['n']} not found in {book['slug']}")
        return
    with open(book["path"] + ".tmp", "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines))
    os.replace(book["path"] + ".tmp", book["path"])
    handoff = state["handoffs"].get(handoff_id(book, step))
    if handoff and in_flight(handoff):
        view.write_json(os.path.join(HANDOFFS, handoff["id"] + ".json"),
                        {**handoff, "state": "done", "message": "marked done by hand", "reportedAt": int(time.time() * 1000)})
    note(state, f"{book['slug']} step {step['n']} marked done")


def handoff_id(book, step):
    return f"{book['slug']}-{step['n']}"


def in_flight(handoff):
    """Whether a session still has the step: a handoff nobody took for SENT_WAIT seconds no longer counts."""
    if handoff["state"] == "sent":
        return time.time() - handoff["sentAt"] / 1000 <= SENT_WAIT
    return handoff["state"] in ("running", "waiting")


def read_handoffs():
    try:
        names = [f for f in os.listdir(HANDOFFS) if f.endswith(".json")]
    except OSError:
        return {}
    found = (view.load_json(os.path.join(HANDOFFS, f), None) for f in names)
    return {h["id"]: h for h in found if h and "id" in h}


def handoff_line(handoff):
    """What the step's session made of it, shown under the step: (text, style), or None once a success is old news."""
    status, message = handoff["state"], handoff.get("message") or ""
    if status == "sent":
        late = time.time() - handoff["sentAt"] / 1000 > SENT_WAIT
        return ("↳ no Claude session took it: is the cockpit plugin loaded there?", RED) if late else ("↳ sent to its session…", ORANGE)
    if status == "running":
        return "↳ running in its session…", ORANGE
    if status == "waiting":
        return f"↳ ⏸ waits for your go · {message}", BOLD + ORANGE
    if status == "unreported":
        return "↳ its session stopped without saying how it went", RED
    if status == "done":
        return (f"↳ {message}", DIM + GREEN) if time.time() - handoff.get("reportedAt", 0) / 1000 < DONE_SHOWN else None
    return f"↳ {status}: {message}", RED


def run_step(state, book, step):
    """Hands the step to the runbook's own Claude conversation, resumed if it is closed; that session owes its outcome."""
    if not view.confirmed(state, f"run-{book['slug']}-{step['n']}"):
        note(state, f"click run again to hand step {step['n']} to the {book['slug'].upper()} session")
        return
    pane, session = owner_of(book)
    handoff = {"id": handoff_id(book, step), "slug": book["slug"], "step": step["n"], "title": step["step"], "path": book["path"],
               "state": "sent", "sentAt": int(time.time() * 1000)}
    view.write_json(os.path.join(HANDOFFS, handoff["id"] + ".json"), handoff)
    prompt = (f"Run step {step['n']} of the {book['slug'].upper()} runbook ({book['path']}): {step['step']}, on {step['where']}"
              + (f", command `{step['command']}`" if step["command"] else "")
              + ". Check its preconditions first. For anything that touches production, show me exactly what it will do and wait"
              " for my go before running it."
              + f" Handoff {handoff['id']}: report how it went with the {REPORT_TOOL} tool, done, failed or blocked once it has run,"
              " or waiting while it needs my go. It sets the step's status in the runbook; without that tool, set the status yourself.")
    script = cockpit_config.script("ask-session.sh")
    subprocess.Popen(["sh", script, pane or "-", session or "-", prompt], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     start_new_session=True)
    note(state, f"step {step['n']} sent to its session" + (" (resuming it)" if session != "-" else ""))


def focus_session(state, handoff):
    """Brings the tab of the session running the step into view, with its space."""
    if not handoff.get("pane"):
        note(state, "its session has not picked the step up yet")
        return
    try:
        got = subprocess.run([HERDR, "pane", "get", handoff["pane"]], capture_output=True, text=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        note(state, "herdr did not answer: try again")
        return
    try:
        tab = json.loads(got.stdout)["result"]["pane"]["tab_id"]
    except (ValueError, KeyError, TypeError):
        # The session is gone with the step: the row offers run again.
        view.write_json(os.path.join(HANDOFFS, handoff["id"] + ".json"), {**handoff, "state": "unreported"})
        note(state, "its session's pane is gone: run the step again")
        return
    try:
        subprocess.run([HERDR, "tab", "focus", tab], capture_output=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        note(state, "herdr did not answer: try again")


def delete(state, book):
    if view.confirmed(state, f"delete-{book['path']}"):
        os.remove(book["path"])
        note(state, f"deleted {book['slug']}")
    else:
        note(state, f"click delete again to remove the {book['slug']} runbook")


# ---------------------------------------------------------------------------
# Layout


def live_badge(repo, number, inbox):
    """What the inbox knows about a PR right now: open with its CI, or merged."""
    for pr in inbox.get("mine", []):
        if pr["repo"] == repo and pr["number"] == number:
            if pr.get("ci") in ("FAILURE", "ERROR"):
                return " ci✗", RED
            return (" open", ORANGE) if pr.get("review") != "APPROVED" else (" approved", GREEN)
    for pr in inbox.get("merged") or []:
        if pr["repo"] == repo and pr["number"] == number:
            return f" {ICON['merged']} merged", GREEN
    return "", ""


def build(state, cols):
    books = state["books"]
    inbox = view.load_json(INBOX, {})
    lines = []

    def line():
        lines.append(Line())
        return lines[-1]

    left = sum(1 for b in books for s in b["steps"] if s["status"].startswith(OPEN))
    line().add(f"{ICON['runbook']}  ", ORANGE).add("Runbooks", BOLD + ORANGE).add(f"   {len(books)} · {left} steps left", DIM)
    line()
    if not books:
        line().add("  No runbook yet: run /runbook in a Claude session working on a ticket.", DIM)

    # The table spans the full width, like the inbox: a row's buttons cover its end while it is hovered.
    table = cols - 1
    free = table - 4 - NUM - 3
    status_room = max(STATUS, int(free * STATUS_SHARE))
    where_room = max(WHERE, int(free * WHERE_SHARE))
    step_room = max(16, free - status_room - where_room)
    for index, book in enumerate(books):
        is_open = (index == 0) != (book["slug"] in state["toggled"])
        todo = sum(1 for s in book["steps"] if s["status"].startswith(OPEN))
        head = line()
        head.add(f"  {ICON['unfold'] if is_open else ICON['fold']} ", DIM, lambda b=book: state["toggled"].__ixor__({b["slug"]}))
        head.add(book["title"] or book["slug"], BOLD + (ORANGE if todo else GREEN), lambda b=book: state["toggled"].__ixor__({b["slug"]}))
        head.add(f"   updated {book['updated']} · {todo} to do" if todo else f"   updated {book['updated']} · all done", DIM)
        # Refresh stays visible on every runbook, not only on hover, at the table's right edge.
        refresh = lambda b=book: update(state, b)
        status = refresh_status(state, book)
        pill = [(status[0], status[1], None)] if status else [
            (CAP_OPEN, PILL_MAIN[0], refresh), (f"{ICON['update']} refresh", PILL_MAIN[1], refresh), (CAP_CLOSE, PILL_MAIN[0], refresh)]
        used = sum(width(t) for t, _ in head.parts)
        head.add(" " * max(3, table - used - sum(width(t) for t, _, _ in pill)))
        for text, style, action in pill:
            head.add(text, style, action)
        head.anchor = book["slug"]
        head.buttons = ([] if status else [Button("refresh", refresh, main=True)]) + [Button("talk", lambda b=book: talk(state, b)),
                                                                                      Button("delete", lambda b=book: delete(state, b))]
        head.primary = lambda b=book: state["toggled"].__ixor__({b["slug"]})
        if not is_open:
            continue
        cols_head = line().add(" " * 4)
        for label, cells in (("#", NUM), ("status", status_room), ("step", step_room), ("where", where_room)):
            cols_head.add(fit(label, cells) + " ", DIM)
        for step in book["steps"]:
            row = line().add(" " * 4)
            is_done = not step["status"].startswith(OPEN)
            row.add(fit(step["n"], NUM) + " ", DIM)
            started = state["flash"].get((book["slug"], step["n"]))
            t = time.time() - started if started is not None else None
            if t is not None and t < 0:
                row.add(fit(f"{SPINNER[0]} " + step["status"].split(" ", 1)[-1], status_room) + " ", DIM)
            elif t is not None and t < 0.5:
                row.add(fit(f"{SPINNER[int(t * 14) % 4]} " + step["status"].split(" ", 1)[-1], status_room) + " ", ORANGE)
            elif t is not None and t < FLASH:
                # The tint fades from teal to nothing; a spark shows for the first second.
                fade = 1 - (t - 0.5) / (FLASH - 0.5)
                row.bg = "\033[48;2;%d;%d;%dm" % (int(16 + 24 * fade), int(16 + 84 * fade), int(16 + 64 * fade))
                status = step["status"] + (" ✦" if t < 1.5 else "")
                row.add(fit(status, status_room) + " ", BOLD + GREEN)
            else:
                row.add(fit(step["status"], status_room) + " ", "" if not is_done else DIM)
            row.add(fit(step["step"], step_room) + " ", DIM if is_done else "")
            ref = PR_REF.match(step["where"])
            buttons = []
            if ref:
                repo, number = ref[1], int(ref[2])
                badge, badge_style = live_badge(repo, number, inbox)
                where = fit(step["where"], where_room - width(badge))
                row.add(where, LINK + DIM, lambda r=repo, n=number: open_pr(state, r, n)).add(badge, badge_style).add(" ")
                buttons.append(Button("open", lambda r=repo, n=number: open_pr(state, r, n)))
            else:
                row.add(fit(step["where"], where_room) + " ", DIM)
            if step["command"]:
                buttons.append(Button("copy", lambda c=step["command"]: copy(state, c)))
            handoff = state["handoffs"].get(handoff_id(book, step))
            if not is_done:
                if handoff and in_flight(handoff):
                    buttons.insert(0, Button("session", lambda h=handoff: focus_session(state, h), main=True))
                else:
                    buttons.insert(0, Button("run", lambda b=book, s=step: run_step(state, b, s), main=True))
                buttons.insert(1, Button("done", lambda b=book, s=step: mark_done(state, b, s)))
            row.buttons = buttons
            row.primary = buttons[0].action if buttons else (lambda: None)
            # The command under its step, wrapped over the full width rather than cut; a click copies it.
            if step["command"]:
                indent = 4 + NUM + 1 + status_room + 1
                for part in textwrap.wrap(step["command"], max(20, cols - indent - 2), break_on_hyphens=False):
                    line().add(" " * indent).add(part, DIM if is_done else ORANGE, lambda c=step["command"]: copy(state, c))
            said = handoff_line(handoff) if handoff else None
            if said:
                line().add(" " * (4 + NUM + 1 + status_room + 1)).add(*said)
        for text in book["notes"]:
            for n, part in enumerate(textwrap.wrap(text, max(20, cols - 8), break_on_hyphens=False)):
                line().add(f"    {ICON['note']} " if n == 0 else " " * 6, DIM).add(part, DIM)
        line()

    if view.noted(state):
        line().add(state["note"], ORANGE)
    else:
        line().add("run hands a step to its Claude session, which reports back under it · click a PR to open it, a command to copy it · j/k rows · h/l or tab buttons · enter press · /runbook in a session writes one", DIM)
    return lines


# ---------------------------------------------------------------------------
# The loop


def poll(state):
    """Runbook files changing, background refreshes ending, notification clicks and the done animation all redraw."""
    changed = False
    now = stamp()
    if now != state["seen"]:
        state["seen"] = now
        reload(state)
        report(state["books"])
        changed = True
    for slug, proc in list(state["refreshing"].items()):
        if proc.poll() == 0 and slug not in state["announced"]:
            state["announced"].add(slug)
            announce(state, slug)
    changed |= take_focus(state)
    take_requests(state)
    state["flash"] = {k: v for k, v in state["flash"].items() if time.time() - v < FLASH}
    # A running refresh redraws every second for its timer, and once more when it ends.
    busy = any(proc.poll() is None for proc in state["refreshing"].values())
    changed |= bool(state["flash"]) or busy or busy != state["was_busy"]
    state["was_busy"] = busy
    return changed


def main():
    os.makedirs(DIR, exist_ok=True)
    state = view.new_state(books=read_books(), toggled=set(), refreshing={}, refresh_started={}, flash={}, newly={},
                           announced=set(), focus_at=time.time(), reasons={}, seen=None, was_busy=False, handoffs={})
    reload(state)
    report(state["books"])
    view.App("runbook", state, build, ICON, poll=poll,
             timeout=lambda state: 0.06 if state["flash"] else 1.0).run()


if __name__ == "__main__":
    main()