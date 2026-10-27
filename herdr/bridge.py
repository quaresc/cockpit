"""Runs one of the text apps' actions for the Rust apps, so every action stays written once, in Python.

A JSON request on stdin, {module, function, args, armed}, calls `module.function(state, *args)`; the note it left, the
requests it filed and the refreshes it started come back as JSON on stdout. {"$book": slug} stands for that runbook,
{"$handoff": id} for that handoff, {"$inbox": true} for the inbox cache.
"""

import importlib
import json
import os
import sys
import time

import view


def resolve(arg, module):
    if isinstance(arg, dict) and "$book" in arg:
        return next((b for b in module.read_books() if b["slug"] == arg["$book"]), None)
    if isinstance(arg, dict) and "$handoff" in arg:
        return module.read_handoffs().get(arg["$handoff"])
    if isinstance(arg, dict) and arg.get("$inbox"):
        return view.load_json(module.INBOX, {})
    return arg


def main():
    request = json.load(sys.stdin)
    module = importlib.import_module(request["module"])
    # The same runbooks folder as the app that asks, so a demo copy never reaches the real runbooks.
    if os.environ.get("COCKPIT_RUNBOOKS_DIR") and hasattr(module, "read_books"):
        module.DIR = os.environ["COCKPIT_RUNBOOKS_DIR"]
    state = view.new_state(open=set(), work_mode="feature", choice=None, pending={}, outcome={}, seen=None, books=[], toggled=set(),
                           refreshing={}, refresh_started={}, flash={}, newly={}, announced=set(), focus_at=time.time(), reasons={},
                           was_busy=False, handoffs={})
    # The dialog was the first press of an action that wants two.
    if request.get("armed"):
        state["confirm"] = (request["armed"], time.time())
    try:
        if hasattr(module, "read_handoffs"):
            state["handoffs"] = module.read_handoffs()
        if hasattr(module, "read_books"):
            state["books"] = module.read_books()
        getattr(module, request["function"])(state, *[resolve(a, module) for a in request.get("args", [])])
        error = None
    except Exception as err:  # the Rust app shows it as a toast rather than crashing
        error = f"{request['function']}: {err}"
    print(json.dumps({"note": state["note"], "pending": state["pending"], "error": error,
                      "refreshing": {slug: proc.pid for slug, proc in state["refreshing"].items()}}, default=str))


if __name__ == "__main__":
    main()
