#!/bin/sh
# Keeps the to-do app running in the to-do space; ctrl+c twice in a row leaves it.
cd "$(dirname "$0")" || exit 1
while :; do
  python3 todo.py
  sleep 1
done
