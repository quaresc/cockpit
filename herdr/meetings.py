#!/usr/bin/env python3
"""Upcoming meetings for the herdr meetings space, from ~/.cache/cockpit/meetings.json.

The cockpit mod fills the file from Google Calendar every few minutes. Meetings are grouped by
day, the running one highlighted; a row shows its buttons while hovered or selected, like the inbox.
"""

import json
import os
import re
import select
import subprocess
import sys
import termios
import time
import tty
import unicodedata
from datetime import datetime, timedelta

import cockpit_config

HOME = os.path.expanduser("~")
CACHE = os.path.join(HOME, ".cache", "cockpit", "meetings.json")
BRIEFS = os.path.join(HOME, ".cache", "cockpit", "meeting-briefs")
FOCUS = os.path.join(HOME, ".cache", "cockpit", "meetings-focus.json")
BRIEF_SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "meeting-brief.sh")
HERDR = os.environ.get("HERDR_BIN_PATH", "herdr")

DIM, BOLD, RESET = "\033[2m", "\033[1m", "\033[0m"
GREEN, RED, ORANGE = "\033[38;2;153;255;228m", "\033[38;2;255;128;128m", "\033[38;2;255;199;153m"
HOVER, NOW_BG = "\033[48;2;30;30;34m", "\033[48;2;48;38;30m"
PILL_MAIN = ("\033[38;2;255;199;153m", "\033[48;2;255;199;153m\033[38;2;16;16;16m")
PILL = ("\033[38;2;44;44;48m", "\033[48;2;44;44;48m\033[38;2;210;210;210m")
PILL_FOCUS = ("\033[38;2;245;245;245m", "\033[48;2;245;245;245m\033[38;2;16;16;16m\033[1m")
ICON = {"brief": "", "prepare": "", "redo": "", "calendar": "", "join": "\U000f0567", "open": "", "people": "", "now": "",
        "accepted": "", "tentative": "", "needsAction": ""}
TIME, DURATION, PEOPLE, ACTIONS = 14, 8, 6, 26


def width(text):
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 0 if unicodedata.combining(c) else 1 for c in text)


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


def load():
    try:
        with open(CACHE, encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError):
        return None


def parse(iso):
    return datetime.fromisoformat(iso).astimezone()


