#!/usr/bin/env python3
"""A to-do list for the herdr to-do space, kept in ~/.config/cockpit/todo.md.

The file is plain Markdown checkboxes, so Claude (or any editor) can add to it too; the app reloads
it when it changes. Ticket keys and URLs in an item are clickable. A row shows its buttons only while
hovered or selected, like the inbox.
"""

import os
import re
import select
import subprocess
import sys
import termios
import time
import tty
import unicodedata
from datetime import date

import cockpit_config

HOME = os.path.expanduser("~")
TODO = os.path.join(HOME, ".config", "cockpit", "todo.md")
HERDR = os.environ.get("HERDR_BIN_PATH", "herdr")
TEAMS = cockpit_config.team_keys(cockpit_config.load())

DIM, BOLD, RESET, STRIKE = "\033[2m", "\033[1m", "\033[0m", "\033[9m"
GREEN, RED, ORANGE, LINK = "\033[38;2;153;255;228m", "\033[38;2;255;128;128m", "\033[38;2;255;199;153m", "\033[4m"
HOVER = "\033[48;2;30;30;34m"
PILL_MAIN = ("\033[38;2;255;199;153m", "\033[48;2;255;199;153m\033[38;2;16;16;16m")
PILL = ("\033[38;2;44;44;48m", "\033[48;2;44;44;48m\033[38;2;210;210;210m")
PILL_FOCUS = ("\033[38;2;245;245;245m", "\033[48;2;245;245;245m\033[38;2;16;16;16m\033[1m")
ICON = {
    "todo": "", "open": "", "done": "", "add": "", "edit": "",
    "delete": "", "undo": "", "up": "", "down": "", "fold": "", "unfold": "",
}
ACTIONS = 50
TOKEN = re.compile(r"https?://\S+" + (r"|\b(?:%s)-\d+\b" % "|".join(TEAMS) if TEAMS else ""), re.I)
LINE = re.compile(r"^- \[( |x)\] (.*)$")


def width(text):
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 0 if unicodedata.combining(c) else 1 for c in text)


def fit(text, cells):
    if width(text) <= cells:
        return text + " " * (cells - width(text))
    out = ""
    for c in text:
        if width(out + c) > cells - 1:
            break
        out += c
    return out + "…" + " " * (cells - 1 - width(out))


# ---------------------------------------------------------------------------
# The file: one "- [ ] text" or "- [x] text" line per item, open ones first


def read_items():
    try:
        with open(TODO, encoding="utf-8") as handle:
            lines = handle.read().splitlines()
    except OSError:
        return []
    items = []
    for raw in lines:
        match = LINE.match(raw.strip())
        if match:
            items.append({"done": match[1] == "x", "text": match[2]})
    return items


def write_items(items):
    os.makedirs(os.path.dirname(TODO), exist_ok=True)
    ordered = [i for i in items if not i["done"]] + [i for i in items if i["done"]]
    body = "\n".join(f"- [{'x' if i['done'] else ' '}] {i['text']}" for i in ordered)
    with open(TODO + ".tmp", "w", encoding="utf-8") as handle:
        handle.write("# To-do\n\n" + body + "\n")
    os.rename(TODO + ".tmp", TODO)


def report(items):
    """The count on the to-do space's sidebar row."""
    open_count = sum(1 for i in items if not i["done"])
    workspace = os.environ.get("HERDR_WORKSPACE_ID")
    if not workspace:
        return
    value = f"{ICON['open']}  {open_count} open" if open_count else f"{ICON['done']}  all done"
    subprocess.run([HERDR, "workspace", "report-metadata", workspace, "--source", "cockpit-todo", "--token", f"todo={value}"],
                   capture_output=True, timeout=5)


# ---------------------------------------------------------------------------
# Layout


class Line:
    def __init__(self):
        self.parts, self.spans, self.buttons, self.button_spans = [], [], [], []
        self.primary = None

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
            for n, (label, fn, _, is_main) in enumerate(self.buttons):
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
        if is_active and self.primary:
            out = HOVER + out.replace(RESET, RESET + HOVER) + RESET
        return out


