#!/bin/sh
# Runs one cockpit app: its Textual version (<name>_tx.py) when the config's `apps` is "full" (the default) and uv is
# installed, its Rust one (tui/target/release/cockpit-<name>) when it is "rust" and built (else as for "full"), and the
# text one otherwise. One that fails within its first seconds gives way to the text one, so a space never stays empty.
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
cd "$(dirname "$0")" || exit 1
name=$1
shift
mode=$(jq -r '.apps // "full"' "$HOME/.config/cockpit/config.json" 2>/dev/null)
binary="tui/target/release/cockpit-$name"
if [ "$mode" = rust ] && [ -x "$binary" ]; then
  started=$(date +%s)
  COCKPIT_HERDR_DIR=$(pwd) "$binary" "$@"
  code=$?
  [ "$code" -eq 0 ] || [ $(($(date +%s) - started)) -ge 5 ] && exit "$code"
elif [ "$mode" != light ] && [ -f "${name}_tx.py" ] && command -v uv >/dev/null; then
  started=$(date +%s)
  uv run --quiet --script "${name}_tx.py" "$@"
  code=$?
  [ "$code" -eq 0 ] || [ $(($(date +%s) - started)) -ge 5 ] && exit "$code"
fi
exec python3 "$name.py" "$@"
