#!/bin/sh
# Sends a prompt to an agent's conversation: in its pane when the agent runs there, else resumed first, in that pane
# or, when the pane is gone, in a new tab of the actions space (a new space without one). Focuses its tab so the answer
# is in view. A pane `app:<terminal>` is a Herdr app terminal, driven with herdr-app and hc. The agent is Claude Code
# unless given.
#   ask-session.sh <pane or -> <session or -> <prompt> [agent]
. "$(dirname "$0")/lib.sh"
PATH="$HOME/.local/bin:$PATH"
HC=${COCKPIT_HC_BIN:-hc}
pane=$1 session=$2 prompt=$3 agent=${4:-claude}

# The Herdr app: an agent's status from `herdr-app ls`, "shell" when a shell is in front, nothing once it is gone.
app_state() {
  found=$(herdr-app ls 2>/dev/null | jq -r --arg t "${pane#app:}" '[.spaces[].tabs[].terminals[] | select(.terminal == $t)][0] // empty
    | .status // "none"' 2>/dev/null)
  [ "$found" = none ] || { printf '%s\n' "$found"; return; }
  case "$("$HC" get "${pane#app:}" --json 2>/dev/null | jq -r '.foreground.name // ""')" in
    "" | *sh | nu) echo shell ;;
    *) echo idle ;;
  esac
}
herdr_state() { "$H" pane get "$pane" 2>/dev/null | jq -r '.result.pane | if .agent then .agent_status else "shell" end' 2>/dev/null; }
state() { case "$pane" in app:*) app_state ;; *) herdr_state ;; esac; }
run_in() { case "$pane" in app:*) "$HC" send --force "${pane#app:}" "$1" >/dev/null ;; *) "$H" pane run "$pane" "$1" >/dev/null ;; esac; }

st=$(state)
if [ -z "$st" ]; then
  case "$pane" in app:*) COCKPIT_FRONTEND=app && export COCKPIT_FRONTEND ;; esac
  pane=$(sh "$ROOT/cockpit.sh" actions tab "$REPOS" runbook)
  case "$pane" in
    "") [ "${COCKPIT_FRONTEND:-}" = app ] || pane=$("$H" workspace create --cwd "$REPOS" --label runbook --focus | jq -r '.result.root_pane.pane_id') ;;
  esac
  [ -n "$pane" ] || exit 1
  st=shell
fi
if [ "$st" = "shell" ]; then
  if [ "$session" != "-" ]; then
    sleep 1
    line=$(sh "$ROOT/cockpit.sh" agent argv resume --agent "$agent" --session "$session")
  else
    line=$(sh "$ROOT/cockpit.sh" agent argv start --agent "$agent")
  fi
  [ -n "$line" ] || exit 1
  run_in "$line"
fi
i=0
while :; do
  case "$(state)" in idle | done) break ;; esac
  i=$((i + 1))
  [ $i -gt 450 ] && exit 1
  sleep 2
done
case "$pane" in
  app:*)
    herdr-app "do" cockpit.go:work >/dev/null 2>&1
    herdr-app "do" pane.focus "{\"terminal\": \"${pane#app:}\"}" >/dev/null 2>&1
    "$HC" send "${pane#app:}" "$prompt" >/dev/null
    ;;
  *)
    tab=$("$H" pane get "$pane" | jq -r '.result.pane.tab_id')
    "$H" tab focus "$tab" >/dev/null 2>&1
    "$H" agent prompt "$pane" "$prompt" >/dev/null
    ;;
esac
