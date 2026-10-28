#!/bin/sh
# A herdr folder script (ask-session.sh, notify.sh, meeting-brief.sh) that only logs its call.
exec python3 "$COCKPIT_TEST_DIR/bin/logcall" "$0" "$@"
