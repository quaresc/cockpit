#!/bin/sh
# shellcheck disable=SC2034  # the scripts sourcing this file use its settings
# Settings shared by the scripts, from the cockpit config that /cockpit:setup writes.
H="${HERDR_BIN_PATH:-herdr}"
ROOT=$(cd "$(dirname "$0")" && pwd -P)
OS=$(uname)
# herdr installs each update in a new folder: keys and space loops go through this link, kept on the current one.
# Only the folder herdr's registry names moves it, so a script left in an older folder cannot pull it back.
LINK="$HOME/.config/cockpit/herdr"
REGISTERED=$(jq -r '.[] | select(.plugin_id == "cockpit") | .plugin_root' "$HOME/.config/herdr/plugins.json" 2>/dev/null)
if [ "$REGISTERED" = "$ROOT" ] && [ "$(readlink "$LINK")" != "$ROOT" ]; then
  mkdir -p "$(dirname "$LINK")" && ln -sfn "$ROOT" "$LINK"
fi

# herdr keeps every installed version of a GitHub plugin: all but the registered one go.
prune_installs() {
  case "$ROOT" in */github-installations/*/checkout/herdr) ;; *) return 0 ;; esac
  [ "$REGISTERED" = "$ROOT" ] || return 0
  current=$(dirname "$(dirname "$ROOT")")
  for old in "$(dirname "$current")"/[0-9]*-[0-9]*; do
    [ -d "$old" ] && [ "$old" != "$current" ] && rm -rf "$old"
  done
  return 0
}

open_url() {
  if [ "$OS" = Darwin ]; then open "$1"; else xdg-open "$1" >/dev/null 2>&1; fi
}
CONFIG="$HOME/.config/cockpit/config.json"
# What the scripts read: the config as the mod resolved it, or the raw file before any session has run.
SETTINGS="$HOME/.cache/cockpit/config.json"
[ -f "$SETTINGS" ] || SETTINGS=$CONFIG
REPOS=$(jq -r '.reposDir // empty' "$SETTINGS" 2>/dev/null | sed "s|^~|$HOME|")
[ -d "$REPOS" ] || REPOS=$HOME
# The pinned spaces, one per line as name, icon and cwd split by \037: each is "name" (a built-in, or a space in
# reposDir) or { name, cwd, icon, key }.
SPACE_ROWS=$(jq -r '(.spaces // ["inbox", "terminal", "to-do", "runbook", "meetings"])[]
  | if type == "object" then . else {name: .} end | [.name, .icon // "", .cwd // ""] | join("\u001f")' "$SETTINGS" 2>/dev/null) ||
  SPACE_ROWS=$(printf 'inbox\nterminal\nto-do\nrunbook\nmeetings')
SPACES=$(printf '%s\n' "$SPACE_ROWS" | cut -d "$(printf '\037')" -f1)
