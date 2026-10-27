#!/bin/sh
# An agent waiting on a permission or an answer out of view: a notification whose click leads to its space, or to its
# tab in a pinned space.
. "$(dirname "$0")/lib.sh"
event=$(printf '%s' "${HERDR_PLUGIN_EVENT_JSON:-}" | jq -c '.data // .' 2>/dev/null)
[ "$(printf '%s' "$event" | jq -r '.agent_status // empty')" = "blocked" ] || exit 0
pane=$(printf '%s' "$event" | jq -r '.pane_id // empty')
[ -n "$pane" ] || exit 0
row=$(place "$pane") || exit 0
IFS="$(printf '\t')" read -r label target focused <<ROW
$row
ROW
# In view means its space or tab is focused and the terminal hosting herdr is the app in front (macOS can tell).
front() {
  [ "$OS" = Darwin ] || return 0
  app=$(cat "$HOME/.cache/cockpit/terminal-app" 2>/dev/null) || return 0
  now=$(osascript -e 'tell application "System Events" to get bundle identifier of first application process whose frontmost is true' 2>/dev/null)
  # Without the automation permission nothing is known: count it as in front.
  [ -z "$now" ] || [ "$now" = "$app" ]
}
[ "$focused" = "true" ] && front && exit 0
# One notification per pane a minute: an agent flapping between blocked and working does not repeat it.
stamp="$HOME/.cache/cockpit/blocked/$pane"
mkdir -p "$(dirname "$stamp")"
[ -n "$(find "$stamp" -mmin -1 2>/dev/null)" ] && exit 0
touch "$stamp"
agent=$(printf '%s' "$event" | jq -r '.display_agent // .agent // "The agent"')
sh "$ROOT/notify.sh" "⏸ $label" "$agent is waiting for you" "$target"
