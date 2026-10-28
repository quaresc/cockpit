#!/bin/sh
# Screenshots of a page at phone, tablet and desktop sizes in the terminal-browser beside the calling pane.
#   shots.sh <url> <out dir> [--sizes mobile,tablet,desktop] [--full] [--session <Claude session id>]
# Exits 2 when they cannot be taken here, so the caller takes them in Chrome instead.
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
dir=$(dirname "$0")
for binary in "$dir/bin/cockpit" "$dir/tui/target/release/cockpit"; do
  [ -x "$binary" ] && exec "$binary" shots "$@"
done
echo "cockpit: no cockpit binary for this machine yet: take these screenshots in Chrome instead" >&2
exit 2
