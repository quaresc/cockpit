#!/bin/sh
# Makes sure the pinned spaces exist and run their app, e.g. after herdr restores a session.
. "$(dirname "$0")/lib.sh"
prune_installs

ensure() {
  name=$1 label=$2 app=$3 cwd=${4:-$REPOS}
  listed=$("$H" workspace list)
  ws=$(printf '%s' "$listed" | jq -r --arg n "$name" '[.result.workspaces[] | select(.label | endswith("  " + $n))][0].workspace_id // empty')
  # A space you opened yourself under that name becomes the pinned one, keeping its panes.
  if [ -z "$ws" ]; then
    own=$(printf '%s' "$listed" | jq -r --arg n "$name" '[.result.workspaces[]
      | (.label | capture("^(?<p>\\[[0-9]+\\] )?(?<rest>.*)$")) as $l | select($l.rest == $n) | [.workspace_id, $l.p // ""]][0] // empty | @tsv')
    ws=${own%%"$(printf '\t')"*}
    [ -n "$ws" ] && "$H" workspace rename "$ws" "${own#*"$(printf '\t')"}$label" >/dev/null 2>&1
  fi
  if [ -z "$ws" ]; then
    ws=$("$H" workspace create --cwd "$cwd" --label "$label" --no-focus | jq -r '.result.workspace.workspace_id')
  fi
  [ -n "$app" ] || return 0
  pane=$("$H" pane list --workspace "$ws" | jq -r '.result.panes[0].pane_id // empty')
  [ -n "$pane" ] || return 0
  i=0
  while [ $i -lt 20 ]; do
    fg=$("$H" pane process-info --pane "$pane" | jq -r '.result.process_info.foreground_processes | map(.cmdline) | join(" ")')
    case "$fg" in
      *"$app"*) return 0 ;;
      # A clean line first: an app that died with mouse reporting on leaves mouse codes typed at the prompt.
      *zsh* | *bash* | *fish* | -sh*)
        "$H" pane send-keys "$pane" C-c >/dev/null 2>&1
        "$H" pane run "$pane" "printf '\\033[?1003l\\033[?1006l\\033[?1000l'; clear; sh '$LINK/$app-loop.sh'" >/dev/null
        return 0 ;;
      "") sleep 1 ;;
      *) return 0 ;;
    esac
    i=$((i + 1))
  done
}

printf '%s\n' "$SPACE_ROWS" | while IFS="$(printf '\037')" read -r name icon cwd; do
  [ -n "$name" ] || continue
  cwd=$(printf '%s' "$cwd" | sed "s|^~|$HOME|")
  case "$name" in
    inbox) glyph='\357\200\234' app=inbox ;;
    terminal) glyph='\357\222\211' app="" ;;
    actions) glyph='\357\203\247' app="" ;;
    to-do) glyph='\357\221\236' app=todo ;;
    runbook) glyph='\357\220\205' app=runbook ;;
    meetings) glyph='\357\221\225' app=meetings ;;
    *) glyph='\357\220\223' app="" ;;
  esac
  [ -n "$icon" ] || icon=$(printf "$glyph")
  ensure "$name" "$icon  $name" "$app" "$cwd" </dev/null
done
