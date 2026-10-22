#!/bin/sh
# Names each single-tab space after its tab, so the sidebar's space list reads as the task list.
# The inbox, terminal, to-do, runbook and meetings spaces and shell tabs keep their own names; automatic-rename adds the "[N] " prefix.
H="${HERDR_BIN_PATH:-herdr}"
tabs=$("$H" tab list) || exit 0
"$H" workspace list | jq -r --argjson tabs "$tabs" '
  def strip: sub("^\\[[0-9]+\\] "; "");
  .result.workspaces[]
  | select(.tab_count == 1)
  | . as $w
  | ($tabs.result.tabs[] | select(.workspace_id == $w.workspace_id) | .label | strip) as $want
  | select(($w.label | test("inbox|terminal|to-do|runbook|meetings")) | not)
  | select($want != "" and ($want | test("^(zsh|bash|sh|fish)$") | not))
  | select(($w.label | strip) != $want)
  | [$w.workspace_id, (($w.label | capture("^(?<p>\\[[0-9]+\\] )?").p // "") + $want)] | @tsv' |
while IFS="$(printf '\t')" read -r wid label; do
  "$H" workspace rename "$wid" "$label" >/dev/null 2>&1
done
