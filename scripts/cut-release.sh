#!/bin/sh
# Bumps every half to the next version and writes its CHANGELOG.md section from the commits since the last release,
# ready for a release commit.   cut-release.sh [patch|minor|major]
set -eu
cd "$(dirname "$0")/.."

bump=${1:-patch}
current=$(jq -r .version plugin/.claude-plugin/plugin.json)
next=$(printf '%s\n' "$current" | awk -F. -v bump="$bump" '{
  if (bump == "major") print $1 + 1 ".0.0"; else if (bump == "minor") print $1 "." $2 + 1 ".0"; else print $1 "." $2 "." $3 + 1 }')

# Every commit since the last release but the release commits themselves, oldest first.
notes=$(git log --reverse --format=%s "v$current..HEAD" | grep -Ev '^[0-9]+\.[0-9]+\.[0-9]+:' | sed 's/^/- /' || true)
if [ -z "$notes" ]; then
  echo "nothing to release since v$current" >&2
  exit 1
fi

# In place, line by line, so each file keeps its own layout.
for file in plugin/.claude-plugin/plugin.json codex/.codex-plugin/plugin.json; do
  sed -i.bak "s/^  \"version\": \"$current\",$/  \"version\": \"$next\",/" "$file" && rm "$file.bak"
done
sed -i.bak "s/^version = \"$current\"$/version = \"$next\"/" herdr/herdr-plugin.toml && rm herdr/herdr-plugin.toml.bak
sed -i.bak "/^\[workspace.package\]/,/^\[/s/^version = \"$current\"$/version = \"$next\"/" herdr/tui/Cargo.toml && rm herdr/tui/Cargo.toml.bak
(cd herdr/tui && cargo update --workspace --quiet)

# The new section goes above the newest one.
section=$(printf '## %s (%s)\n\n%s\n' "$next" "$(date -u +%Y-%m-%d)" "$notes")
SECTION=$section awk '!done && /^## / { print ENVIRON["SECTION"]; print ""; done = 1 } { print }' CHANGELOG.md >CHANGELOG.md.tmp
mv CHANGELOG.md.tmp CHANGELOG.md

echo "$next"
