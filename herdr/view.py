"""The terminal layer the herdr space apps share: styles, lines with clickable spans and hover buttons, and the
input loop (alternate screen, SGR mouse with motion, j/k rows, h/l buttons, Enter to press).

An app describes its screen as a list of `Line`s from its `build(state, cols)`; `run` owns drawing, scrolling,
selection, hover, button focus and the terminal's restoration on exit.
"""

import json
import os
import re
import select
import signal
import subprocess
import sys
import termios
import time
import tty
import unicodedata
from typing import Callable, NamedTuple, Optional

HERDR = os.environ.get("HERDR_BIN_PATH", "herdr")

# Vesper colours: warm orange accent, teal for good, red for broken, the rest dim.
DIM, BOLD, RESET, STRIKE, LINK = "\033[2m", "\033[1m", "\033[0m", "\033[9m", "\033[4m"
GREEN, RED, ORANGE = "\033[38;2;153;255;228m", "\033[38;2;255;128;128m", "\033[38;2;255;199;153m"
HOVER = "\033[48;2;30;30;34m"
# A pill is its caps, drawn in the fill colour, around its body, drawn on it.
PILL_MAIN = ("\033[38;2;255;199;153m", "\033[48;2;255;199;153m\033[38;2;16;16;16m")
PILL = ("\033[38;2;44;44;48m", "\033[48;2;44;44;48m\033[38;2;210;210;210m")
PILL_FOCUS = ("\033[38;2;245;245;245m", "\033[48;2;245;245;245m\033[38;2;16;16;16m\033[1m")
CAP_OPEN, CAP_CLOSE = "", ""

DOWN, UP, RIGHT, LEFT, PAGE_DOWN, PAGE_UP, BACKTAB = "\033[B", "\033[A", "\033[C", "\033[D", "\033[6~", "\033[5~", "\033[Z"
MOUSE = re.compile(r"\033\[<(\d+);(\d+);(\d+)([Mm])")


# ---------------------------------------------------------------------------
# Text and files


def width(text):
    """Terminal cells: wide characters take two, combining marks and the emoji variation selector none."""
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 0 if unicodedata.combining(c) or c == "️" else 1 for c in text)


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


def load_json(path, default):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError):
        return default


def write_json(path, value):
    """Writes through a temporary file, so a reader never sees half of it."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path + ".tmp", "w", encoding="utf-8") as handle:
        json.dump(value, handle)
    os.replace(path + ".tmp", path)


def report(source, token, value):
    """A token on the app's own space row in the herdr sidebar, when the app runs in its pinned space."""
    workspace = os.environ.get("HERDR_WORKSPACE_ID") if os.environ.get("COCKPIT_SPACE") else None
    if workspace:
        try:
            subprocess.run([HERDR, "workspace", "report-metadata", workspace, "--source", source, "--token", f"{token}={value}"],
                           capture_output=True, timeout=5)
        except (OSError, subprocess.SubprocessError):
            pass


# ---------------------------------------------------------------------------
# State an app shares with the loop


def new_state(**extra):
    """The loop's own keys (selection, scroll, hover, focused button, note, pending confirmation) plus the app's."""
    return {"sel": 0, "top": 0, "hover": -1, "btn": -1, "has_keyboard": False, "lines": [], "selectable": [],
            "note": "", "note_at": 0, "confirm": None, **extra}


def note(state, text):
    state["note"], state["note_at"] = text, time.time()


def noted(state, seconds=6):
    """The note set within the last few seconds, else None."""
    return state["note"] if state["note"] and time.time() - state["note_at"] < seconds else None


CONFIRM_SECONDS = 6


def confirmed(state, key):
    """True on the second press within a few seconds; the first one only arms it."""
    pending = state.get("confirm")
    if pending and pending[0] == key and time.time() - pending[1] < CONFIRM_SECONDS:
        state["confirm"] = None
        return True
    state["confirm"] = (key, time.time())
    return False


def asking(state, key, label):
    """A button's label, "sure?" while it waits for its confirming press."""
    pending = state.get("confirm")
    return "sure?" if pending and pending[0] == key and time.time() - pending[1] < CONFIRM_SECONDS else label


# ---------------------------------------------------------------------------
# Lines


