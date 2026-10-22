#!/bin/sh
# Keeps the meetings app running in the meetings space; ctrl+c twice in a row leaves it.
cd "$(dirname "$0")" || exit 1
while :; do
  python3 meetings.py
  sleep 1
done
