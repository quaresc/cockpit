#!/bin/sh
# Keeps the pinned spaces on top of the sidebar, in the configured order, agents after.
# One listing per event: the moves are worked out on it, and none is sent when the order already holds.
. "$(dirname "$0")/lib.sh"
"$H" workspace list | SPACES="$SPACES" python3 -c '
import json, os, socket, sys
listed = json.load(sys.stdin)["result"]["workspaces"]
order = [w["workspace_id"] for w in listed]
label = {w["workspace_id"]: w["label"] for w in listed}
for i, name in enumerate(os.environ["SPACES"].splitlines()):
    found = next((space for space in order if label[space].endswith("  " + name)), None)
    if found is None or order.index(found) == i:
        continue
    order.remove(found)
    order.insert(i, found)
    s = socket.socket(socket.AF_UNIX)
    s.connect(os.environ["HERDR_SOCKET_PATH"])
    s.sendall((json.dumps({"id": "cockpit:pin", "method": "workspace.move", "params": {"workspace_id": found, "insert_index": i}}) + "\n").encode())
    s.recv(65536)
    s.close()
'
