#!/bin/sh
# A Herdr app window of its own on the acme demo world, for working on the cockpit mod: never the person's window,
# never their cockpit data. Everything lives under $COCKPIT_SANDBOX (default: $TMPDIR/cockpit-gui). The window shares
# herdr-core with the person's own: its spaces and terminals are theirs, so never create, close or type into them here.
#
#   sh scripts/sandbox.sh start            fake HOME from docs/screenshots/demo_data.py (FRESH=1 rebuilds it), dev build,
#                                      a separate Herdr window on the cockpit layout, and the layout and mod copied
#                                      again each time a file of app/ changes (Herdr reloads the mod on its own)
#   sh scripts/sandbox.sh do <cmd> [json]  a Herdr command in that window (layout.select, app.theme, ...)
#   sh scripts/sandbox.sh show <app>       that app's block in front (work, todo, inbox, ...)
#   sh scripts/sandbox.sh kit [charts]     the design kit's gallery as a window (every ui.js component, both themes)
#   sh scripts/sandbox.sh light|dark       the window's appearance
#   sh scripts/sandbox.sh key <keystroke>  a key pressed in the shown views (j, shift-d, ctrl-p, escape, ...)
#   sh scripts/sandbox.sh shot [file.png]  a screenshot of that window, its path printed
#   sh scripts/sandbox.sh check            herdr-app check on the mod
#   sh scripts/sandbox.sh status           herdr-app status (errors from the mod show here)
#   sh scripts/sandbox.sh notify <title> <body> [action]
#                                      a cockpit notification, as the hub and the plugin post them
#   sh scripts/sandbox.sh env              the variables to run cockpit or herdr-app against the sandbox yourself
#   sh scripts/sandbox.sh stop             quits the window and stops the sandbox's hub (and its herdr-core, see HC_SESSION)
#
# HC_SESSION=<name> gives the window a herdr-core of its own, empty, so none of the person's terminals shows in it
# (screenshots); terminals and spaces made there are the sandbox's.
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
tmp=${TMPDIR:-/tmp}
box=${COCKPIT_SANDBOX:-${tmp%/}/cockpit-gui}
box=${box%/}
fake_home="$box/home"
app_home="$box/herdr-home"
bin=${COCKPIT_BIN:-$repo/herdr/tui/target/debug/cockpit}
herdr_app=${HERDR_APP_BIN:-$HOME/.local/bin/herdr-app}
app_bundle=${HERDR_APP_BUNDLE:-$HOME/Applications/Herdr.app}
session=${HC_SESSION:-}

# Notifications always show as the window's toasts (it is rarely in front), and system ones only go to a log. hc runs
# on the real HOME: under the fake one it would start a herdr-core of its own, not the window's. The hub fetches
# nothing: the demo world's documents stay as written.
in_box() {
  HOME="$fake_home" HERDR_APP_HOME="$app_home" COCKPIT_FRONTEND=app COCKPIT_TOASTS=always COCKPIT_HUB_FEED=off \
    COCKPIT_NOTIFY_PROGRAM="$box/notified.sh" COCKPIT_HC_BIN="$box/hc.sh" "$@"
}
app() { HERDR_APP_HOME="$app_home" "$herdr_app" "$@"; }

running() { [ -S "$app_home/herdr-app.sock" ] && app status >/dev/null 2>&1; }

start() {
  (cd "$repo/herdr/tui" && cargo build -q -p cockpit-cli)
  mkdir -p "$fake_home" "$app_home"
  # The welcome window would cover every screenshot.
  [ -f "$app_home/shared.json" ] || printf '{"frame.welcomed": "skipped"}\n' >"$app_home/shared.json"
  printf '#!/bin/sh\nprintf "%%s | " "$@" >> "%s/notified.log"; echo >> "%s/notified.log"\n' "$box" "$box" >"$box/notified.sh"
  chmod +x "$box/notified.sh"
  printf '#!/bin/sh\nHOME="%s" %sexec "%s/.local/bin/hc" "$@"\n' "$HOME" "${session:+HC_SESSION=$session }" "$HOME" >"$box/hc.sh"
  chmod +x "$box/hc.sh"
  if [ ! -f "$fake_home/.config/cockpit/config.json" ] || [ "${FRESH:-}" = 1 ]; then
    # The window's mod would start a hub again at once, and the hub's store keeps the documents of the last run.
    if running; then
      app "do" app.quit >/dev/null 2>&1 || true
      i=0
      while [ -S "$app_home/herdr-app.sock" ] && app status >/dev/null 2>&1 && [ $i -lt 40 ]; do i=$((i + 1)); sleep 0.25; done
    fi
    in_box "$bin" hub stop >/dev/null 2>&1 || true
    rm -f "$fake_home/.cache/cockpit/store.redb"
    python3 "$repo/docs/screenshots/demo_data.py" "$fake_home" >/dev/null
  fi
  if ! running; then
    # The app writes its home (AGENTS.md, keys.json, the shipped mods) on its first start.
    if [ -n "$session" ]; then
      open -n "$app_bundle" --env HERDR_APP_HOME="$app_home" --env HC_SESSION="$session"
    else
      open -n "$app_bundle" --env HERDR_APP_HOME="$app_home"
    fi
    i=0
    until running; do
      i=$((i + 1))
      [ $i -gt 60 ] && { echo "the sandbox's Herdr window did not start" >&2; exit 1; }
      sleep 0.5
    done
  fi
  [ -f "$box/watch.pid" ] && kill "$(cat "$box/watch.pid")" 2>/dev/null || true
  in_box "$bin" app install --from "$repo/app"
  # The mod's next hub starts with the environment the install just wrote.
  in_box "$bin" hub stop >/dev/null 2>&1 || true
  # The watcher notes its own pid: $! would name the subshell running in_box, and stop would leave the watcher behind.
  in_box sh -c 'echo $$ >"$1"; exec nohup "$2" app install --watch --from "$3"' sh "$box/watch.pid" "$bin" "$repo/app" >"$box/watch.log" 2>&1 &
  app "do" overlay.close >/dev/null 2>&1 || true
  app "do" layout.switch '{"name": "cockpit"}' >/dev/null
  echo "sandbox: $box (HOME $fake_home, HERDR_APP_HOME $app_home)"
}

case "${1:-}" in
  start) start ;;
  do) shift; app "do" "$@" ;;
  show) app "do" "cockpit.go:$2" ;;
  kit) app "do" "cockpit.kit${2:+:$2}" ;;
  light | dark)
    python3 -c "import json,sys; p=sys.argv[1]; d=json.load(open(p)); d['appearance']=sys.argv[2]; json.dump(d, open(p, 'w'), indent=2)" \
      "$app_home/settings.json" "$1"
    ;;
  key) app "do" "cockpit.key:$2" >/dev/null ;;
  shot) sleep "${SETTLE:-1}"; app shot "${2:-$box/shot.png}" ;;
  check) app check "$app_home/mods/cockpit" ;;
  status) app status ;;
  notify) shift; in_box "$bin" notify "$@" ;;
  env) echo "HOME=$fake_home HERDR_APP_HOME=$app_home COCKPIT_FRONTEND=app" ;;
  stop)
    [ -f "$box/watch.pid" ] && kill "$(cat "$box/watch.pid")" 2>/dev/null || true
    in_box "$bin" hub stop >/dev/null 2>&1 || true
    running && app "do" app.quit >/dev/null 2>&1 || true
    [ -z "$session" ] || "$box/hc.sh" server stop >/dev/null 2>&1 || true
    ;;
  *) sed -n '2,23p' "$0"; exit 1 ;;
esac
