#!/bin/sh
# Opens a URL beside a Claude session's pane in terminal-browser when it is installed, else in the default browser.
#   browse.sh [--size <fraction>] [--reload] <url> [<Claude session id>]   the session's pane, else the calling pane
#   browse.sh --beside [<Claude session id>]                               the browser beside it, as JSON
#   browse.sh --external [<Claude session id>]                             its page, in the default browser
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
dir=$(dirname "$0")
for binary in "$dir/bin/cockpit" "$dir/tui/target/release/cockpit"; do
  [ -x "$binary" ] && exec "$binary" browse "$@"
done
echo "cockpit: no cockpit binary for this machine yet; /cockpit:doctor says why" >&2
exit 1
