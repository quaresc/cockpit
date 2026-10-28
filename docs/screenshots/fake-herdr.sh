#!/bin/sh
# Stands in for herdr while shooting: one agent tab, waiting for its user.
case "$1 $2" in
  "workspace list") echo '{"result":{"workspaces":[{"workspace_id":"w7","label":"[7] quota-banner","focused":true}]}}' ;;
  "pane list") echo '{"result":{"panes":[{"pane_id":"p7","tab_id":"t7","workspace_id":"w7","agent":"claude","agent_status":"blocked"}]}}' ;;
  "pane get") echo '{"result":{"pane":{"pane_id":"p7","tab_id":"t7","workspace_id":"w7","agent":"claude","agent_status":"blocked"}}}' ;;
  "tab get") echo '{"result":{"tab":{"tab_id":"t7","label":"quota-banner","focused":false}}}' ;;
esac
