#!/usr/bin/env python3
"""Runbooks for the herdr runbook space: ~/.config/cockpit/runbooks/*.md, written by /runbook.

Each file is a title, an `updated:` line, a `| # | Step | Where | Status | Command |` table and optional
notes. The newest runbook is open, the others folded. PRs open on click, commands copy on click, and a
row shows its buttons while hovered or selected, like the inbox.
"""

import json
import os
import re
import select
import signal
import shlex
import shutil
import subprocess
import sys
import termios
import time
import tty
import unicodedata

import cockpit_config

HOME = os.path.expanduser("~")
DIR = os.path.join(HOME, ".config", "cockpit", "runbooks")
INBOX = os.path.join(HOME, ".cache", "cockpit", "inbox.json")
CONFIG = cockpit_config.load()
REPOS = CONFIG["reposDir"]
HERDR = os.environ.get("HERDR_BIN_PATH", "herdr")
FOCUS = os.path.join(HOME, ".cache", "cockpit", "runbook-focus.json")
REQUESTS = os.path.join(HOME, ".cache", "cockpit", "runbook-refresh-requests.json")
NOTIFY = cockpit_config.script("notify.sh")
# A step that turns done plays for FLASH seconds, each one CASCADE seconds after the previous.
FLASH, CASCADE = 2.6, 0.3
SPINNER = "◐◓◑◒"

DIM, BOLD, RESET = "\033[2m", "\033[1m", "\033[0m"
GREEN, RED, ORANGE, LINK = "\033[38;2;153;255;228m", "\033[38;2;255;128;128m", "\033[38;2;255;199;153m", "\033[4m"
HOVER = "\033[48;2;30;30;34m"
PILL_MAIN = ("\033[38;2;255;199;153m", "\033[48;2;255;199;153m\033[38;2;16;16;16m")
PILL = ("\033[38;2;44;44;48m", "\033[48;2;44;44;48m\033[38;2;210;210;210m")
PILL_FOCUS = ("\033[38;2;245;245;245m", "\033[48;2;245;245;245m\033[38;2;16;16;16m\033[1m")
ICON = {"done": "", "run": "", "runbook": "", "copy": "", "open": "", "update": "", "delete": "",
        "fold": "", "unfold": "", "merged": "", "note": ""}
OPEN = ("⬜", "⏳", "🟡", "❌")
NUM, STATUS, WHERE, COMMAND, ACTIONS = 4, 22, 30, 36, 24
PR_REF = re.compile(r"^([\w.-]+)#(\d+)$")


def width(text):
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 0 if unicodedata.combining(c) or c == "️" else 1 for c in text)


def fit(text, cells):
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


def load_json(path, default):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError):
        return default


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
        except OSError:
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
    if before is None:
        return
    for book in books:
        if book["slug"] in before:
            newly = [n for n in state["done"][book["slug"]] if n not in before[book["slug"]]]
            if newly:
                play(state, book["slug"], newly)
                state["newly"][book["slug"]] = newly


def take_focus(state):
    """A click on a runbook notification: that runbook opened and scrolled to, its steps played as done."""
    try:
        at = os.path.getmtime(FOCUS)
    except OSError:
        return False
    if at <= state["focus_at"]:
        return False
    state["focus_at"] = at
    request = load_json(FOCUS, {})
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
    requests = load_json(REQUESTS, [])
    if not requests:
        return
    try:
        os.remove(REQUESTS)
    except OSError:
        pass
    for request in requests:
        book = next((b for b in state["books"] if b["slug"] == request.get("slug")), None)
        if book:
            update(state, book)
            state["reasons"][book["slug"]] = request.get("reason", "")


def is_in_view():
    """Whether the runbook space is the one herdr shows now."""
    try:
        spaces = json.loads(subprocess.run([HERDR, "workspace", "list"], capture_output=True, text=True, timeout=5).stdout)
        return any(w["focused"] and "runbook" in w["label"] for w in spaces["result"]["workspaces"])
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
        return tuple(sorted((f, os.path.getmtime(os.path.join(DIR, f))) for f in os.listdir(DIR))) + (os.path.getmtime(INBOX),)
    except OSError:
        return ()


def report(books):
    """The count on the runbook space's sidebar row."""
    workspace = os.environ.get("HERDR_WORKSPACE_ID") if os.environ.get("COCKPIT_SPACE") else None
    if not workspace:
        return
    left = sum(1 for b in books for s in b["steps"] if s["status"].startswith(OPEN))
    value = f"{ICON['runbook']}  {len(books)} runbooks · {left} to do" if books else f"{ICON['runbook']}  no runbook"
    subprocess.run([HERDR, "workspace", "report-metadata", workspace, "--source", "cockpit-runbook", "--token", f"runbook={value}"],
                   capture_output=True, timeout=5)


