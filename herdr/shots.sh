#!/bin/sh
# Screenshots of a page at phone, tablet and desktop sizes in the terminal-browser beside the calling pane.
#   shots.sh <url> <out dir> [--sizes mobile,tablet,desktop] [--full] [--session <Claude session id>]
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
exec python3 "$(dirname "$0")/shots.py" "$@"
