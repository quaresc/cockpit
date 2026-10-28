#!/usr/bin/env python3
"""A fake tool (claude, open, pbcopy, gh, a herdr script): logs its name, arguments and input to calls.log."""
import json
import os
import sys
import time

# Run as itself by a script, the script's path comes first.
argv = sys.argv[1:] if os.path.basename(sys.argv[0]) == "logcall" else sys.argv
name, args = os.path.basename(argv[0]), argv[1:]
stdin = "" if sys.stdin is None or sys.stdin.isatty() else sys.stdin.read() if name == "pbcopy" else ""
with open(os.path.join(os.environ["COCKPIT_TEST_DIR"], "calls.log"), "a") as log:
    log.write(json.dumps({"at": time.time(), "cmd": name, "args": args, "stdin": stdin}) + "\n")
