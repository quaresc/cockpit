#!/bin/sh
# A system notification with the cockpit icon; a click opens the action: a URL, or one focus.sh understands.
#   notify.sh <title> <body> [action]
. "$(dirname "$0")/lib.sh"
title=$1 body=$2 action=${3:-}
# The terminal app hosting herdr, brought forward when a click focuses a space.
[ -n "${__CFBundleIdentifier:-}" ] && [ "$__CFBundleIdentifier" != "dev.cockpit.notifier" ] &&
  echo "$__CFBundleIdentifier" > "$HOME/.cache/cockpit/terminal-app" 2>/dev/null
APP="$HOME/.config/cockpit/Cockpit.app/Contents/MacOS/cockpit-notify"
if [ "$(uname)" = Darwin ] && sh "$ROOT/notifier/build.sh" >/dev/null 2>&1 && [ -x "$APP" ]; then
  "$APP" post "$title" "$body" "$action" && exit 0
fi
"$H" notification show "$title" --body "$body" --sound request >/dev/null
