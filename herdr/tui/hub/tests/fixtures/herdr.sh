#!/bin/sh
# A fake herdr: logs each call to calls.log and answers plausible JSON; $COCKPIT_TEST_DIR/herdr.json overrides an
# answer by the longest command prefix, {"fail": "message"} making that call fail.
box=$COCKPIT_TEST_DIR
args=""
for arg do args="$args${args:+,}$(jq -n --arg v "$arg" '$v')"; done
printf '{"cmd":"herdr","args":[%s]}\n' "$args" >>"$box/calls.log"
answer=$(jq -c --arg line "$*" '[to_entries[] | select(.key as $k | $line == $k or ($line | startswith($k + " ")))]
  | if length == 0 then empty else max_by(.key | length).value end' "$box/herdr.json" 2>/dev/null)
if [ -n "$answer" ]; then
  if fail=$(printf '%s' "$answer" | jq -er 'if type == "object" and has("fail") then .fail else empty end'); then
    printf '%s\n' "$fail" >&2
    exit 1
  fi
  printf '%s' "$answer" | jq -c '{result: .}'
  exit 0
fi
case "$1 $2" in
  "workspace list")
    jq -cn '{result: {workspaces: [["inbox", "terminal", "actions", "to-do", "runbook", "meetings"] | to_entries[]
      | {workspace_id: "w\(.key + 1)", label: "[\(.key + 1)] x  \(.value)", focused: false}]}}' ;;
  "workspace create" | "tab create" | "worktree create")
    echo '{"result": {"root_pane": {"pane_id": "wT:p1", "tab_id": "wT:t1", "workspace_id": "wT"}}}' ;;
  "pane get")
    jq -cn --arg pane "${3-wT:p1}" '{result: {pane: {pane_id: $pane, tab_id: "wT:t1", workspace_id: "wT"}}}' ;;
  *) echo '{"result": {}}' ;;
esac
