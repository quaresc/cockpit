#!/bin/sh
# Opens a URL in terminal-browser, split right of a Claude session's pane (a tab of the browser already there), when
# the config's terminalBrowser is on; otherwise, or outside herdr, in the default browser.
#   browse.sh <url> [<Claude session id>]   the session's pane, else the calling pane
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$HOME/.claude/local:$PATH"
. "$(dirname "$0")/lib.sh"
url=$1 session=${2:-}
if [ "$(jq -r '.terminalBrowser // false' "$CONFIG" 2>/dev/null)" = true ] && command -v terminal-browser >/dev/null; then
  panes=$("$H" pane list 2>/dev/null)
  pane=$(printf '%s' "$panes" | jq -r --arg s "$session" 'first(.result.panes[] | select($s != "" and .agent_session.value == $s) | .pane_id) // empty')
  pane=${pane:-${HERDR_PANE_ID:-}}
  tab=$(printf '%s' "$panes" | jq -r --arg p "$pane" 'first(.result.panes[] | select(.pane_id == $p) | .tab_id) // empty')
  if [ -n "$pane" ] && [ -n "$tab" ]; then
    HERDR_PANE_ID=$pane HERDR_TAB_ID=$tab nohup terminal-browser open "$url" --split right >/dev/null 2>&1 </dev/null &
    exit 0
  fi
fi
open_url "$url"
