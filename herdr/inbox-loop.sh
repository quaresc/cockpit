#!/bin/sh
# Keeps the inbox app running in the inbox space; ctrl+c twice in a row leaves it.
while :; do
  # Entered on every run, so an update of the plugin reaches the next one.
  cd "$(dirname "$0")" || exit 1
  python3 inbox.py
  sleep 1
done