class Button(NamedTuple):
    label: str
    action: Callable[[], object]
    key: Optional[str] = None  # a one-key shortcut while the row is selected
    main: bool = False  # drawn in the accent colour


class Line:
    def __init__(self):
        self.parts = []  # (text, style)
        self.spans = []  # (start, end, action): clickable text
        self.buttons = []  # Button, shown while the row is active
        self.button_spans = []
        self.status = None  # (text, style): a request in flight or its outcome, shown instead of the buttons
        self.primary = None  # what Enter or a click on the row does; a row without one cannot be selected
        self.bg = ""
        self.anchor = None  # a name `state["scroll_to"]` can bring into view

    def add(self, text, style="", action=None):
        start = sum(width(t) for t, _ in self.parts)
        self.parts.append((text, style))
        if action:
            self.spans.append((start, start + width(text), action))
        return self

    def keys(self):
        return {b.key: b.action for b in self.buttons if b.key}

    def render(self, cols, is_active, focus, icons, actions):
        """The row as one string of exactly `cols` cells. With `actions`, the buttons start that many cells from the
        right edge; without, they end at the edge and cover the end of the row."""
        tail = []
        if self.status:
            tail = [(self.status[0], self.status[1], None)]
        elif is_active and self.buttons:
            for n, button in enumerate(self.buttons):
                cap, body = PILL_FOCUS if n == focus else PILL_MAIN if button.main else PILL
                icon = icons.get(button.label, icons.get(button.label.split()[0], icons.get("open", "")))
                tail += [(CAP_OPEN, cap, button.action), (f"{icon} {button.label}", body, button.action),
                         (CAP_CLOSE, cap, button.action), (" ", "", None)]
        tail_width = sum(width(t[0]) for t in tail)
        room_for_parts = cols - tail_width - (2 if tail else 0) if actions is None else cols
        out, used = "", 0
        for text, style in self.parts:
            room = room_for_parts - used
            if room <= 0:
                break
            piece = text if width(text) <= room else fit(text, room)
            out += style + piece + RESET
            used += width(piece)
        if tail:
            if actions is None:
                start = max(used, cols - tail_width)
            else:
                start = min(cols, max(sum(width(t) for t, _ in self.parts) + 1, cols - actions))
            out += " " * max(0, start - used)
            used = max(used, start)
        self.button_spans = []
        for text, style, action in tail:
            room = cols - used
            if room <= 0:
                break
            piece = text if width(text) <= room else fit(text, room)
            out += style + piece + RESET
            if action:
                self.button_spans.append((used, used + width(piece), action))
            used += width(piece)
        out += " " * max(0, cols - used)
        background = HOVER if is_active and self.primary else self.bg
        if background:
            out = background + out.replace(RESET, RESET + background) + RESET
        return out


# ---------------------------------------------------------------------------
# The loop


def size():
    try:
        return os.get_terminal_size()
    except OSError:
        return os.terminal_size((120, 40))


