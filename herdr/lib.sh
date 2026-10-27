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
SPACE_ROWS=$(jq -r '(.spaces // ["inbox", "terminal", "actions", "to-do", "runbook", "meetings"])[]
  | if type == "object" then . else {name: .} end | [.name, .icon // "", .cwd // ""] | join("\u001f")' "$SETTINGS" 2>/dev/null) ||
  SPACE_ROWS=$(printf 'inbox\nterminal\nactions\nto-do\nrunbook\nmeetings')
SPACES=$(printf '%s\n' "$SPACE_ROWS" | cut -d "$(printf '\037')" -f1)

# Whether a space label, "[N] <icon>  <name>", is one of the pinned spaces.
pinned() {
  printf '%s\n' "$SPACES" | { while IFS= read -r name; do case "$1" in *"  $name") exit 0 ;; esac; done; exit 1; }
}

# Where the session in pane $1 shows, as label, click target and whether it is in view, split by tabs: its tab when
# it sits in a pinned space, else its space.
place() {
  found=$("$H" pane get "$1" 2>/dev/null | jq -c '.result.pane') && [ -n "$found" ] || return 1
  space=$("$H" workspace list | jq -c --arg w "$(printf '%s' "$found" | jq -r '.workspace_id')" '.result.workspaces[] | select(.workspace_id == $w)')
  [ -n "$space" ] || return 1
  if pinned "$(printf '%s' "$space" | jq -r '.label')"; then
    "$H" tab get "$(printf '%s' "$found" | jq -r '.tab_id')" | jq -r '.result.tab | [.label, "tab:" + .tab_id, .focused] | @tsv'
  else
    printf '%s' "$space" | jq -r '[(.label | sub("^\\[[0-9]+\\] *"; "")), "workspace:" + .workspace_id, .focused] | @tsv'
  fi
}
