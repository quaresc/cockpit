#!/bin/sh
# Stands in for herdr while shooting: one agent space, waiting for its user.
case "$1 $2" in
  "workspace list") echo '{"result":{"workspaces":[{"workspace_id":"w7","label":"[7] quota-banner","focused":true}]}}' ;;
  "pane list") echo '{"result":{"panes":[{"workspace_id":"w7","agent":"claude","agent_status":"blocked"}]}}' ;;
esac