# ---------------------------------------------------------------------------
# Actions


def note(state, text):
    state["note"], state["note_at"] = text, time.time()


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
         *[cockpit_config.tool_prefix(CONFIG[source]["server"]) for source in ("linear", "circleci", "backstage") if CONFIG[source]["enabled"]],
         *CONFIG["runbookTools"]]
DEPLOY_FACTS = ("Backstage ship_get-deploy-status, then gh compare of the merge commit against the deployed tag"
                if CONFIG["backstage"]["enabled"] else "from the deploy tooling the connected tools reach, then gh compare of the merge commit against the deployed ref")


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
    log = open(os.path.join(LOGS, f"runbook-refresh-{slug}.log"), "w", encoding="utf-8")
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


def owner_of(book):
    owner = load_json(os.path.join(HOME, ".cache", "cockpit", "runbook-owners.json"), {}).get(book["slug"])
    if isinstance(owner, str):
        return owner, "-"
    return (owner or {}).get("pane", "-"), (owner or {}).get("session", "-")


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
    note(state, f"{book['slug']} step {step['n']} marked done")


def run_step(state, book, step):
    """Asks the runbook's own Claude conversation to run the step, resumed if it is closed."""
    key = f"run-{book['slug']}-{step['n']}"
    pending = state.get("confirm")
    if not (pending == key and time.time() - state["confirm_at"] < 6):
        state["confirm"], state["confirm_at"] = key, time.time()
        note(state, f"click run again to hand step {step['n']} to the {book['slug'].upper()} session")
        return
    state["confirm"] = None
    pane, session = owner_of(book)
    prompt = (f"Run step {step['n']} of the {book['slug'].upper()} runbook ({book['path']}): {step['step']}, on {step['where']}"
              + (f", command `{step['command']}`" if step["command"] else "")
              + ". Check its preconditions first. For anything that touches production, show me exactly what it will do and wait"
              " for my go before running it. Then update that step's status in the runbook.")
    script = cockpit_config.script("ask-session.sh")
    subprocess.Popen(["sh", script, pane or "-", session or "-", prompt], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     start_new_session=True)
    note(state, f"step {step['n']} sent to its session" + (" (resuming it)" if session != "-" else ""))


def delete(state, book):
    pending = state.get("confirm")
    if pending == book["path"] and time.time() - state["confirm_at"] < 5:
        os.remove(book["path"])
        state["confirm"] = None
        note(state, f"deleted {book['slug']}")
    else:
        state["confirm"], state["confirm_at"] = book["path"], time.time()
        note(state, f"click delete again to remove the {book['slug']} runbook")


# ---------------------------------------------------------------------------
# Layout


