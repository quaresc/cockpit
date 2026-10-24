#!/usr/bin/env python3
"""A to-do list for the herdr to-do space, kept in ~/.config/cockpit/todo.md.

The file is plain Markdown checkboxes, so Claude (or any editor) can add to it too; the app reloads
it when it changes. Ticket keys and URLs in an item are clickable. A row shows its buttons only while
hovered or selected, like the inbox.
"""

import os
import re
from datetime import date

import cockpit_config
import view
from view import BOLD, DIM, GREEN, LINK, ORANGE, STRIKE, Button, Line, note

HOME = os.path.expanduser("~")
TODO = os.path.join(HOME, ".config", "cockpit", "todo.md")
CONFIG = cockpit_config.load()
TEAMS = cockpit_config.team_keys(CONFIG)

ICON = {
    "todo": "", "open": "", "done": "", "add": "", "edit": "",
    "delete": "", "undo": "", "up": "", "down": "", "fold": "", "unfold": "",
}
ACTIONS = 50
TOKEN = re.compile(r"https?://\S+" + (r"|\b(?:%s)-\d+\b" % "|".join(TEAMS) if TEAMS else ""), re.I)
LINE = re.compile(r"^- \[( |x)\] (.*)$")


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
    view.report("cockpit-todo", "todo", f"{ICON['open']}  {open_count} open" if open_count else f"{ICON['done']}  all done")


# ---------------------------------------------------------------------------
# Layout


def open_link(state, token):
    url = token if token.lower().startswith("http") else cockpit_config.ticket_url(token, CONFIG)
    if not url:
        note(state, f"no URL known for {token.upper()}: set tickets.urlTemplate in the cockpit config")
        return
    cockpit_config.open_url(url)
    note(state, f"opened {url}")


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
            Button("undo" if item["done"] else "done", lambda: toggle(state, item), "x", True),
            Button("edit", lambda: start_input(state, item), "e"),
        ]
        if not item["done"]:
            row.buttons += [Button("up", lambda: move(state, item, -1), "K"), Button("down", lambda: move(state, item, 1), "J")]
        row.buttons.append(Button("delete", lambda: delete(state, item), "d"))
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
    if view.noted(state):
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
    if view.confirmed(state, f"delete-{id(item)}"):
        state["items"].remove(item)
        save(state)
        note(state, "deleted")
    else:
        note(state, "click delete again to confirm")


# ---------------------------------------------------------------------------
# Keys and the loop


def on_key(state, data):
    """While a to-do is typed, every key goes to it; `a` starts a new one."""
    if state["input"] is not None:
        if data in ("\r", "\n"):
            commit_input(state)
        elif data == "\033":
            state["input"], state["editing"] = None, None
        elif data in ("\x7f", "\b"):
            state["input"] = state["input"][:-1]
        elif not data.startswith("\033"):
            state["input"] += "".join(c for c in data if c.isprintable())
        return True
    if data == "a":
        start_input(state)
        return True
    return False


def poll(state):
    """The file changed outside the app (an editor, /todo): read again, unless a to-do is being typed."""
    mtime = os.path.getmtime(TODO) if os.path.exists(TODO) else 0
    if mtime == state["mtime"] or state["input"] is not None:
        return False
    state["items"], state["mtime"] = read_items(), mtime
    report(state["items"])
    return True


def main():
    state = view.new_state(items=read_items(), mtime=os.path.getmtime(TODO) if os.path.exists(TODO) else 0, input=None,
                           editing=None, show_done=False)
    report(state["items"])
    view.App("to-do", state, build, ICON, actions=ACTIONS, poll=poll, on_key=on_key,
             is_typing=lambda state: state["input"] is not None).run()


if __name__ == "__main__":
    main()