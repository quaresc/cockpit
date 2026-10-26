#!/bin/sh
# Stops the agent-browser daemons terminal-browser leaves behind: each serves one browser, named in its session
# (terminal-browser-<browser pid>-<n>), and outlives it once that browser is gone and its debugging port closed.
#   prune-browsers.sh [--count]   --count prints how many there are instead
left=0
for pid in $(pgrep -f 'terminal-browser/agent-browser/bin/agent-browser' 2>/dev/null); do
  if [ -r "/proc/$pid/environ" ]; then env=$(tr '\0' '\n' <"/proc/$pid/environ")
  else env=$(ps -E -o command= -p "$pid" 2>/dev/null | tr ' ' '\n'); fi
  session=$(printf '%s\n' "$env" | sed -n 's/^AGENT_BROWSER_SESSION=//p' | head -1)
  port=$(printf '%s\n' "$env" | sed -n 's/^AGENT_BROWSER_CDP=//p' | head -1)
  dir=$(printf '%s\n' "$env" | sed -n 's/^AGENT_BROWSER_SOCKET_DIR=//p' | head -1)
  browser=$(printf '%s' "$session" | sed -n 's/^terminal-browser-\([0-9][0-9]*\)-[0-9][0-9]*$/\1/p')
  [ -n "$browser" ] || continue
  kill -0 "$browser" 2>/dev/null && continue
  [ -n "$port" ] && lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1 && continue
  left=$((left + 1))
  [ "$1" = --count ] && continue
  kill "$pid" 2>/dev/null
  [ -n "$dir" ] && rm -f "$dir/$session".*
done
[ "$1" = --count ] && { echo "$left"; exit 0; }
# The files of a session whose browser is gone, daemon or not.
for file in "$HOME"/.local/state/terminal-browser-*/agent-browser/terminal-browser-*-*.*; do
  [ -e "$file" ] || continue
  browser=$(basename "$file" | sed -n 's/^terminal-browser-\([0-9][0-9]*\)-[0-9][0-9]*\..*$/\1/p')
  [ -n "$browser" ] && ! kill -0 "$browser" 2>/dev/null && rm -f "$file"
done
exit 0
