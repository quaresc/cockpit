#!/bin/sh
# Names each single-tab space after its tab, so the sidebar's space list reads as the task list.
# The pinned spaces, shell tabs and the hub's fetcher keep their own names; automatic-rename adds the "[N] " prefix.
. "$(dirname "$0")/lib.sh"
tabs=$("$H" tab list) || exit 0
# The fetcher's space, by the id the hub wrote down: its tab is named after Claude Code, which would hide why it is there.
fetcher=$(tr -d '[:space:]' <"$HOME/.cache/cockpit/fetcher.space" 2>/dev/null)
"$H" workspace list | jq -r --argjson tabs "$tabs" --arg pinned "$SPACES" --arg fetcher "$fetcher" '
  def strip: sub("^\\[[0-9]+\\] "; "");
  .result.workspaces[]
  | select(.tab_count == 1)
  | select($fetcher == "" or .workspace_id != $fetcher)
  | . as $w
  | ($tabs.result.tabs[] | select(.workspace_id == $w.workspace_id) | .label | strip) as $want
  | select(any($pinned | split("\n")[] | select(. != ""); . as $n | $w.label | endswith("  " + $n)) | not)
  | select($want != "" and ($want | test("^(zsh|bash|sh|fish)$") | not))
  | select(($w.label | strip) != $want)
  | [$w.workspace_id, (($w.label | capture("^(?<p>\\[[0-9]+\\] )?").p // "") + $want)] | @tsv' |
while IFS="$(printf '\t')" read -r wid label; do
  "$H" workspace rename "$wid" "$label" >/dev/null 2>&1
done
