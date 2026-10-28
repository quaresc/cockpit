#!/bin/sh
# Runs one cockpit app as `cockpit <app> [args]`: the release's binary in bin/, else a dev build in tui/target/release/.
# When neither is there it fetches the release's binary first, and otherwise says so and waits (COCKPIT_APP_WAIT seconds,
# 30 by default), so the space's loop does not spin.
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
cd "$(dirname "$0")" || exit 1
name=$1
shift
COCKPIT_HERDR_DIR=$(pwd)
export COCKPIT_HERDR_DIR

# By its full path, which the hub looks for to restart the apps on a new binary.
for dir in bin tui/target/release; do
  [ -x "$dir/cockpit" ] && exec "$COCKPIT_HERDR_DIR/$dir/cockpit" "$name" "$@"
done
sh fetch-apps.sh && [ -x bin/cockpit ] && exec "$COCKPIT_HERDR_DIR/bin/cockpit" "$name" "$@"
echo "cockpit: no cockpit binary for this machine yet; /cockpit:doctor says why" >&2
sleep "${COCKPIT_APP_WAIT:-30}"
exit 1
