#!/bin/sh
# The recap popup: what a tab's agent did since you left it. q closes it.
. "$(dirname "$0")/lib.sh"
tab=${RECAP_TAB:?} since=${RECAP_SINCE:?}
log="$HOME/.cache/cockpit/activity/$tab.jsonl"
agent=$("$H" pane list | jq -c --arg t "$tab" '[.result.panes[] | select(.tab_id == $t and .agent != null)][0] // empty')
label=$(place "$(printf '%s' "$agent" | jq -r '.pane_id // empty')" | cut -f1)
[ -n "$label" ] || label=$("$H" tab get "$tab" | jq -r '.result.tab.label')
status=$(printf '%s' "$agent" | jq -r '.agent_status // empty')
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
