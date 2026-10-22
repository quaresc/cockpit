#!/bin/sh
# An agent waiting on a permission or an answer in a space out of view: a notification whose click leads there.
. "$(dirname "$0")/lib.sh"
event=$(printf '%s' "${HERDR_PLUGIN_EVENT_JSON:-}" | jq -c '.data // .' 2>/dev/null)
[ "$(printf '%s' "$event" | jq -r '.agent_status // empty')" = "blocked" ] || exit 0
ws=$(printf '%s' "$event" | jq -r '.workspace_id')
space=$("$H" workspace list | jq -c --arg w "$ws" '.result.workspaces[] | select(.workspace_id == $w)')
[ -n "$space" ] && [ "$(printf '%s' "$space" | jq -r '.focused')" != "true" ] || exit 0
label=$(printf '%s' "$space" | jq -r '.label' | sed 's/^\[[0-9]*\] *//')
agent=$(printf '%s' "$event" | jq -r '.display_agent // .agent // "The agent"')
sh "$ROOT/notify.sh" "⏸ $label" "$agent is waiting for you" "workspace:$ws"
