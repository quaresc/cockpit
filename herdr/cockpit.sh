#!/bin/sh
# Runs the cockpit command: the release's binary in bin/, else a dev build in tui/target/release/. Without either it
# exits quietly, since herdr runs it on events, where an error helps nobody; /cockpit:doctor reports the missing binary.
dir=$(cd "$(dirname "$0")" && pwd)
COCKPIT_HERDR_DIR=$dir
export COCKPIT_HERDR_DIR
for binary in "$dir/bin/cockpit" "$dir/tui/target/release/cockpit"; do
  [ -x "$binary" ] && exec "$binary" "$@"
done
exit 0