class App:
    """One space app: `build(state, cols)` lays out its lines; the hooks are optional.

    `poll(state)` runs every loop and says whether the screen must be drawn again (files changed, timers);
    `on_key(state, data)` sees keys first and returns True when it used them; `timeout(state)` is how long the loop
    waits for input; `is_typing(state)` turns row selection off while a text field takes the keys.
    """

    def __init__(self, title, state, build, icons, actions=None, poll=None, on_key=None, timeout=None, is_typing=None):
        self.title, self.state, self.build, self.icons, self.actions = title, state, build, icons, actions
        self.poll = poll or (lambda state: False)
        self.on_key = on_key or (lambda state, data: False)
        self.timeout = timeout or (lambda state: 1.0)
        self.is_typing = is_typing or (lambda state: False)

    def active(self):
        state = self.state
        if state["has_keyboard"] and state["selectable"] and not self.is_typing(state):
            return state["selectable"][state["sel"]]
        return state["hover"]

    def current(self):
        state = self.state
        return state["lines"][state["selectable"][state["sel"]]] if state["selectable"] else None

    def draw(self):
        state = self.state
        cols, rows = size()
        lines = self.build(state, cols)
        state["lines"] = lines
        target = state.pop("scroll_to", None)
        if target is not None:
            state["top"] = next((max(0, i - 2) for i, l in enumerate(lines) if l.anchor == target), state["top"])
        selectable = [i for i, l in enumerate(lines) if l.primary]
        state["selectable"] = selectable
        if selectable:
            state["sel"] = min(state["sel"], len(selectable) - 1)
            if state["has_keyboard"]:
                current = selectable[state["sel"]]
                state["top"] = min(max(state["top"], current - rows + 1), current)
        state["top"] = max(0, min(state["top"], max(0, len(lines) - rows)))
        active = self.active()
        out = ["\033[H"]
        for row in range(rows):
            index = state["top"] + row
            line = lines[index] if index < len(lines) else None
            focus = state["btn"] if state["has_keyboard"] and index == active and line else -1
            if line and focus >= len(line.buttons):
                focus = state["btn"] = len(line.buttons) - 1
            text = line.render(cols, index == active, focus, self.icons, self.actions) if line else " " * cols
            out.append(text + ("\r\n" if row < rows - 1 else ""))
        sys.stdout.write("".join(out))
        sys.stdout.flush()

    def key(self, data):
        state = self.state
        if self.on_key(state, data):
            return
        selectable, current = state["selectable"], self.current()
        buttons = current.buttons if current else []
        state["has_keyboard"] = True
        last = max(0, len(selectable) - 1)
        if data in ("j", "k", DOWN, UP, PAGE_DOWN, PAGE_UP):
            state["btn"] = -1
        if data in ("j", DOWN):
            state["sel"] = min(state["sel"] + 1, last)
        elif data in ("k", UP):
            state["sel"] = max(state["sel"] - 1, 0)
        elif data == PAGE_DOWN:
            state["sel"] = min(state["sel"] + 10, last)
        elif data == PAGE_UP:
            state["sel"] = max(state["sel"] - 10, 0)
        elif data in ("l", RIGHT, "\t") and buttons:
            state["btn"] = min(state["btn"] + 1, len(buttons) - 1)
        elif data in ("h", LEFT, BACKTAB):
            state["btn"] = max(state["btn"] - 1, -1)
        elif data in ("\r", "\n", " ") and current:
            if 0 <= state["btn"] < len(buttons):
                buttons[state["btn"]].action()
            elif current.primary:
                current.primary()
        elif current and data in current.keys():
            current.keys()[data]()

    def click(self, x, y):
        state = self.state
        index = state["top"] + y - 1
        if 0 <= index < len(state["lines"]):
            line = state["lines"][index]
            for start, end, action in line.button_spans + line.spans:
                if start <= x - 1 < end:
                    action()
                    return

    def handle(self, data):
        """Returns whether the screen needs drawing again."""
        state = self.state
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
                self.click(x, y)
                changed = True
            position = match.end()
        rest = data[position:] if position else data
        if rest and not rest.startswith("\033[<"):
            self.key(rest)
            changed = True
        return changed

    def run(self):
        state = self.state
        # Stopped by a signal (an update restarting the app), it still restores the terminal on the way out.
        signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
        signal.signal(signal.SIGHUP, lambda *_: sys.exit(0))
        fd = sys.stdin.fileno()
        saved = termios.tcgetattr(fd)
        # 1003 reports every mouse move, which is what shows the hovered row's buttons.
        sys.stdout.write(f"\033[?1049h\033[?25l\033[?1003h\033[?1006h\033]2;{self.title}\007")
        try:
            tty.setraw(fd)
            last_size = None
            while True:
                changed = self.poll(state)
                if size() != last_size:
                    last_size, changed = size(), True
                # A pending "sure?" goes back to its label on screen once it expires.
                if state["confirm"] and time.time() - state["confirm"][1] >= CONFIRM_SECONDS:
                    state["confirm"], changed = None, True
                if changed:
                    self.draw()
                ready, _, _ = select.select([fd], [], [], self.timeout(state))
                if ready:
                    data = os.read(fd, 4096).decode("utf-8", "replace")
                    if data == "\x03":
                        break
                    if self.handle(data):
                        self.draw()
        finally:
            termios.tcsetattr(fd, termios.TCSADRAIN, saved)
            sys.stdout.write("\033[?1003l\033[?1006l\033[?25h\033[?1049l")
            sys.stdout.flush()
