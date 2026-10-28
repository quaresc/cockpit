#!/usr/bin/env python3
"""A fake herdr: logs each call to calls.log and answers plausible JSON; $COCKPIT_TEST_DIR/herdr.json overrides an
answer by the longest command prefix, {"fail": "message"} making that call fail."""
import json
import os
import sys
import time

box = os.environ["COCKPIT_TEST_DIR"]
args = sys.argv[1:]
with open(os.path.join(box, "calls.log"), "a") as log:
    log.write(json.dumps({"at": time.time(), "cmd": "herdr", "args": args}) + "\n")
try:
    with open(os.path.join(box, "herdr.json")) as handle:
        overrides = json.load(handle)
except OSError:
    overrides = {}
line = " ".join(args)
matches = sorted((k for k in overrides if line == k or line.startswith(k + " ")), key=len, reverse=True)
if matches:
    answer = overrides[matches[0]]
    if isinstance(answer, dict) and "fail" in answer:
        sys.stderr.write(answer["fail"] + "\n")
        sys.exit(1)
    print(json.dumps({"result": answer}))
    sys.exit(0)
pair = " ".join(args[:2])
names = ["inbox", "terminal", "actions", "to-do", "runbook", "meetings"]
if pair == "workspace list":
    result = {"workspaces": [{"workspace_id": f"w{i}", "label": f"[{i}] x  {n}", "focused": False} for i, n in enumerate(names, 1)]}
elif pair in ("workspace create", "tab create", "worktree create"):
    result = {"root_pane": {"pane_id": "wT:p1", "tab_id": "wT:t1", "workspace_id": "wT"}}
elif pair == "pane get":
    result = {"pane": {"pane_id": args[2] if len(args) > 2 else "wT:p1", "tab_id": "wT:t1", "workspace_id": "wT"}}
else:
    result = {}
print(json.dumps({"result": result}))
