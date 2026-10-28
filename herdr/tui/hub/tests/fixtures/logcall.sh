#!/bin/sh
# A fake tool (claude, open, pbcopy, gh, the notification receiver, a herdr folder script): logs its name, arguments
# and, for pbcopy, its input to calls.log.
args=""
for arg do args="$args${args:+,}$(jq -n --arg v "$arg" '$v')"; done
stdin='""'
if [ "${0##*/}" = pbcopy ] && [ ! -t 0 ]; then stdin=$(jq -Rs .); fi
printf '{"cmd":%s,"args":[%s],"stdin":%s}\n' "$(jq -n --arg v "${0##*/}" '$v')" "$args" "$stdin" >>"$COCKPIT_TEST_DIR/calls.log"
