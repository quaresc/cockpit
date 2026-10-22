#!/bin/sh
# Keeps the inbox app running in the inbox space; ctrl+c twice in a row leaves it.
while :; do
  # Entered on every run, so an update of the plugin reaches the next one.
  cd "$(dirname "$0")" || exit 1
  # The extra argument marks the app as cockpit's, for update.sh to restart it.
  python3 inbox.py cockpit-space
  sleep 1
done
