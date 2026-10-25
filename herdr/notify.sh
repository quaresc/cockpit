#!/bin/sh
# A system notification with the cockpit icon; a click opens the action: a URL, or one focus.sh understands.
# Each <label>=<action> after it adds a button doing that action instead.
#   notify.sh <title> <body> [action] [<label>=<action> ...]
. "$(dirname "$0")/lib.sh"
title=$1 body=$2 action=${3:-}
[ $# -gt 3 ] && shift 3 || set --
# Notifications switched off in the config silence every caller, the mod's and the apps' alike.
[ "$(jq -r '.notifications' "$SETTINGS" 2>/dev/null)" = false ] && exit 0
# The terminal app hosting herdr, brought forward when a click focuses a space.
[ -n "${__CFBundleIdentifier:-}" ] && [ "$__CFBundleIdentifier" != "dev.cockpit.notifier" ] &&
  echo "$__CFBundleIdentifier" > "$HOME/.cache/cockpit/terminal-app" 2>/dev/null
APP="$HOME/.config/cockpit/Cockpit.app/Contents/MacOS/cockpit-notify"
if [ "$OS" = Darwin ] && sh "$ROOT/notifier/build.sh" >/dev/null 2>&1 && [ -x "$APP" ]; then
  "$APP" post "$title" "$body" "$action" "$@" && exit 0
fi
# Linux: notify-send, its default action (a click, where the daemon supports it) running the same target, and each
# <label>=<action> a button b<N> running its own.
if [ "$OS" != Darwin ] && command -v notify-send >/dev/null; then
  if { [ -n "$action" ] || [ $# -gt 0 ]; } && notify-send --help 2>&1 | grep -q -- --action; then
    n=0
    for button in "$@"; do
      n=$((n + 1))
      eval "target_$n=\${button#*=}"
      set -- "$@" -A "b$n=${button%%=*}"
    done
    shift "$n"
    # shellcheck disable=SC2086  # the default action is two words or none
    (choice=$(notify-send -a Cockpit -i "$ROOT/notifier/icon.png" ${action:+-A default=Open} "$@" --wait "$title" "$body" 2>/dev/null)
      case "$choice" in
        default) target=$action ;;
        b[0-9]*) eval "target=\${target_${choice#b}:-}" ;;
        *) target= ;;
      esac
      [ -n "$target" ] && sh "$LINK/focus.sh" "$target") >/dev/null 2>&1 &
    exit 0
  fi
  notify-send -a Cockpit -i "$ROOT/notifier/icon.png" "$title" "$body" && exit 0
fi
"$H" notification show "$title" --body "$body" --sound request >/dev/null
