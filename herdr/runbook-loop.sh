#!/bin/sh
# Keeps the runbook app running in the runbook space; ctrl+c twice in a row leaves it.
cd "$(dirname "$0")" || exit 1
while :; do
  python3 runbook.py
  sleep 1
done
