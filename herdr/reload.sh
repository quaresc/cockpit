#!/bin/sh
# Hands /reload-plugins to every Claude session running an older cockpit than the one installed, in herdr and in the
# Herdr app. A session at its prompt takes it at once; a busy one takes it when it is next free, which is why this
# exists: each stale session otherwise waits for someone to type the command into it by hand.
#
#   sh reload.sh            every session that is behind
#   sh reload.sh --idle     only the ones waiting at their prompt (what an update does, mid-update)
set -eu

root=$(cd "$(dirname "$0")" && pwd)
H="${HERDR_BIN_PATH:-herdr}"
HC="${HC_BIN_PATH:-$HOME/.local/bin/hc}"
app_home=${HERDR_APP_HOME:-$HOME/.config/herdr-app}
idle_only=${1:-}

want=$(jq -r .version "$(dirname "$root")/plugin/.claude-plugin/plugin.json" 2>/dev/null) || want=""
[ -z "$want" ] && want=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/herdr-plugin.toml")

# The panes of sessions whose heartbeat says they run something else; with no heartbeat, every Claude pane.
behind=$(
  for beat in "$HOME/.cache/cockpit/sessions"/*.json; do
    [ -f "$beat" ] || continue
    jq -r --arg want "$want" 'select((.version // "") != $want) | .pane // empty' "$beat" 2>/dev/null
  done
)

sent=0
if command -v "$H" >/dev/null 2>&1; then
  filter='.result.panes[] | select(.agent == "claude")'
  [ "$idle_only" = "--idle" ] && filter="$filter | select(.agent_status == \"idle\" or .agent_status == \"done\")"
  for pane in $("$H" pane list 2>/dev/null | jq -r "$filter | .pane_id"); do
    case " $behind " in
      *" $pane "*) "$H" agent prompt "$pane" "/reload-plugins" >/dev/null 2>&1 && sent=$((sent + 1)) ;;
    esac
  done
fi
if command -v herdr-app >/dev/null 2>&1 && [ -S "$app_home/herdr-app.sock" ]; then
  terminals=$(herdr-app ls 2>/dev/null | jq -r '.spaces[].tabs[].terminals[] | select(.status == "idle" or .status == "done") | .terminal')
  for terminal in $terminals; do
    case "$("$HC" get "$terminal" --json 2>/dev/null | jq -r '.foreground.name // ""')" in
      # Sent from a terminal, hc tags the text with its sender, and a tagged /reload-plugins is no command.
      *claude*) env -u HC_TERMINAL_ID "$HC" send "$terminal" "/reload-plugins" >/dev/null 2>&1 && sent=$((sent + 1)) ;;
    esac
  done
fi
echo "cockpit: /reload-plugins sent to $sent session(s); they pick up $want when they are next free"
