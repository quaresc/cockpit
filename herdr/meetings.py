#!/usr/bin/env python3
"""Upcoming meetings for the herdr meetings space, from ~/.cache/cockpit/meetings.json.

The cockpit mod fills the file from Google Calendar every few minutes. Meetings are grouped by
day, the running one highlighted; a row shows its buttons while hovered or selected, like the inbox.
"""

import os
import re
import subprocess
import time
from datetime import datetime, timedelta

import cockpit_config
import view
from view import BOLD, DIM, GREEN, ORANGE, RED, Button, Line, fit, note

HOME = os.path.expanduser("~")
CONFIG = cockpit_config.load()
CACHE = os.path.join(HOME, ".cache", "cockpit", "meetings.json")
BRIEFS = os.path.join(HOME, ".cache", "cockpit", "meeting-briefs")
FOCUS = os.path.join(HOME, ".cache", "cockpit", "meetings-focus.json")

NOW_BG = "\033[48;2;48;38;30m"
ICON = {"brief": "", "prepare": "", "redo": "", "calendar": "", "join": "\U000f0567", "open": "", "people": "", "now": "",
        "accepted": "", "tentative": "", "needsAction": ""}
TIME, DURATION, PEOPLE, ACTIONS = 14, 8, 6, 26


def load():
    return view.load_json(CACHE, None)


def parse(iso):
    return datetime.fromisoformat(iso).astimezone()


def until_text(start, now):
    minutes = int((start - now).total_seconds() // 60)
    if minutes < 60:
        return f"in {minutes} min"
    return f"in {minutes // 60} h {minutes % 60:02d}" if minutes < 6 * 60 else start.strftime("%H:%M")


_reported = {"value": None}


def report(state):
    """The next meeting on the meetings space's sidebar row."""
    now = datetime.now().astimezone()
    upcoming = [m for m in (state or {}).get("meetings", []) if parse(m["end"]) > now]
    upcoming.sort(key=lambda m: parse(m["start"]))
    if not upcoming:
        value = f"{ICON['calendar']}  no meeting this week"
    else:
        m = upcoming[0]
        start = parse(m["start"])
        when = "now" if start <= now else until_text(start, now) if start.date() == now.date() else start.strftime("%a %H:%M")
        value = f"{ICON['calendar']}  {when} · {m['title'][:20]}"
    if value != _reported["value"]:
        view.report("cockpit-meetings", "meetings", value)
        _reported["value"] = value


# ---------------------------------------------------------------------------
# Layout


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
        note(state, "already preparing that brief")
        return
    brief_script = cockpit_config.script("meeting-brief.sh")
    if not os.path.exists(brief_script):
        note(state, f"cannot write briefs: {brief_script} is missing")
        return
    subprocess.Popen(["sh", brief_script, m["id"]], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    state["open"].add(m["id"])
    note(state, f"preparing the brief of {m['title']}, about a minute")


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
    target = view.load_json(FOCUS, {}).get("id")
    if target is None:
        return False
    state["open"].add(target)
    state["scroll_to"], state["has_keyboard"], state["hover"] = target, False, -1
    return True


def open_url(state, url):
    cockpit_config.open_url(url)
    note(state, f"opened {url}")


def build(state, cols):
    data = state["data"]
    lines = []

    def line():
        lines.append(Line())
        return lines[-1]

    now = datetime.now().astimezone()
    meetings = sorted((m for m in (data or {}).get("meetings", []) if parse(m["end"]) > now - timedelta(hours=1)),
                      key=lambda m: parse(m["start"]))
    head = line().add(f"{ICON['calendar']}  ", ORANGE).add("Meetings", BOLD + ORANGE)
    if data and data.get("at"):
        head.add(f"   updated {datetime.fromtimestamp(data['at'] / 1000).strftime('%H:%M')}", DIM)
    else:
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
            banner.buttons = [Button("join", lambda m=first: open_url(state, m["join"]), main=True)]
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
            row.bg = NOW_BG
        row.anchor = m["id"]
        brief = read_brief(m)
        is_preparing = os.path.exists(brief_path(m, ".pending"))
        row.buttons = ([Button("join", lambda m=m: open_url(state, m["join"]), main=True)] if m.get("join") else []) + [
            Button("open", lambda m=m: open_url(state, m["url"]), main=not m.get("join"))]
        if brief is not None:
            row.buttons.insert(0, Button("brief", lambda m=m: toggle_brief(state, m)))
            row.buttons.append(Button("redo", lambda m=m: prepare(state, m)))
        elif m.get("briefable") and not is_past and CONFIG["meetingBrief"]["enabled"]:
            row.buttons.append(Button("prepare", lambda m=m: prepare(state, m)))
        row.primary = row.buttons[0].action
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
    if view.noted(state):
        line().add(state["note"], ORANGE)
    else:
        line().add("click a meeting to open it · j/k rows · h/l or tab buttons · enter press · join opens the video call", DIM)
    return lines


# ---------------------------------------------------------------------------
# The loop


def poll(state):
    """A notification click, the cache or a brief changing, and every new minute (the "in N min") redraw."""
    focused = take_focus(state)
    mtime = os.path.getmtime(CACHE) if os.path.exists(CACHE) else 0
    briefs = os.path.getmtime(BRIEFS) if os.path.exists(BRIEFS) else 0
    seen = (mtime, briefs, int(time.time() // 60))
    if seen == state["seen"]:
        return focused
    state["seen"], state["data"] = seen, load()
    report(state["data"])
    return True


def main():
    state = view.new_state(data=load(), open=set(), focus_at=time.time(), seen=None)
    report(state["data"])
    view.App("meetings", state, build, ICON, actions=ACTIONS, poll=poll).run()


if __name__ == "__main__":
    main()