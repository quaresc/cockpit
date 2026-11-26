#!/bin/sh
# Checks that each skill shipped to Claude Code (plugin/skills) and to Codex (codex/skills) says the same thing in both,
# apart from the description line and the passages between `<!-- agent: … -->` and `<!-- /agent -->`, and that the
# plugin's sites table (plugin/hooks/sites.ts) lists the same kinds as the Rust core's (herdr/tui/core/src/sites.rs).
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
a=$(mktemp) b=$(mktemp)
grep -o 'kind: "[a-z-]*"' herdr/tui/core/src/sites.rs | tr -d '"' | sort > "$a"
grep -o "kind: '[a-z-]*'" plugin/hooks/sites.ts | tr -d "'" | sort > "$b"
diff -u --label herdr/tui/core/src/sites.rs --label plugin/hooks/sites.ts "$a" "$b" || status=1
rm -f "$a" "$b"
exit "$status"
