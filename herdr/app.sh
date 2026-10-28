#!/bin/sh
# Runs one cockpit app as the config's `apps` says: "rust" (the default) its binary, from the release (bin/) or a dev build
# (tui/target/release/); "full" its Textual version (<name>_tx.py) when uv is installed; "light" the text one. A version
# that is missing or fails within its first seconds gives way to the next one down, so a space never stays empty.
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
cd "$(dirname "$0")" || exit 1
name=$1
shift
mode=$(jq -r '.apps // "rust"' "$HOME/.config/cockpit/config.json" 2>/dev/null)
mode=${mode:-rust}
COCKPIT_HERDR_DIR=$(pwd)
export COCKPIT_HERDR_DIR

# Runs one version and leaves with its exit code, unless it failed at once.
attempt() {
  started=$(date +%s)
  "$@"
  code=$?
  [ "$code" -eq 0 ] || [ $(($(date +%s) - started)) -ge 5 ] && exit "$code"
}

if [ "$mode" = rust ]; then
  for binary in "bin/cockpit-$name" "tui/target/release/cockpit-$name"; do
    [ -x "$binary" ] || continue
    attempt "$binary" "$@"
    break
  done
fi
if [ "$mode" != light ] && [ -f "${name}_tx.py" ] && command -v uv >/dev/null; then
  attempt uv run --quiet --script "${name}_tx.py" "$@"
fi
exec python3 "$name.py" "$@"
