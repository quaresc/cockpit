#!/bin/sh
# An agent waiting on a permission or an answer in a space out of view: a notification whose click leads there.
. "$(dirname "$0")/lib.sh"
event=$(printf '%s' "${HERDR_PLUGIN_EVENT_JSON:-}" | jq -c '.data // .' 2>/dev/null)
[ "$(printf '%s' "$event" | jq -r '.agent_status // empty')" = "blocked" ] || exit 0
ws=$(printf '%s' "$event" | jq -r '.workspace_id')
space=$("$H" workspace list | jq -c --arg w "$ws" '.result.workspaces[] | select(.workspace_id == $w)')
[ -n "$space" ] || exit 0
# In view means its space is focused and the terminal hosting herdr is the app in front (macOS can tell).
front() {
  [ "$OS" = Darwin ] || return 0
  app=$(cat "$HOME/.cache/cockpit/terminal-app" 2>/dev/null) || return 0
  now=$(osascript -e 'tell application "System Events" to get bundle identifier of first application process whose frontmost is true' 2>/dev/null)
  # Without the automation permission nothing is known: count it as in front.
  [ -z "$now" ] || [ "$now" = "$app" ]
}
[ "$(printf '%s' "$space" | jq -r '.focused')" = "true" ] && front && exit 0
# One notification per pane a minute: an agent flapping between blocked and working does not repeat it.
pane=$(printf '%s' "$event" | jq -r '.pane_id // empty')
stamp="$HOME/.cache/cockpit/blocked/${pane:-$ws}"
mkdir -p "$(dirname "$stamp")"
[ -n "$(find "$stamp" -mmin -1 2>/dev/null)" ] && exit 0
touch "$stamp"
label=$(printf '%s' "$space" | jq -r '.label' | sed 's/^\[[0-9]*\] *//')
agent=$(printf '%s' "$event" | jq -r '.display_agent // .agent // "The agent"')
sh "$ROOT/notify.sh" "⏸ $label" "$agent is waiting for you" "workspace:$ws"
