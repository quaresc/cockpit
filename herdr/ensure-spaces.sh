#!/bin/sh
# Makes sure the pinned spaces exist and run their app, e.g. after herdr restores a session.
. "$(dirname "$0")/lib.sh"

ensure() {
  name=$1 label=$2 app=$3
  ws=$("$H" workspace list | jq -r --arg n "$name" '[.result.workspaces[] | select(.label | contains($n))][0].workspace_id // empty')
  if [ -z "$ws" ]; then
    ws=$("$H" workspace create --cwd "$REPOS" --label "$label" --no-focus | jq -r '.result.workspace.workspace_id')
  fi
  [ -n "$app" ] || return 0
  pane=$("$H" pane list --workspace "$ws" | jq -r '.result.panes[0].pane_id // empty')
  [ -n "$pane" ] || return 0
  i=0
  while [ $i -lt 20 ]; do
    fg=$("$H" pane process-info --pane "$pane" | jq -r '.result.process_info.foreground_processes | map(.cmdline) | join(" ")')
    case "$fg" in
      *"$app"*) return 0 ;;
      *zsh* | *bash* | *fish* | -sh*) "$H" pane run "$pane" "sh '$LINK/$app-loop.sh'" >/dev/null; return 0 ;;
      "") sleep 1 ;;
      *) return 0 ;;
    esac
    i=$((i + 1))
  done
}

for space in $SPACES; do
  case "$space" in
    inbox) ensure inbox "$(printf '\357\200\234')  inbox" inbox ;;
    terminal) ensure terminal "$(printf '\357\222\211')  terminal" "" ;;
    to-do) ensure to-do "$(printf '\357\221\236')  to-do" todo ;;
    runbook) ensure runbook "$(printf '\357\220\205')  runbook" runbook ;;
    meetings) ensure meetings "$(printf '\357\221\225')  meetings" meetings ;;
  esac
done
