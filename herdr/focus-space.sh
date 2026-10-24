#!/bin/sh
# Focuses the pinned space named $1 (e.g. inbox, terminal), labelled "<icon>  <name>".
H="${HERDR_BIN_PATH:-herdr}"
id=$("$H" workspace list | jq -r --arg m "$1" '[.result.workspaces[] | select(.label | endswith("  " + $m))][0].workspace_id // empty')
[ -n "$id" ] && "$H" workspace focus "$id" >/dev/null