class Line:
    def __init__(self):
        self.parts, self.spans, self.buttons, self.button_spans = [], [], [], []
        self.primary = None
        self.bg = ""
        self.book = None

    def add(self, text, style="", action=None):
        start = sum(width(t) for t, _ in self.parts)
        self.parts.append((text, style))
        if action:
            self.spans.append((start, start + width(text), action))
        return self

    def render(self, cols, is_active, focus=-1):
        parts = list(self.parts)
        self.button_spans = []
        if is_active and self.buttons:
            used = sum(width(t) for t, _ in parts)
            column = max(used + 1, cols - ACTIONS)
            parts.append((" " * (column - used), ""))
            position = column
            for n, (label, fn, is_main) in enumerate(self.buttons):
                cap, body = PILL_FOCUS if n == focus else PILL_MAIN if is_main else PILL
                text = f"{ICON[label]} {label}"
                self.button_spans.append((position, position + width(text) + 2, fn))
                parts += [("", cap), (text, body), ("", cap), (" ", "")]
                position += width(text) + 3
        out, used = "", 0
        for text, style in parts:
            room = cols - used
            if room <= 0:
                break
            piece = text if width(text) <= room else fit(text, room)
            out += style + piece + RESET
            used += width(piece)
        out += " " * max(0, cols - used)
        background = HOVER if is_active and self.primary else self.bg
        if background:
            out = background + out.replace(RESET, RESET + background) + RESET
        return out


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
    inbox = load_json(INBOX, {})
    lines = []

    def line():
        lines.append(Line())
        return lines[-1]

    left = sum(1 for b in books for s in b["steps"] if s["status"].startswith(OPEN))
    line().add(f"{ICON['runbook']}  ", ORANGE).add("Runbooks", BOLD + ORANGE).add(f"   {len(books)} · {left} steps left", DIM)
    line()
    if not books:
        line().add("  No runbook yet: run /runbook in a Claude session working on a ticket.", DIM)

    step_room = max(16, cols - NUM - STATUS - WHERE - COMMAND - ACTIONS - 4)
    for index, book in enumerate(books):
        is_open = (index == 0) != (book["slug"] in state["toggled"])
        todo = sum(1 for s in book["steps"] if s["status"].startswith(OPEN))
        head = line()
        head.add(f"  {ICON['unfold'] if is_open else ICON['fold']} ", DIM, lambda b=book: state["toggled"].__ixor__({b["slug"]}))
        head.add(book["title"] or book["slug"], BOLD + (ORANGE if todo else GREEN), lambda b=book: state["toggled"].__ixor__({b["slug"]}))
        head.add(f"   updated {book['updated']} · {todo} to do" if todo else f"   updated {book['updated']} · all done", DIM)
        # Refresh stays visible on every runbook, not only on hover.
        head.add("   ")
        refresh = lambda b=book: update(state, b)
        status = refresh_status(state, book)
        if status:
            head.add(status[0], status[1])
        else:
            head.add("\ue0b6", PILL_MAIN[0], refresh).add(f"{ICON['update']} refresh", PILL_MAIN[1], refresh).add("\ue0b4", PILL_MAIN[0], refresh)
        head.book = book["slug"]
        head.buttons = [("delete", lambda b=book: delete(state, b), False)]
        head.primary = lambda b=book: state["toggled"].__ixor__({b["slug"]})
        if not is_open:
            continue
        cols_head = line().add(" " * 4)
        for label, cells in (("#", NUM), ("status", STATUS), ("step", step_room), ("where", WHERE), ("command", COMMAND)):
            cols_head.add(fit(label, cells) + " ", DIM)
        for step in book["steps"]:
            row = line().add(" " * 4)
            is_done = not step["status"].startswith(OPEN)
            row.add(fit(step["n"], NUM) + " ", DIM)
            started = state["flash"].get((book["slug"], step["n"]))
            t = time.time() - started if started is not None else None
            if t is not None and t < 0:
                row.add(fit(f"{SPINNER[0]} " + step["status"].split(" ", 1)[-1], STATUS) + " ", DIM)
            elif t is not None and t < 0.5:
                row.add(fit(f"{SPINNER[int(t * 14) % 4]} " + step["status"].split(" ", 1)[-1], STATUS) + " ", ORANGE)
            elif t is not None and t < FLASH:
                # The tint fades from teal to nothing; a spark shows for the first second.
                fade = 1 - (t - 0.5) / (FLASH - 0.5)
                row.bg = "\033[48;2;%d;%d;%dm" % (int(16 + 24 * fade), int(16 + 84 * fade), int(16 + 64 * fade))
                status = step["status"] + (" ✦" if t < 1.5 else "")
                row.add(fit(status, STATUS) + " ", BOLD + GREEN)
            else:
                row.add(fit(step["status"], STATUS) + " ", "" if not is_done else DIM)
            row.add(fit(step["step"], step_room) + " ", DIM if is_done else "")
            ref = PR_REF.match(step["where"])
            buttons = []
            if ref:
                repo, number = ref[1], int(ref[2])
                badge, badge_style = live_badge(repo, number, inbox)
                where = fit(step["where"], WHERE - width(badge))
                row.add(where, LINK + DIM, lambda r=repo, n=number: open_pr(state, r, n)).add(badge, badge_style).add(" ")
                buttons.append(("open", lambda r=repo, n=number: open_pr(state, r, n), False))
            else:
                row.add(fit(step["where"], WHERE) + " ", DIM)
            if step["command"]:
                row.add(fit(step["command"], COMMAND), ORANGE, lambda c=step["command"]: copy(state, c))
            if not is_done:
                buttons.insert(0, ("run", lambda b=book, s=step: run_step(state, b, s), True))
                buttons.insert(1, ("done", lambda b=book, s=step: mark_done(state, b, s), False))
            row.buttons = buttons
            row.primary = buttons[0][1] if buttons else (lambda: None)
        for text in book["notes"]:
            line().add(f"    {ICON['note']} ", DIM).add(text, DIM)
        line()

    if state["note"] and time.time() - state["note_at"] < 6:
        line().add(state["note"], ORANGE)
    else:
        line().add("run hands a step to its Claude session · click a PR to open it, a command to copy it · j/k rows · h/l or tab buttons · enter press · /runbook in a session writes one", DIM)
    return lines


# ---------------------------------------------------------------------------
# Terminal loop


def size():
    try:
        return os.get_terminal_size()
    except OSError:
        return os.terminal_size((120, 40))


