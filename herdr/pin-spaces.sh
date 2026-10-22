#!/bin/sh
# Keeps the pinned spaces on top of the sidebar, in the configured order, agents after.
. "$(dirname "$0")/lib.sh"
pin() {
  "$H" workspace list | jq -r --arg m "$1" --argjson i "$2" '
    .result.workspaces | to_entries
    | map(select(.value.label | contains($m))) | .[0] // empty
    | select(.key != $i) | .value.workspace_id' |
  while read -r id; do
    printf '{"id":"cockpit:pin","method":"workspace.move","params":{"workspace_id":"%s","insert_index":%s}}\n' "$id" "$2" |
      python3 -c 'import os,socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(os.environ["HERDR_SOCKET_PATH"]); s.sendall(sys.stdin.buffer.read()); s.recv(65536)'
  done
}
i=0
for space in $SPACES; do
  pin "$space" "$i"
  i=$((i + 1))
done
