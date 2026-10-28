#!/bin/sh
# prefix+space: the palette popup over the pane in view, which it is told of so it can type into a Claude session there.
H="${HERDR_BIN_PATH:-herdr}"
pane=${HERDR_PANE_ID:-}
[ -n "$pane" ] || pane=$("$H" pane current | jq -r '.result.pane.pane_id // empty')
"$H" plugin pane open --plugin cockpit --entrypoint palette --placement popup --env "PALETTE_PANE=$pane" >/dev/null
