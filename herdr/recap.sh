#!/bin/sh
# The recap popup: what a space's agent did since you left it. q closes it.
. "$(dirname "$0")/lib.sh"
ws=${RECAP_SPACE:?} since=${RECAP_SINCE:?}
log="$HOME/.cache/cockpit/activity/$ws.jsonl"
label=$("$H" workspace list | jq -r --arg w "$ws" '.result.workspaces[] | select(.workspace_id == $w) | .label' | sed 's/^\[[0-9]*\] *//')
status=$("$H" pane list | jq -r --arg w "$ws" '[.result.panes[] | select(.workspace_id == $w and .agent != null)][0].agent_status // empty')
mins=$(( ($(date +%s) - since) / 60 ))
B=$(printf '\033[1m') D=$(printf '\033[2m') O=$(printf '\033[38;2;255;199;153m') G=$(printf '\033[38;2;153;255;228m') R=$(printf '\033[0m')
{
  printf '%s%sWhile you were away%s  %s%s · %s min%s\n\n' "$B" "$O" "$R" "$D" "$label" "$mins" "$R"
  jq -s -r --argjson since "$since" --arg g "$G" --arg d "$D" --arg r "$R" '
    [.[] | select(.at / 1000 > $since)] as $e
    | ($e | map(select(.kind == "turn"))) as $turns
    | ($e | map(select(.kind == "edit")) | map(.text) | unique) as $files
    | ($e | map(select(.kind == "command")) | map(.text)) as $commands
    | "\($g)✓ \($turns | length) turn\(if ($turns | length) == 1 then "" else "s" end) finished\($r)",
      ($turns[-3:][] | "  \(.text)"),
      "",
      (if ($files | length) > 0 then "\($d)files changed (\($files | length))\($r)", ($files[:10][] | "  \(.)"), "" else empty end),
      (if ($commands | length) > 0 then "\($d)last commands\($r)", ($commands[-6:][] | "  \(.)") else empty end)
  ' "$log" 2>/dev/null
  case "$status" in
    blocked) printf '\n%sit is waiting for you%s\n' "$O" "$R" ;;
    working) printf '\n%sstill working%s\n' "$D" "$R" ;;
  esac
} | less -R -K -P "q to close"
