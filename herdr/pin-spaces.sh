#!/bin/sh
# Keeps the pinned spaces on top of the sidebar, in the configured order, agents after.
# One listing per event: the moves are worked out on it, and none is sent when the order already holds. herdr's CLI has
# no move, so each goes to its socket.
. "$(dirname "$0")/lib.sh"
"$H" workspace list | jq -c --arg spaces "$SPACES" '
  .result.workspaces as $listed
  | ($listed | map({key: .workspace_id, value: (.label // "")}) | from_entries) as $label
  | reduce ($spaces | split("\n") | to_entries[]) as $space ({order: [$listed[].workspace_id], moves: []};
      ([.order[] | select($label[.] | endswith("  " + $space.value))][0]) as $found
      | if $found == null or (.order | index($found)) == $space.key then .
        else .order = ((.order - [$found]) | .[:$space.key] + [$found] + .[$space.key:])
          | .moves += [{id: "cockpit:pin", method: "workspace.move", params: {workspace_id: $found, insert_index: $space.key}}]
        end)
  | .moves[]' 2>/dev/null |
  while IFS= read -r move; do
    printf '%s\n' "$move" | nc -w 2 -U "$HERDR_SOCKET_PATH" >/dev/null
  done
