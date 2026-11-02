#!/bin/sh
# The Herdr app half's screenshots: each cockpit view in a sandbox window (app/sandbox.sh) on the acme demo world, in
# dark and light, at a normal and a narrow width, as PNGs in docs/app/ (`<view>-<theme>.png`, `<view>-<theme>-<width>.png`).
#
#   sh docs/screenshots/app_shots.sh [view ...]   default: home work inbox runbook todo meetings monitor routines palette recap kit
#   OUT=<dir> WIDTHS="1280 760" THEMES="dark light" SETTLE=1.2 MAXW=1600 sh docs/screenshots/app_shots.sh palette
#
# The window comes to the front for each shot (a window behind others does not repaint), so leave the machine alone
# while it runs. COCKPIT_SANDBOX picks the sandbox; keep its path short, macOS caps a socket's path at 104 bytes.
set -eu

repo=$(cd "$(dirname "$0")/../.." && pwd)
tmp=${TMPDIR:-/tmp}
COCKPIT_SANDBOX=${COCKPIT_SANDBOX:-${tmp%/}/cockpit-gui}
export COCKPIT_SANDBOX
out=${OUT:-$repo/docs/app}
widths=${WIDTHS:-1280 760}
themes=${THEMES:-dark light}
settle=${SETTLE:-1.2}
views=${*:-home work inbox runbook todo meetings monitor routines palette recap kit}
# Retina shots are twice the window's width: each is scaled down to this many pixels across.
maxw=${MAXW:-1600}
herdr_app=${HERDR_APP_BIN:-$HOME/.local/bin/herdr-app}
app_home="$COCKPIT_SANDBOX/herdr-home"

sandbox() { sh "$repo/app/sandbox.sh" "$@"; }
app() { HERDR_APP_HOME="$app_home" "$herdr_app" "$@"; }

# The sandbox window's process: the Herdr whose environment names this sandbox's home.
window_pid() {
  for p in $(pgrep -f "Herdr.app/Contents/MacOS/Herdr"); do
    ps eww "$p" | tr ' ' '\n' | grep -qx "HERDR_APP_HOME=$app_home" && { echo "$p"; return; }
  done
}

front() { osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $1) to true" >/dev/null 2>&1 || true; }

# The window at this width: Herdr takes its size from window.json when it starts, so it starts again.
resize() {
  app "do" app.quit >/dev/null 2>&1 || true
  i=0
  while [ -n "$(window_pid)" ] && [ $i -lt 40 ]; do
    i=$((i + 1))
    sleep 0.25
  done
  python3 -c "import json,sys; p=sys.argv[1]; d=json.load(open(p)); d['state']='windowed'; d['bounds'][2]=float(sys.argv[2]); d['bounds'][3]=820.0; json.dump(d, open(p, 'w'), indent=2)" \
    "$app_home/window.json" "$1"
  sandbox start >/dev/null
  # The mod answers its commands once it has loaded.
  i=0
  until app "do" cockpit.go:work 2>/dev/null | grep -q '"ok"' || [ $i -gt 40 ]; do
    i=$((i + 1))
    sleep 0.25
  done
  sleep 1
  app "do" overlay.close >/dev/null 2>&1 || true
}

mkdir -p "$out"
# The app in front before, given the front back at the end.
before=$(lsappinfo info -only bundleid "$(lsappinfo front)" 2>/dev/null | sed 's/.*="\(.*\)"/\1/')
sandbox start

first=$(echo "$widths" | cut -d' ' -f1)
for width in $widths; do
  resize "$width"
  pid=$(window_pid)
  [ -n "$pid" ] || { echo "no sandbox window found for $app_home" >&2; exit 1; }
  for theme in $themes; do
    sandbox "$theme"
    for view in $views; do
      app "do" overlay.close >/dev/null 2>&1 || true
      case $view in
        palette) sandbox show work >/dev/null; app "do" cockpit.palette >/dev/null ;;
        recap) sandbox show home >/dev/null; app "do" cockpit.recap >/dev/null ;;
        kit) sandbox kit >/dev/null ;;
        *) sandbox show "$view" >/dev/null ;;
      esac
      front "$pid"
      sleep "$settle"
      name=$view-$theme
      [ "$width" = "$first" ] || name=$name-$width
      app shot "$out/$name.png" >/dev/null
      sips -Z "$maxw" "$out/$name.png" >/dev/null 2>&1 || true
      echo "$out/$name.png"
      # The kit is a window, which only a layout switch closes.
      [ "$view" != kit ] || { app "do" layout.switch '{"name": "main"}' >/dev/null; app "do" layout.switch '{"name": "cockpit"}' >/dev/null; }
    done
  done
done
app "do" overlay.close >/dev/null 2>&1 || true
[ "$(echo "$widths" | wc -w)" -eq 1 ] || resize "$first"
[ -z "$before" ] || [ "$before" = dev.herdr.app ] || osascript -e "tell application id \"$before\" to activate" >/dev/null 2>&1 || true
