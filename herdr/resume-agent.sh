#!/bin/sh
# Restarts the Claude session in pane $1 on conversation $2 once it is idle: /exit, then claude --resume.
# An optional $3 is prepended to the command, e.g. "CLAUDE_CODE_NO_FLICKER=0".
set -u
H="${HERDR_BIN_PATH:-herdr}"
pane=$1 session=$2 prefix=${3:-}
status() { "$H" pane get "$pane" | jq -r '.result.pane | if .agent then .agent_status else "gone" end'; }

i=0
while [ "$(status)" = "working" ] && [ $i -lt 600 ]; do sleep 2; i=$((i + 1)); done
"$H" agent prompt "$pane" "/exit" >/dev/null 2>&1
i=0
while [ "$(status)" != "gone" ] && [ $i -lt 30 ]; do sleep 1; i=$((i + 1)); done
[ "$(status)" = "gone" ] || { echo "resume-agent: $pane still runs claude, not restarted"; exit 1; }
sleep 1
"$H" pane run "$pane" "${prefix:+$prefix }claude --resume $session" >/dev/null && echo "resume-agent: $pane resumed $session"
