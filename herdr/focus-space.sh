#!/bin/sh
# Focuses the space whose label contains $1 (e.g. inbox, terminal).
H="${HERDR_BIN_PATH:-herdr}"
id=$("$H" workspace list | jq -r --arg m "$1" '[.result.workspaces[] | select(.label | contains($m))][0].workspace_id // empty')
[ -n "$id" ] && "$H" workspace focus "$id" >/dev/null
