#!/bin/sh
# Keeps the meetings app running in the meetings space; ctrl+c twice in a row leaves it.
# Run from here, the app is in its own space and reports its count on that space's sidebar row.
export COCKPIT_SPACE=1
while :; do
  # Entered on every run, so an update of the plugin reaches the next one.
  cd "$(dirname "$0")" || exit 1
  python3 meetings.py
  sleep 1
done
