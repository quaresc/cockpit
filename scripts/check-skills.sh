#!/bin/sh
# Checks that each skill shipped to Claude Code (plugin/skills) and to Codex (codex/skills) says the same thing in both,
# apart from the description line and the passages between `<!-- agent: … -->` and `<!-- /agent -->`.
set -eu
cd "$(dirname "$0")/.."
strip() { awk '/^<!-- agent: / { skip = 1; next } /^<!-- \/agent -->$/ { skip = 0; next } !skip && !/^description: /' "$1"; }
status=0
for codex in codex/skills/*/SKILL.md; do
  claude="plugin/skills/${codex#codex/skills/}"
  [ -f "$claude" ] || { echo "$codex has no copy in plugin/skills"; status=1; continue; }
  a=$(mktemp) b=$(mktemp)
  strip "$claude" > "$a"
  strip "$codex" > "$b"
  diff -u --label "$claude" --label "$codex" "$a" "$b" || status=1
  rm -f "$a" "$b"
done
exit "$status"
