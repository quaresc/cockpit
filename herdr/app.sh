#!/bin/sh
# Runs one cockpit app: the release's binary in bin/, else a dev build in tui/target/release/. When neither is there it
# fetches the release's binaries first, and otherwise says so and waits (COCKPIT_APP_WAIT seconds, 30 by default), so the
# space's loop does not spin.
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
cd "$(dirname "$0")" || exit 1
name=$1
shift
COCKPIT_HERDR_DIR=$(pwd)
export COCKPIT_HERDR_DIR

for dir in bin tui/target/release; do
  [ -x "$dir/cockpit-$name" ] && exec "$dir/cockpit-$name" "$@"
done
sh fetch-apps.sh && [ -x "bin/cockpit-$name" ] && exec "bin/cockpit-$name" "$@"
echo "cockpit: no cockpit-$name for this machine yet; /cockpit:doctor says why" >&2
sleep "${COCKPIT_APP_WAIT:-30}"
exit 1
