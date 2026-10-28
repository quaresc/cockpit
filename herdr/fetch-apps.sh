#!/bin/sh
# Installs the cockpit binary built for this version and machine from its GitHub release into bin/, once per version.
# A checkout that builds its own (tui/target/release) needs none. A binary does it with `cockpit update-apps`: this
# folder's, else the one an earlier install of the plugin holds, which copies itself in while the release has no binary
# yet, so the spaces keep running. Only the first install, with no binary anywhere, downloads it here.
PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
cd "$(dirname "$0")" || exit 1
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' herdr-plugin.toml)
[ -n "$version" ] || exit 0
[ "$(cat bin/.version 2>/dev/null)" = "$version" ] && exit 0
[ -x tui/target/release/cockpit ] && exit 0
here=$(pwd -P)

# The binaries of earlier installs, newest first: the one the link names, and herdr's other folders for the plugin.
earlier() {
  case "$here" in */github-installations/*/checkout/herdr) siblings="$here/../../../*/checkout/herdr/bin/.version" ;; *) siblings= ;; esac
  # shellcheck disable=SC2086  # the siblings' pattern expands here
  ls -t "$HOME/.config/cockpit/herdr/bin/.version" $siblings 2>/dev/null | while IFS= read -r noted; do
    bin=$(cd "$(dirname "$noted")" && pwd -P)
    [ "$bin" = "$here/bin" ] || [ ! -x "$bin/cockpit" ] || printf '%s\n' "$bin/cockpit"
  done
}

# A binary without update-apps exits 1 with its usage: the next one, or the download below, takes over.
spaces=$IFS
IFS='
'
# shellcheck disable=SC2046  # one binary per line, whatever spaces its path holds
set -- bin/cockpit $(earlier)
IFS=$spaces
for binary do
  [ -x "$binary" ] || continue
  said=$("$binary" update-apps "$here" 2>&1)
  case $? in
    0) exit 0 ;;
    3) printf '%s\n' "$said" >&2; exit 1 ;;
  esac
done

case "$(uname -s) $(uname -m)" in
  "Darwin arm64") target=aarch64-apple-darwin ;;
  "Darwin x86_64") target=x86_64-apple-darwin ;;
  "Linux x86_64") target=x86_64-unknown-linux-gnu ;;
  "Linux aarch64" | "Linux arm64") target=aarch64-unknown-linux-gnu ;;
  *) exit 0 ;;
esac

# One download at a time, since ensure-spaces runs on several events at once; a lock left by a killed run expires.
find . -maxdepth 1 -name fetch.lock -mmin +10 -exec rmdir {} \; 2>/dev/null
mkdir fetch.lock 2>/dev/null || exit 0
tmp=$(mktemp -d bin.XXXXXX) || { rmdir fetch.lock; exit 1; }
trap 'rm -rf "$tmp"; rmdir fetch.lock' EXIT
trap 'exit 1' INT TERM HUP

base=${COCKPIT_RELEASES_URL:-https://github.com/quaresc/cockpit/releases/download}
file=cockpit-apps-$version-$target.tar.gz
fail() {
  echo "cockpit: cockpit $version could not be installed ($1); the spaces stay empty until then → sh $(pwd)/fetch-apps.sh" >&2
  exit 1
}
get() {
  curl -sSL --connect-timeout 10 --max-time 300 -o "$tmp/$1" -w '%{http_code}' "$base/v$version/$1" 2>/dev/null
}

code=$(get "$file") || fail "download failed"
[ "$code" = 404 ] && exit 0
[ "$code" = 200 ] || fail "HTTP $code for $file"
code=$(get "$file.sha256") || fail "checksum download failed"
[ "$code" = 404 ] && exit 0
[ "$code" = 200 ] || fail "HTTP $code for $file.sha256"
want=$(cut -d ' ' -f 1 "$tmp/$file.sha256")
got=$({ sha256sum "$tmp/$file" 2>/dev/null || shasum -a 256 "$tmp/$file"; } | cut -d ' ' -f 1)
[ -n "$want" ] && [ "$want" = "$got" ] || fail "checksum mismatch"
mkdir "$tmp/bin" && tar -xzf "$tmp/$file" -C "$tmp/bin" 2>/dev/null || fail "unreadable archive"
[ -f "$tmp/bin/cockpit" ] || fail "no cockpit binary in the archive"
chmod +x "$tmp"/bin/*
printf '%s\n' "$version" > "$tmp/bin/.version"
rm -rf bin && mv "$tmp/bin" bin
