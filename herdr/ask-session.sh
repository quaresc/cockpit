#!/bin/sh
# Sends a prompt to a Claude conversation: in its pane when Claude runs there, else resumed first,
# in that pane or in a new space when the pane is gone. Focuses the space so the answer is in view.
#   ask-session.sh <pane or -> <session or -> <prompt>
. "$(dirname "$0")/lib.sh"
pane=$1 session=$2 prompt=$3
state() { "$H" pane get "$pane" 2>/dev/null | jq -r '.result.pane | if .agent then .agent_status else "shell" end' 2>/dev/null; }

st=$(state)
if [ -z "$st" ]; then
  pane=$("$H" workspace create --cwd "$REPOS" --label runbook --focus | jq -r '.result.root_pane.pane_id')
  st=shell
fi
if [ "$st" = "shell" ]; then
  if [ "$session" != "-" ]; then
    sleep 1
    "$H" pane run "$pane" "claude --resume $session" >/dev/null
  else
    "$H" pane run "$pane" "claude" >/dev/null
  fi
fi
i=0
while :; do
  case "$(state)" in idle | done) break ;; esac
  i=$((i + 1))
  [ $i -gt 450 ] && exit 1
  sleep 2
done
ws=$("$H" pane get "$pane" | jq -r '.result.pane.workspace_id')
"$H" workspace focus "$ws" >/dev/null 2>&1
"$H" agent prompt "$pane" "$prompt" >/dev/null
