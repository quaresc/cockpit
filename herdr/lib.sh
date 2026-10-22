#!/bin/sh
# Settings shared by the scripts, from the cockpit config that /cockpit:setup writes.
H="${HERDR_BIN_PATH:-herdr}"
ROOT=$(cd "$(dirname "$0")" && pwd -P)
# herdr installs each update in a new folder: keys and space loops go through this link, kept on the current one.
LINK="$HOME/.config/cockpit/herdr"
mkdir -p "$(dirname "$LINK")" && [ "$(readlink "$LINK")" = "$ROOT" ] || ln -sfn "$ROOT" "$LINK"
CONFIG="$HOME/.config/cockpit/config.json"
REPOS=$(jq -r '.reposDir // empty' "$CONFIG" 2>/dev/null | sed "s|^~|$HOME|")
[ -d "$REPOS" ] || REPOS=$HOME
SPACES=$(jq -r '(.spaces // ["inbox", "terminal", "to-do", "runbook", "meetings"])[]' "$CONFIG" 2>/dev/null) ||
  SPACES=$(printf 'inbox\nterminal\nto-do\nrunbook\nmeetings')
