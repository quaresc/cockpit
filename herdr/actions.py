#!/usr/bin/env python3
"""The pinned actions space: a tab per one-off Claude session the inbox or the runbook starts (a review, a plan, a talk
about a ticket), so the sidebar's spaces stay the work in progress.

    actions.py tab <cwd> <label>    a new tab there, in view, its pane id printed; nothing without the space
    actions.py sidebar              the space's sidebar row lists its sessions again
"""

import json
import os
import re
import subprocess
import sys

import cockpit_config
import view
from view import HERDR

# The pane each action runs in, by key such as "review api#4": pressed again, it brings that tab back.
PANES = os.path.join(os.path.expanduser("~"), ".cache", "cockpit", "actions.json")


def herdr(*args):
    """A herdr CLI call's result; None when it fails."""
    try:
        done = subprocess.run([HERDR, *args], capture_output=True, text=True, timeout=15)
        return json.loads(done.stdout)["result"] if done.returncode == 0 else None
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError):
        return None


def space():
    """The actions space's id; None when it is not among the pinned spaces or not open."""
    config = cockpit_config.load()
    if "actions" not in [s["name"] if isinstance(s, dict) else s for s in config["spaces"]]:
        return None
    listed = (herdr("workspace", "list") or {}).get("workspaces") or []
    return next((w["workspace_id"] for w in listed if w["label"].endswith("  actions")), None)


def pane_of(key):
    """The pane where the action's Claude session still runs; None once it has ended or its tab closed."""
    pane = view.load_json(PANES, {}).get(key)
    found = pane and herdr("pane", "get", pane)
    return pane if found and found["pane"].get("agent") else None


def focus(pane):
    """Brings the pane's tab into view, with its space."""
    found = herdr("pane", "get", pane)
    return bool(found) and herdr("tab", "focus", found["pane"]["tab_id"]) is not None


def open_tab(cwd, label, key=None):
    """A new tab in the actions space, in view: its pane id, kept under `key`; None without the space."""
    ws = space()
    created = ws and herdr("tab", "create", "--workspace", ws, "--cwd", cwd, "--label", label, "--focus")
    if not created:
        return None
    pane = created["root_pane"]["pane_id"]
    if key:
        live = {k: p for k, p in view.load_json(PANES, {}).items() if herdr("pane", "get", p)}
        view.write_json(PANES, {**live, key: pane})
    return pane


def promote(key, label):
    """Moves the action's tab out to a space of its own, in view, once it turns into work; False when it no longer runs."""
    pane = pane_of(key)
    if not pane or not herdr("pane", "move", pane, "--new-workspace", "--label", label, "--focus"):
        return False
    view.write_json(PANES, {k: p for k, p in view.load_json(PANES, {}).items() if k != key})
    return True


# The sessions the sidebar lists under the actions space, the last line saying how many more there are.
SLOTS = 6
STATE = {"working": "◐", "blocked": "⏸", "done": "✓", "idle": "○"}


def sidebar_width():
    try:
        with open(os.path.join(os.path.expanduser("~"), ".config", "herdr", "config.toml"), encoding="utf-8") as handle:
            found = re.search(r"^\s*sidebar_width\s*=\s*(\d+)", handle.read(), re.M)
    except OSError:
        found = None
    return int(found.group(1)) if found else 34


def sidebar():
    """Lists the space's Claude sessions under its sidebar row, one line each with its state; a waiting one stands out."""
    ws = space()
    if not ws:
        return
    tabs = [t for t in (herdr("tab", "list") or {}).get("tabs") or [] if t["workspace_id"] == ws and t.get("agent_status") in STATE]
    room = sidebar_width() - 4
    args = ["workspace", "report-metadata", ws, "--source", "cockpit-actions"]
    for i in range(SLOTS):
        tab = tabs[i] if i < len(tabs) else None
        text = tab and view.fit(f"{STATE[tab['agent_status']]} {tab['label']}", room).rstrip()
        if i == SLOTS - 1 and len(tabs) > SLOTS:
            tab, text = None, f"  +{len(tabs) - i} more"
        is_waiting = bool(tab) and tab["agent_status"] == "blocked"
        for name, value in ((f"act_{i + 1}", None if is_waiting else text), (f"act_{i + 1}_wait", text if is_waiting else None)):
            args += ["--token", f"{name}={value}"] if value else ["--clear-token", name]
    herdr(*args)


def main(args):
    if args == ["sidebar"]:
        sidebar()
        return
    if len(args) != 3 or args[0] != "tab":
        sys.exit(__doc__)
    pane = open_tab(args[1], args[2])
    if pane:
        print(pane)


if __name__ == "__main__":
    main(sys.argv[1:])
