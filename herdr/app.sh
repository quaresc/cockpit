#!/bin/sh
# Runs one cockpit app: its Textual version (<name>_tx.py) when the config's `apps` is "full" (the default) and uv is
# installed, else the text one. A Textual version that fails within its first seconds gives way to the text one, so a
# space never stays empty.
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH"
cd "$(dirname "$0")" || exit 1
name=$1
shift
mode=$(jq -r '.apps // "full"' "$HOME/.config/cockpit/config.json" 2>/dev/null)
if [ "$mode" = full ] && [ -f "${name}_tx.py" ] && command -v uv >/dev/null; then
  started=$(date +%s)
  uv run --quiet --script "${name}_tx.py" "$@"
  code=$?
  [ "$code" -eq 0 ] || [ $(($(date +%s) - started)) -ge 5 ] && exit "$code"
fi
exec python3 "$name.py" "$@"
