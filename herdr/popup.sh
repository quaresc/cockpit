#!/bin/sh
# prefix+shift+a / prefix+shift+q: the agents or queue popup, told of the pane in view (the queue is that agent's).
H="${HERDR_BIN_PATH:-herdr}"
pane=${HERDR_PANE_ID:-}
[ -n "$pane" ] || pane=$("$H" pane current | jq -r '.result.pane.pane_id // empty')
"$H" plugin pane open --plugin cockpit --entrypoint "$1" --placement popup --env "QUEUE_PANE=$pane" >/dev/null
