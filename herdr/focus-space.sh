#!/bin/sh
# Focuses the pinned space named $1 (e.g. inbox, terminal), labelled "<icon>  <name>". When there is no such space the
# key would otherwise do nothing at all, so it says why: a feature left out of the setup has no space.
H="${HERDR_BIN_PATH:-herdr}"
dir=$(cd "$(dirname "$0")" && pwd)
id=$("$H" workspace list | jq -r --arg m "$1" '[.result.workspaces[] | select(.label | endswith("  " + $m))][0].workspace_id // empty')
if [ -z "$id" ]; then
  sh "$dir/cockpit.sh" notify "No $1 space" "This key opens the $1 space, which is not pinned. cockpit setup adds it." >/dev/null 2>&1
  exit 0
fi
"$H" workspace focus "$id" >/dev/null
