#!/bin/sh
# Runs the Chromium side browser's end-to-end test (browser/tests/smoke.electron.mjs) in the Electron browser/pins.json
# pins: COCKPIT_BROWSER_ELECTRON names one already there, else the pinned one is downloaded once into
# ~/.cache/cockpit-ci/electron and checked against its checksum. macOS only, like the browser; needs Node 22.13 or later.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
[ "$(uname -s)" = Darwin ] || { echo "the Chromium side browser runs on macOS only" >&2; exit 2; }
electron=${COCKPIT_BROWSER_ELECTRON:-}
if [ -z "$electron" ]; then
  case "$(uname -m)" in arm64) arch=arm64 ;; *) arch=x64 ;; esac
  pins=$repo/browser/pins.json
  version=$(jq -r .electron.version "$pins")
  want=$(jq -r ".electron.checksum.$arch" "$pins")
  url=$(jq -r .electron.url "$pins" | sed "s/{version}/$version/g; s/{arch}/$arch/g")
  dir=$HOME/.cache/cockpit-ci/electron/$version
  electron=$dir/Electron.app/Contents/MacOS/Electron
  if [ ! -x "$electron" ]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    echo "downloading Electron $version ($arch)"
    curl -fsSL --connect-timeout 15 --max-time 900 -o "$tmp/electron.zip" "$url"
    got=$(shasum -a 256 "$tmp/electron.zip" | cut -d ' ' -f 1)
    [ "$got" = "$want" ] || { echo "Electron $version: the download does not have the pinned checksum" >&2; exit 1; }
    rm -rf "$dir"
    mkdir -p "$dir"
    ditto -x -k "$tmp/electron.zip" "$dir"
  fi
fi
cd "$repo"
COCKPIT_BROWSER_ELECTRON=$electron exec node --test browser/tests/smoke.electron.mjs
