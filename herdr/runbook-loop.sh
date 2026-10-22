#!/bin/sh
# Keeps the runbook app running in the runbook space; ctrl+c twice in a row leaves it.
cd "$(dirname "$0")" || exit 1
# Run from here, the app is in its own space and reports its count on that space's sidebar row.
export COCKPIT_SPACE=1
while :; do
  python3 runbook.py
  sleep 1
done
