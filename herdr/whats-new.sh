#!/bin/sh
# What an update brought, as a popup that asks about its new options: after update.sh and when herdr starts, and only
# while an update with something to say was not shown yet (`cockpit options pending`).
. "$(dirname "$0")/lib.sh"
sh "$ROOT/cockpit.sh" options pending >/dev/null 2>&1 || exit 0
"$H" plugin pane open --plugin cockpit --entrypoint whats-new --placement popup >/dev/null
