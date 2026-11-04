#!/bin/sh
# Updates both halves of cockpit, then restarts what runs them: the space apps, the Herdr app's layout and mod, and idle
# Claude sessions reload the plugin. Without herdr it updates the Herdr app half alone, from install.sh's copy or the Claude plugin's.
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$HOME/.claude/local:$PATH"
. "$(dirname "$0")/lib.sh"
# An install made by install.sh, outside herdr and Claude Code, updates by running it again.
if [ -z "$REGISTERED" ] && [ -f "$(dirname "$ROOT")/.cockpit-install" ]; then
  COCKPIT_NO_SETUP=1 COCKPIT_HOME="$(dirname "$ROOT")" sh "$(dirname "$ROOT")/install.sh" || exit 1
fi
if command -v claude >/dev/null 2>&1; then
  claude plugin marketplace update cockpit
  claude plugin update cockpit@cockpit
fi
app_home=${HERDR_APP_HOME:-$HOME/.config/herdr-app}
HC=${COCKPIT_HC_BIN:-hc}
if [ -n "$REGISTERED" ]; then
  "$H" plugin update cockpit --yes
  root=$(jq -r '.[] | select(.plugin_id == "cockpit") | .plugin_root' "$HOME/.config/herdr/plugins.json")
else
  root=$ROOT
fi
sh "$root/fetch-apps.sh"

# The spaces' supervisors and their apps stop, then ensure-spaces (from the new folder) moves the link, prunes old folders
# and starts them again.
pkill -f "(^|/)cockpit space (inbox|todo|runbook|meetings|monitor|routines)$"
pkill -f "cockpit (inbox|todo|runbook|meetings|monitor|routines) cockpit-space"
sleep 1
if [ -n "$REGISTERED" ]; then
  sh "$root/ensure-spaces.sh"
  sh "$root/pin-spaces.sh"
fi
# The hub serves the old folder: it goes once the link names the new one, so the first app that needs it (a Herdr window
# reconnects at once) starts the new binary.
pkill -f "(^|/)cockpit hub$"

# The Herdr app takes the new layout and mod (it reloads a mod on its own), when it has them.
[ -f "$app_home/layouts/cockpit.json" ] && sh "$root/cockpit.sh" app install >/dev/null

# A Claude session waiting at its prompt reloads the plugin; a busy one keeps the old code until its next restart.
if [ -n "$REGISTERED" ]; then
  "$H" pane list | jq -r '.result.panes[] | select(.agent == "claude" and (.agent_status == "idle" or .agent_status == "done")) | .pane_id' |
    while read -r pane; do "$H" agent prompt "$pane" "/reload-plugins" >/dev/null 2>&1; done
fi
if command -v herdr-app >/dev/null 2>&1 && [ -S "$app_home/herdr-app.sock" ]; then
  herdr-app ls 2>/dev/null | jq -r '.spaces[].tabs[].terminals[] | select(.status == "idle" or .status == "done") | .terminal' |
    while read -r terminal; do
      case "$("$HC" get "$terminal" --json 2>/dev/null | jq -r '.foreground.name // ""')" in
        # Sent from a terminal, hc tags the text with its sender, and a tagged /reload-plugins is no command.
        *claude*) env -u HC_TERMINAL_ID "$HC" send "$terminal" "/reload-plugins" >/dev/null 2>&1 ;;
      esac
    done
fi

version=$(jq -r .version "$(dirname "$root")/plugin/.claude-plugin/plugin.json" 2>/dev/null) ||
  version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/herdr-plugin.toml")
sh "$root/cockpit.sh" notify "Cockpit updated" "Now on ${version:-the latest version}; idle sessions reloaded the mod." "space:inbox"