def until_text(start, now):
    minutes = int((start - now).total_seconds() // 60)
    if minutes < 60:
        return f"in {minutes} min"
    return f"in {minutes // 60} h {minutes % 60:02d}" if minutes < 6 * 60 else start.strftime("%H:%M")


def report(state):
    """The next meeting on the meetings space's sidebar row."""
    workspace = os.environ.get("HERDR_WORKSPACE_ID") if os.environ.get("COCKPIT_SPACE") else None
    if not workspace:
        return
    now = datetime.now().astimezone()
    upcoming = [m for m in (state or {}).get("meetings", []) if parse(m["end"]) > now]
    upcoming.sort(key=lambda m: m["start"])
    if not upcoming:
        value = f"{ICON['calendar']}  no meeting this week"
    else:
        m = upcoming[0]
        start = parse(m["start"])
        when = "now" if start <= now else until_text(start, now) if start.date() == now.date() else start.strftime("%a %H:%M")
        value = f"{ICON['calendar']}  {when} · {m['title'][:20]}"
    subprocess.run([HERDR, "workspace", "report-metadata", workspace, "--source", "cockpit-meetings", "--token", f"meetings={value}"],
                   capture_output=True, timeout=5)


# ---------------------------------------------------------------------------
# Layout


class Line:
    def __init__(self):
        self.parts, self.spans, self.buttons, self.button_spans = [], [], [], []
        self.primary, self.background, self.meeting = None, "", None

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
        background = HOVER if is_active and self.primary else self.background
        if background:
            out = background + out.replace(RESET, RESET + background) + RESET
        return out


def brief_path(m, suffix=".md"):
    return os.path.join(BRIEFS, re.sub(r"[^A-Za-z0-9_-]", "_", m["id"]) + suffix)


def read_brief(m):
    try:
        with open(brief_path(m), encoding="utf-8") as handle:
            return [l.rstrip() for l in handle.read().splitlines()]
    except OSError:
        return None


def prepare(state, m):
    """Writes (or rewrites) a meeting's brief in the background; the row says so until it lands."""
    if os.path.exists(brief_path(m, ".pending")):
        state["note"], state["note_at"] = "already preparing that brief", time.time()
        return
    subprocess.Popen(["sh", BRIEF_SCRIPT, m["id"]], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    state["open"].add(m["id"])
    state["note"], state["note_at"] = f"preparing the brief of {m['title']}, about a minute", time.time()


def toggle_brief(state, m):
    state["open"] ^= {m["id"]}


def take_focus(state):
    """A click on a meeting notification: that meeting's brief open and in view."""
    try:
        at = os.path.getmtime(FOCUS)
    except OSError:
        return False
    if at <= state["focus_at"]:
        return False
    state["focus_at"] = at
    try:
        with open(FOCUS, encoding="utf-8") as handle:
            target = json.load(handle).get("id")
    except (OSError, ValueError):
        return False
    state["open"].add(target)
    state["scroll_to"], state["has_keyboard"], state["hover"] = target, False, -1
    return True


def open_url(state, url):
    cockpit_config.open_url(url)
    state["note"], state["note_at"] = f"opened {url}", time.time()


def build(state, cols):
    data = state["data"]
    lines = []

    def line():
        lines.append(Line())
        return lines[-1]

    now = datetime.now().astimezone()
    meetings = sorted((m for m in (data or {}).get("meetings", []) if parse(m["end"]) > now - timedelta(hours=1)),
                      key=lambda m: m["start"])
    head = line().add(f"{ICON['calendar']}  ", ORANGE).add("Meetings", BOLD + ORANGE)
    if data and data.get("at"):
        head.add(f"   updated {datetime.fromtimestamp(data['at'] / 1000).strftime('%H:%M')}", DIM)
    elif not data or not data.get("at"):
        head.add("   waiting for a Claude session with the Google Calendar connector", DIM)
    if data and data.get("error"):
        line().add(data["error"], RED)
    line()

    upcoming = [m for m in meetings if parse(m["end"]) > now]
    if upcoming:
        first = upcoming[0]
        start = parse(first["start"])
        banner = line()
        if start <= now:
            banner.add(f"  {ICON['now']} now  ", BOLD + ORANGE).add(first["title"], BOLD)
            banner.add(f"   until {parse(first['end']).strftime('%H:%M')}", DIM)
        else:
            when = until_text(start, now) if start.date() == now.date() else start.strftime("%A %H:%M")
            banner.add("  next  ", DIM).add(when, BOLD + ORANGE).add("  ").add(first["title"], BOLD)
        if first.get("join"):
            banner.buttons = [("join", lambda m=first: open_url(state, m["join"]), True)]
            banner.primary = lambda m=first: open_url(state, m["join"])
        line()
    elif data:
        line().add("  Nothing left this week.", DIM)

    title_room = max(16, cols - 4 - TIME - DURATION - PEOPLE - ACTIONS)
    day = None
    for m in meetings:
        start, end = parse(m["start"]), parse(m["end"])
        if start.date() != day:
            day = start.date()
            label = "Today" if day == now.date() else "Tomorrow" if day == (now + timedelta(days=1)).date() else start.strftime("%A %d %B")
            if lines and lines[-1].parts:
                line()
            line().add(f"  {label}", BOLD)
        is_past = end <= now
        is_now = start <= now < end
        row = line()
        row.add("  ")
        glyph = ICON["now"] if is_now else ICON.get(m["response"], ICON["accepted"])
        glyph_style = ORANGE if is_now else GREEN if m["response"] == "accepted" else DIM
        row.add(glyph + " ", glyph_style)
        row.add(fit(f"{start.strftime('%H:%M')}–{end.strftime('%H:%M')}", TIME), DIM if is_past else "")
        minutes = int((end - start).total_seconds() // 60)
        row.add(fit(f"{minutes} min" if minutes < 90 else f"{minutes / 60:.1f} h", DURATION), DIM)
        row.add(fit(m["title"], title_room), DIM if is_past else BOLD if is_now else "", lambda m=m: open_url(state, m["url"]))
        row.add(fit(f"{ICON['people']} {m['attendees']}" if m["attendees"] > 1 else "", PEOPLE), DIM)
        if is_now:
            row.background = NOW_BG
        row.meeting = m["id"]
        brief = read_brief(m)
        is_preparing = os.path.exists(brief_path(m, ".pending"))
        row.buttons = ([("join", lambda m=m: open_url(state, m["join"]), True)] if m.get("join") else []) + [
            ("open", lambda m=m: open_url(state, m["url"]), not m.get("join"))]
        if brief is not None:
            row.buttons.insert(0, ("brief", lambda m=m: toggle_brief(state, m), False))
            row.buttons.append(("redo", lambda m=m: prepare(state, m), False))
        elif m.get("briefable") and not is_past:
            row.buttons.append(("prepare", lambda m=m: prepare(state, m), False))
        row.primary = row.buttons[0][1]
        if is_preparing:
            line().add(" " * 6).add("preparing the brief…", DIM + ORANGE)
        if brief is not None and m["id"] in state["open"]:
            for text in brief:
                if not text.strip():
                    continue
                is_heading = text.startswith("#")
                shown = text.lstrip("# ").strip() if is_heading else text
                line().add(" " * 6).add(shown, BOLD if is_heading else "")
            line()

    line()
    if state["note"] and time.time() - state["note_at"] < 6:
        line().add(state["note"], ORANGE)
    else:
        line().add("click a meeting to open it · j/k rows · h/l or tab buttons · enter press · join opens the video call", DIM)
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
        state["top"] = next((max(0, i - 2) for i, l in enumerate(lines) if l.meeting == target), state["top"])
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
    state = {"data": load(), "sel": 0, "top": 0, "hover": -1, "btn": -1, "has_keyboard": False,
             "note": "", "note_at": 0, "lines": [], "selectable": [], "open": set(), "focus_at": time.time()}
    report(state["data"])
    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    sys.stdout.write("\033[?1049h\033[?25l\033[?1003h\033[?1006h\033]2;meetings\007")
    try:
        tty.setraw(fd)
        seen, last_size, last_minute = None, None, None
        while True:
            mtime = os.path.getmtime(CACHE) if os.path.exists(CACHE) else 0
            # A brief landing or starting redraws too.
            briefs = os.path.getmtime(BRIEFS) if os.path.exists(BRIEFS) else 0
            mtime = (mtime, briefs)
            minute = int(time.time() // 60)
            if take_focus(state):
                draw(state)
            if (mtime, size(), minute) != (seen, last_size, last_minute):
                seen, last_size, last_minute = mtime, size(), minute
                state["data"] = load()
                report(state["data"])
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
