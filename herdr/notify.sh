#!/bin/sh
# A system notification with the cockpit icon; a click opens the action: a URL, or one focus.sh understands.
# Each <label>=<action> after it adds a button doing that action instead (macOS).
#   notify.sh <title> <body> [action] [<label>=<action> ...]
. "$(dirname "$0")/lib.sh"
title=$1 body=$2 action=${3:-}
[ $# -gt 3 ] && shift 3 || set --
# Notifications switched off in the config silence every caller, the mod's and the apps' alike.
[ "$(jq -r '.notifications' "$CONFIG" 2>/dev/null)" = false ] && exit 0
# The terminal app hosting herdr, brought forward when a click focuses a space.
[ -n "${__CFBundleIdentifier:-}" ] && [ "$__CFBundleIdentifier" != "dev.cockpit.notifier" ] &&
  echo "$__CFBundleIdentifier" > "$HOME/.cache/cockpit/terminal-app" 2>/dev/null
APP="$HOME/.config/cockpit/Cockpit.app/Contents/MacOS/cockpit-notify"
if [ "$OS" = Darwin ] && sh "$ROOT/notifier/build.sh" >/dev/null 2>&1 && [ -x "$APP" ]; then
  "$APP" post "$title" "$body" "$action" "$@" && exit 0
fi
# Linux: notify-send, its default action (a click, where the daemon supports it) running the same target.
if [ "$OS" != Darwin ] && command -v notify-send >/dev/null; then
  if [ -n "$action" ] && notify-send --help 2>&1 | grep -q -- --action; then
    (choice=$(notify-send -a Cockpit -i "$ROOT/notifier/icon.png" -A default=Open --wait "$title" "$body" 2>/dev/null)
      [ "$choice" = default ] && sh "$LINK/focus.sh" "$action") >/dev/null 2>&1 &
    exit 0
  fi
  notify-send -a Cockpit -i "$ROOT/notifier/icon.png" "$title" "$body" && exit 0
fi
"$H" notification show "$title" --body "$body" --sound request >/dev/null