def draw(state):
    cols, rows = size()
    lines = build(state, cols)
    state["lines"] = lines
    target = state.pop("scroll_to", None)
    if target:
        state["top"] = next((i for i, l in enumerate(lines) if l.book == target), state["top"])
    selectable = [i for i, l in enumerate(lines) if l.primary]
    state["selectable"] = selectable
    if selectable:
        state["sel"] = min(state["sel"], len(selectable) - 1)
        if state["has_keyboard"]:
            current = selectable[state["sel"]]
            state["top"] = min(max(state["top"], current - rows + 1), current)
    active = selectable[state["sel"]] if state["has_keyboard"] and selectable else state["hover"]
    state["top"] = max(0, min(state["top"], max(0, len(lines) - rows)))
    out = ["\033[H"]
    for row in range(rows):
        index = state["top"] + row
        line = lines[index] if index < len(lines) else None
        focus = state["btn"] if state["has_keyboard"] and index == active and line else -1
        if line and focus >= len(line.buttons):
            focus = state["btn"] = len(line.buttons) - 1
        out.append((line.render(cols, index == active, focus) if line else " " * cols) + ("\r\n" if row < rows - 1 else ""))
    sys.stdout.write("".join(out))
    sys.stdout.flush()


def key(state, data):
    selectable = state["selectable"]
    current = state["lines"][selectable[state["sel"]]] if selectable else None
    buttons = current.buttons if current else []
    state["has_keyboard"] = True
    if data in ("j", "k", "\033[B", "\033[A"):
        state["btn"] = -1
    if data in ("j", "\033[B"):
        state["sel"] = min(state["sel"] + 1, max(0, len(selectable) - 1))
    elif data in ("k", "\033[A"):
        state["sel"] = max(state["sel"] - 1, 0)
    elif data in ("l", "\033[C", "\t") and buttons:
        state["btn"] = min(state["btn"] + 1, len(buttons) - 1)
    elif data in ("h", "\033[D", "\033[Z"):
        state["btn"] = max(state["btn"] - 1, -1)
    elif data in ("\r", "\n", " ") and current:
        if 0 <= state["btn"] < len(buttons):
            buttons[state["btn"]][1]()
        elif current.primary:
            current.primary()


MOUSE = re.compile(r"\033\[<(\d+);(\d+);(\d+)([Mm])")


def handle(state, data):
    position, changed = 0, False
    for match in MOUSE.finditer(data):
        button, x, y, kind = int(match[1]), int(match[2]), int(match[3]), match[4]
        index = state["top"] + y - 1
        if button == 64:
            state["top"] = max(0, state["top"] - 3)
        elif button == 65:
            state["top"] += 3
        elif button & 32:
            if index == state["hover"] and not state["has_keyboard"]:
                position = match.end()
                continue
            state["hover"], state["has_keyboard"] = index, False
        elif button == 0 and kind == "M":
            state["hover"], state["has_keyboard"] = index, False
            if 0 <= index < len(state["lines"]):
                line = state["lines"][index]
                for start, end, action in line.button_spans + line.spans:
                    if start <= x - 1 < end:
                        action()
                        break
        changed = True
        position = match.end()
    rest = data[position:] if position else data
    if rest and not rest.startswith("\033[<"):
        key(state, rest)
        changed = True
    return changed


def main():
    os.makedirs(DIR, exist_ok=True)
    state = {"books": read_books(), "toggled": set(), "refreshing": {}, "refresh_started": {}, "sel": 0, "top": 0, "hover": -1, "btn": -1, "has_keyboard": False,
             "note": "", "note_at": 0, "lines": [], "selectable": [], "confirm": None, "confirm_at": 0,
             "flash": {}, "newly": {}, "announced": set(), "focus_at": time.time(), "reasons": {}}
    reload(state)
    report(state["books"])
    # Stopped by a signal (an update restarting the app), it still restores the terminal on the way out.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    signal.signal(signal.SIGHUP, lambda *_: sys.exit(0))
    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    sys.stdout.write("\033[?1049h\033[?25l\033[?1003h\033[?1006h\033]2;runbook\007")
    try:
        tty.setraw(fd)
        seen, last_size = None, None
        while True:
            now = stamp()
            if now != seen:
                seen = now
                reload(state)
                report(state["books"])
                draw(state)
            for slug, proc in list(state["refreshing"].items()):
                if proc.poll() == 0 and slug not in state["announced"]:
                    state["announced"].add(slug)
                    announce(state, slug)
            if take_focus(state):
                draw(state)
            take_requests(state)
            state["flash"] = {k: v for k, v in state["flash"].items() if time.time() - v < FLASH}
            if state["flash"]:
                draw(state)
            # A running refresh redraws every second for its timer, and once more when it ends.
            busy = any(proc.poll() is None for proc in state["refreshing"].values())
            if size() != last_size or busy or busy != state.get("was_busy"):
                last_size, state["was_busy"] = size(), busy
                draw(state)
            ready, _, _ = select.select([fd], [], [], 0.06 if state["flash"] else 1.0)
            if ready:
                data = os.read(fd, 4096).decode("utf-8", "replace")
                if data == "\x03":
                    break
                if handle(state, data):
                    reload(state)
                    draw(state)
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)
        sys.stdout.write("\033[?1003l\033[?1006l\033[?25h\033[?1049l")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
