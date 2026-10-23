#!/bin/sh
# Opens a URL beside a Claude session's pane in terminal-browser when it is installed, else in the default browser.
#   browse.sh [--size <fraction>] [--reload] <url> [<Claude session id>]   the session's pane, else the calling pane
#   browse.sh --beside [<Claude session id>]                               the browser beside it, as JSON
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
exec python3 "$(dirname "$0")/browse.py" "$@"
