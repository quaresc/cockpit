#!/bin/sh
# Updates both halves of cockpit, then restarts what runs them: the space apps, and idle Claude sessions reload the mod.
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$HOME/.claude/local:$PATH"
. "$(dirname "$0")/lib.sh"
claude plugin marketplace update cockpit
claude plugin update cockpit@cockpit
"$H" plugin update cockpit --yes
root=$(jq -r '.[] | select(.plugin_id == "cockpit") | .plugin_root' "$HOME/.config/herdr/plugins.json")

# The spaces' supervisors and their apps stop, then ensure-spaces (from the new folder) moves the link, prunes old folders
# and starts them again.
pkill -f "(^|/)cockpit space (inbox|todo|runbook|meetings)$"
# The apps go too, and the hub, detached from them, which serves the old folder: the first app that needs it starts it again.
pkill -f "cockpit (inbox|todo|runbook|meetings) cockpit-space"
pkill -f "(^|/)cockpit hub$"
sleep 1
sh "$root/ensure-spaces.sh"
sh "$root/pin-spaces.sh"

# A Claude session waiting at its prompt reloads the plugin; a busy one keeps the old code until its next restart.
"$H" pane list | jq -r '.result.panes[] | select(.agent == "claude" and (.agent_status == "idle" or .agent_status == "done")) | .pane_id' |
  while read -r pane; do "$H" agent prompt "$pane" "/reload-plugins" >/dev/null 2>&1; done

version=$(jq -r .version "$(dirname "$root")/plugin/.claude-plugin/plugin.json" 2>/dev/null)
sh "$root/cockpit.sh" notify "Cockpit updated" "Now on ${version:-the latest version}; idle sessions reloaded the mod." "space:inbox"
