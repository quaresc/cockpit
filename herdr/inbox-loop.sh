#!/bin/sh
# Keeps the inbox app running in the inbox space; ctrl+c twice in a row leaves it.
cd "$(dirname "$0")" || exit 1
while :; do
  python3 inbox.py
  sleep 1
done