def open_link(state, token):
    url = token if token.lower().startswith("http") else cockpit_config.ticket_url(token)
    if not url:
        note(state, f"no Linear workspace known for {token.upper()}")
        return
    subprocess.run(["open", url], capture_output=True)
    note(state, f"opened {url}")


def note(state, text):
    state["note"], state["note_at"] = text, time.time()


def item_text(line, state, text, style):
    """The item's text, with its ticket keys and URLs clickable."""
    position = 0
    for match in TOKEN.finditer(text):
        line.add(text[position:match.start()], style)
        token = match[0]
        line.add(token, style + LINK + ORANGE, lambda token=token: open_link(state, token))
        position = match.end()
    line.add(text[position:], style)


def build(state, cols):
    items = state["items"]
    lines = []

    def line():
        lines.append(Line())
        return lines[-1]

    open_items = [i for i in items if not i["done"]]
    done_items = [i for i in items if i["done"]]
    line().add(f"{ICON['todo']}  ", ORANGE).add("To-do", BOLD + ORANGE).add(f"   {len(open_items)} open", DIM)
    line()

    adder = line()
    if state["input"] is not None and state["editing"] is None:
        adder.add(f"  {ICON['add']}  ", ORANGE).add(state["input"], "").add("▏", ORANGE)
    else:
        adder.add(f"  {ICON['add']}  add a to-do", DIM, lambda: start_input(state))
        adder.primary = lambda: start_input(state)
    line()

    def item_row(item):
        row = line()
        if state["editing"] is item:
            row.add(f"  {ICON['edit']}  ", ORANGE).add(state["input"], "").add("▏", ORANGE)
            return
        glyph, glyph_style = (ICON["done"], GREEN) if item["done"] else (ICON["open"], ORANGE)
        row.add("  ").add(glyph, glyph_style, lambda: toggle(state, item)).add("  ")
        item_text(row, state, item["text"], DIM + STRIKE if item["done"] else "")
        row.buttons = [
            ("undo" if item["done"] else "done", lambda: toggle(state, item), "x", True),
            ("edit", lambda: start_input(state, item), "e", False),
        ]
        if not item["done"]:
            row.buttons += [("up", lambda: move(state, item, -1), "K", False), ("down", lambda: move(state, item, 1), "J", False)]
        row.buttons.append(("delete", lambda: delete(state, item), "d", False))
        row.primary = lambda: toggle(state, item)

    for item in open_items:
        item_row(item)
    if not open_items:
        line().add("  nothing left, nice", DIM)

    if done_items:
        line()
        is_open = state["show_done"]
        fold = line().add(f"  {ICON['unfold'] if is_open else ICON['fold']} {len(done_items)} done", DIM,
                          lambda: state.update(show_done=not state["show_done"]))
        fold.primary = fold.spans[0][2]
        if is_open:
            for item in done_items:
                item_row(item)

    line()
    if state["note"] and time.time() - state["note_at"] < 6:
        line().add(state["note"], ORANGE)
    elif state["input"] is not None:
        line().add("enter save · esc cancel", DIM)
    else:
        line().add("a add · space/x done · h/l or tab buttons · enter press · e edit · J/K move · d delete · ticket keys and links open", DIM)
    return lines


# ---------------------------------------------------------------------------
# Edits


def save(state):
    write_items(state["items"])
    state["mtime"] = os.path.getmtime(TODO)
    report(state["items"])


def start_input(state, item=None):
    state["editing"] = item
    state["input"] = item["text"] if item else ""
    state["has_keyboard"] = True


def commit_input(state):
    text = state["input"].strip()
    if state["editing"] is not None:
        if text:
            state["editing"]["text"] = text
    elif text:
        state["items"].insert(0, {"done": False, "text": text})
    state["input"], state["editing"] = None, None
    save(state)


