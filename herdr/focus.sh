#!/bin/sh
# What a click on a cockpit notification does, besides opening a URL:
#   workspace:<id>                    the space of that agent
#   space:<name>                      a pinned space, e.g. space:inbox
#   runbook:<slug>[:<step>,<step>]    the runbook space, that runbook open and its steps played as done
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
. "$(dirname "$0")/lib.sh"
case "${1:-}" in
  workspace:*) "$H" workspace focus "${1#workspace:}" >/dev/null 2>&1 ;;
  space:*) sh "$ROOT/focus-space.sh" "${1#space:}" ;;
  runbook:*)
    rest=${1#runbook:}
    slug=${rest%%:*}
    steps=$([ "$rest" != "$slug" ] && echo "${rest#*:}")
    mkdir -p "$HOME/.cache/cockpit"
    jq -n --arg slug "$slug" --arg steps "$steps" '{slug: $slug, steps: ($steps | split(",") | map(select(. != ""))), at: now}' \
      > "$HOME/.cache/cockpit/runbook-focus.json"
    sh "$ROOT/focus-space.sh" runbook
    ;;
esac
terminal=$(cat "$HOME/.cache/cockpit/terminal-app" 2>/dev/null)
[ -n "$terminal" ] && open -b "$terminal"
