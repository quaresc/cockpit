#!/bin/sh
# Keeps the to-do app running in the to-do space; ctrl+c twice in a row leaves it.
# Run from here, the app is in its own space and reports its count on that space's sidebar row.
export COCKPIT_SPACE=1
while :; do
  # Entered on every run, so an update of the plugin reaches the next one.
  cd "$(dirname "$0")" || exit 1
  # Mouse reporting off first, in case the previous run died without switching it off.
  printf '\033[?1003l\033[?1006l\033[?1000l'
  # The extra argument marks the app as cockpit's, for update.sh to restart it.
  sh app.sh todo cockpit-space
  sleep 1
done