def toggle(state, item):
    item["done"] = not item["done"]
    if item["done"]:
        item["text"] = re.sub(r"\s*\(done \d{4}-\d{2}-\d{2}\)$", "", item["text"]) + f" (done {date.today().isoformat()})"
    else:
        item["text"] = re.sub(r"\s*\(done \d{4}-\d{2}-\d{2}\)$", "", item["text"])
    save(state)


def move(state, item, step):
    open_items = [i for i in state["items"] if not i["done"]]
    index = open_items.index(item)
    target = index + step
    if 0 <= target < len(open_items):
        open_items[index], open_items[target] = open_items[target], open_items[index]
        state["items"] = open_items + [i for i in state["items"] if i["done"]]
        save(state)


def delete(state, item):
    pending = state.get("confirm")
    if pending is item and time.time() - state["confirm_at"] < 5:
        state["items"].remove(item)
        state["confirm"] = None
        save(state)
        note(state, "deleted")
    else:
        state["confirm"], state["confirm_at"] = item, time.time()
        note(state, "click delete again to confirm")


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
    selectable = [i for i, l in enumerate(lines) if l.primary]
    state["selectable"] = selectable
    if selectable:
        state["sel"] = min(state["sel"], len(selectable) - 1)
    active = selectable[state["sel"]] if state["has_keyboard"] and selectable and state["input"] is None else state["hover"]
    state["top"] = max(0, min(state["top"], max(0, len(lines) - rows)))
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


def current_line(state):
    selectable = state["selectable"]
    return state["lines"][selectable[state["sel"]]] if selectable else None


def type_key(state, data):
    if data in ("\r", "\n"):
        commit_input(state)
    elif data == "\033":
        state["input"], state["editing"] = None, None
    elif data in ("\x7f", "\b"):
        state["input"] = state["input"][:-1]
    elif data.startswith("\033"):
        return
    else:
        state["input"] += "".join(c for c in data if c.isprintable())


def key(state, data):
    if state["input"] is not None:
        type_key(state, data)
        return
    selectable, current = state["selectable"], current_line(state)
    buttons = current.buttons if current else []
    state["has_keyboard"] = True
    if data in ("j", "k", "\033[B", "\033[A"):
        state["btn"] = -1
    if data in ("j", "\033[B"):
        state["sel"] = min(state["sel"] + 1, max(0, len(selectable) - 1))
    elif data in ("k", "\033[A"):
        state["sel"] = max(state["sel"] - 1, 0)
    elif data == "a":
        start_input(state)
    # h/l (or the arrows, tab) walk the selected row's buttons; enter presses the focused one.
    elif data in ("l", "\033[C", "\t") and buttons:
        state["btn"] = min(state["btn"] + 1, len(buttons) - 1)
    elif data in ("h", "\033[D", "\033[Z"):
        state["btn"] = max(state["btn"] - 1, -1)
    elif data in ("\r", "\n") and 0 <= state["btn"] < len(buttons):
        buttons[state["btn"]][1]()
    elif data in (" ", "\r", "\n") and current and current.primary:
        current.primary()
    elif current:
        actions = {k: fn for _, fn, k, _ in current.buttons}
        if data in actions:
            actions[data]()


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
    state = {"items": read_items(), "mtime": os.path.getmtime(TODO) if os.path.exists(TODO) else 0, "sel": 0, "top": 0,
             "hover": -1, "btn": -1, "input": None, "editing": None, "show_done": False, "has_keyboard": False, "note": "",
             "note_at": 0, "lines": [], "selectable": [], "confirm": None, "confirm_at": 0}
    report(state["items"])
    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    sys.stdout.write("\033[?1049h\033[?25l\033[?1003h\033[?1006h\033]2;to-do\007")
    try:
        tty.setraw(fd)
        last_size = None
        while True:
            mtime = os.path.getmtime(TODO) if os.path.exists(TODO) else 0
            if mtime != state["mtime"] and state["input"] is None:
                state["items"], state["mtime"] = read_items(), mtime
                report(state["items"])
                draw(state)
            if size() != last_size:
                last_size = size()
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
