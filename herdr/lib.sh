#!/bin/sh
# Settings shared by the scripts, from the cockpit config that /cockpit:setup writes.
H="${HERDR_BIN_PATH:-herdr}"
ROOT=$(cd "$(dirname "$0")" && pwd)
CONFIG="$HOME/.config/cockpit/config.json"
REPOS=$(jq -r '.reposDir // empty' "$CONFIG" 2>/dev/null | sed "s|^~|$HOME|")
[ -d "$REPOS" ] || REPOS=$HOME
SPACES=$(jq -r '(.spaces // ["inbox", "terminal", "to-do", "runbook", "meetings"])[]' "$CONFIG" 2>/dev/null) ||
  SPACES=$(printf 'inbox\nterminal\nto-do\nrunbook\nmeetings')
