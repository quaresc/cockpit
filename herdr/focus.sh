#!/bin/sh
# What a click on a cockpit notification does, besides opening a URL:
#   workspace:<id>                    the space of that agent
#   tab:<id>                          the tab of that agent, with its space
#   space:<name>                      a pinned space, e.g. space:inbox
#   runbook:<slug>[:<step>,<step>]    the runbook space, that runbook open and its steps played as done
#   meeting:<event id>                the meetings space, that meeting's brief open
#   update                            cockpit updated, in the background
#   act:<review|ci|rerun|plan>:<ref>   a notification button: cockpit act with that action, then the terminal
#   http(s)://…                       opened in the browser
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$HOME/.claude/local:$PATH"
. "$(dirname "$0")/lib.sh"
case "${1:-}" in
  http://* | https://*) open_url "$1"; exit 0 ;;
  meeting:*)
    mkdir -p "$HOME/.cache/cockpit"
    jq -n --arg id "${1#meeting:}" '{id: $id, at: now}' > "$HOME/.cache/cockpit/meetings-focus.json.tmp" &&
      mv "$HOME/.cache/cockpit/meetings-focus.json.tmp" "$HOME/.cache/cockpit/meetings-focus.json"
    sh "$ROOT/focus-space.sh" meetings
    ;;
  update) nohup sh "$ROOT/update.sh" >"$HOME/.cache/cockpit/update.log" 2>&1 & exit 0 ;;
  act:*)
    rest=${1#act:}
    sh "$ROOT/cockpit.sh" act "${rest%%:*}" "${rest#*:}" >"$HOME/.cache/cockpit/act.log" 2>&1
    ;;
  workspace:*) "$H" workspace focus "${1#workspace:}" >/dev/null 2>&1 ;;
  tab:*) "$H" tab focus "${1#tab:}" >/dev/null 2>&1 ;;
  space:*) sh "$ROOT/focus-space.sh" "${1#space:}" ;;
  runbook:*)
    rest=${1#runbook:}
    slug=${rest%%:*}
    steps=$([ "$rest" != "$slug" ] && echo "${rest#*:}")
    mkdir -p "$HOME/.cache/cockpit"
    jq -n --arg slug "$slug" --arg steps "$steps" '{slug: $slug, steps: ($steps | split(",") | map(select(. != ""))), at: now}' \
      > "$HOME/.cache/cockpit/runbook-focus.json.tmp" && mv "$HOME/.cache/cockpit/runbook-focus.json.tmp" "$HOME/.cache/cockpit/runbook-focus.json"
    sh "$ROOT/focus-space.sh" runbook
    ;;
esac
terminal=$(cat "$HOME/.cache/cockpit/terminal-app" 2>/dev/null)
[ "$OS" = Darwin ] && [ -n "$terminal" ] && open -b "$terminal"
