#!/bin/sh
# On each focus change: coming back to a space after a while, where its agent finished something, opens the recap popup.
. "$(dirname "$0")/lib.sh"
[ "$(jq -r '.recap.enabled // true' "$CONFIG" 2>/dev/null)" = true ] || exit 0
STATE="$HOME/.cache/cockpit/focus.json"
[ -s "$STATE" ] || echo '{"current": "", "left": {}}' > "$STATE"
ws=$("$H" workspace list | jq -r '.result.workspaces[] | select(.focused) | .workspace_id')
current=$(jq -r '.current' "$STATE")
[ -n "$ws" ] && [ "$ws" != "$current" ] || exit 0
now=$(date +%s)
left=$(jq -r --arg w "$ws" '.left[$w] // 0' "$STATE")
tmp=$(mktemp) && jq --arg c "$current" --arg w "$ws" --argjson now "$now" '.left[$c] = $now | .current = $w' "$STATE" > "$tmp" && mv "$tmp" "$STATE"

away=$(jq -r '.recap.awayMinutes // 10' "$CONFIG" 2>/dev/null)
[ "$left" -gt 0 ] && [ $((now - left)) -ge $((away * 60)) ] || exit 0
log="$HOME/.cache/cockpit/activity/$ws.jsonl"
turns=$(jq -s --argjson since "$left" '[.[] | select(.at / 1000 > $since and .kind == "turn")] | length' "$log" 2>/dev/null)
[ "${turns:-0}" -gt 0 ] || exit 0
"$H" plugin pane open --plugin cockpit --entrypoint recap --placement popup --env "RECAP_SPACE=$ws" --env "RECAP_SINCE=$left" >/dev/null
