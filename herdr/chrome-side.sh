#!/bin/sh
# One Chrome window per Claude space, laid out beside the terminal while that space is focused.
#   claim <workspace>   open Chrome if needed, give the space its window, show it
#   sync                show the focused space's window, minimize the others (herdr focus events)
#   release <workspace> minimize the space's window and forget it (a closed space's window is closed on sync)
H="${HERDR_BIN_PATH:-herdr}"
MAP="$HOME/.cache/cockpit/chrome-windows.json"
LAST="$HOME/.cache/cockpit/chrome-focus"
SPLIT=0.58
mkdir -p "$(dirname "$MAP")"
[ -s "$MAP" ] || echo '{}' > "$MAP"

window_of() { jq -r --arg w "$1" '.[$w] // empty' "$MAP"; }

window_exists() {
  [ -n "$1" ] && pgrep -xq "Google Chrome" &&
    [ "$(osascript -e "tell application \"Google Chrome\" to exists window id $1" 2>/dev/null)" = "true" ]
}

# Terminal on the left of its screen, the Chrome window (when given) on the right; else terminal full width.
layout() {
  osascript -l JavaScript - "${1:-}" "$SPLIT" <<'JS' >/dev/null 2>&1
function run(argv) {
  ObjC.import('AppKit')
  const chromeId = argv[0], split = parseFloat(argv[1])
  const win = Application('iTerm2').currentWindow()
  const b = win.bounds()
  const screens = $.NSScreen.screens
  const mainHeight = screens.objectAtIndex(0).frame.size.height
  const cx = b.x + b.width / 2, cy = b.y + b.height / 2
  let area = null
  for (let i = 0; i < screens.count; i++) {
    const f = screens.objectAtIndex(i).frame, v = screens.objectAtIndex(i).visibleFrame
    const top = mainHeight - (f.origin.y + f.size.height)
    if (cx >= f.origin.x && cx < f.origin.x + f.size.width && cy >= top && cy < top + f.size.height) {
      // The screen's width, the terminal's own top and height (macOS keeps windows clear of the menu bar).
      area = { x: v.origin.x, y: b.y, width: v.size.width, height: b.height }
    }
  }
  if (!area) return
  if (!chromeId) {
    win.bounds = area
    return
  }
  const left = Math.round(area.width * split)
  win.bounds = { x: area.x, y: area.y, width: left, height: area.height }
  const chrome = Application('Google Chrome').windows.byId(parseInt(chromeId))
  chrome.minimized = false
  chrome.bounds = { x: area.x + left, y: area.y, width: area.width - left, height: area.height }
  chrome.index = 1
}
JS
}

minimize_others() {
  keep=$1
  jq -r 'to_entries[] | "\(.key) \(.value)"' "$MAP" | while read -r ws id; do
    [ "$ws" = "$keep" ] && continue
    window_exists "$id" && osascript -e "tell application \"Google Chrome\" to set minimized of window id $id to true" >/dev/null 2>&1
  done
}

case "${1:-}" in
  claim)
    ws=$2
    id=$(window_of "$ws")
    if ! window_exists "$id"; then
      if ! pgrep -xq "Google Chrome"; then
        open -a "Google Chrome"
        # Give the Claude in Chrome extension time to connect before the tool call goes through.
        sleep 5
      fi
      id=$(osascript -e 'tell application "Google Chrome"' -e 'make new window' -e 'get id of front window' -e 'end tell')
      tmp=$(mktemp) && jq --arg w "$ws" --argjson id "$id" '.[$w] = $id' "$MAP" > "$tmp" && mv "$tmp" "$MAP"
      osascript -e 'tell application "iTerm2" to activate' >/dev/null 2>&1
    fi
    minimize_others "$ws"
    layout "$id"
    echo "$ws" > "$LAST"
    ;;
  sync)
    ws=$("$H" workspace list | jq -r '.result.workspaces[] | select(.focused) | .workspace_id')
    [ -n "$ws" ] || exit 0
    [ "$(cat "$LAST" 2>/dev/null)" = "$ws" ] && exit 0
    echo "$ws" > "$LAST"
    # A space that is gone takes its window with it.
    live=$("$H" workspace list | jq -c '[.result.workspaces[].workspace_id]')
    jq -r --argjson live "$live" 'to_entries[] | select(.key as $k | $live | index($k) | not) | .value' "$MAP" | while read -r id; do
      window_exists "$id" && osascript -e "tell application \"Google Chrome\" to close window id $id" >/dev/null 2>&1
    done
    tmp=$(mktemp) && jq --argjson live "$live" 'with_entries(select(.key as $k | $live | index($k)))' "$MAP" > "$tmp" && mv "$tmp" "$MAP"
    id=$(window_of "$ws")
    minimize_others "$ws"
    if window_exists "$id"; then layout "$id"; else layout ""; fi
    ;;
  release)
    ws=$2
    id=$(window_of "$ws")
    window_exists "$id" && osascript -e "tell application \"Google Chrome\" to set minimized of window id $id to true" >/dev/null 2>&1
    tmp=$(mktemp) && jq --arg w "$ws" 'del(.[$w])' "$MAP" > "$tmp" && mv "$tmp" "$MAP"
    [ "$(cat "$LAST" 2>/dev/null)" = "$ws" ] && layout ""
    ;;
esac
