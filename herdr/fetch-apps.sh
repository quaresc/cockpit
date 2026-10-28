#!/bin/sh
# Installs the Rust apps built for this version and machine from its GitHub release into bin/, once per version.
# A version released without them, or a checkout that builds its own, leaves the apps to run full or light.
PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
cd "$(dirname "$0")" || exit 1
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' herdr-plugin.toml)
[ -n "$version" ] || exit 0
[ "$(cat bin/.version 2>/dev/null)" = "$version" ] && exit 0
[ -x tui/target/release/cockpit-hub ] && exit 0
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
  echo "cockpit: the Rust apps $version could not be installed ($1); the apps run full or light until then → sh $(pwd)/fetch-apps.sh" >&2
  exit 1
}
get() {
  curl -sSL --connect-timeout 10 --max-time 300 -o "$tmp/$1" -w '%{http_code}' "$base/v$version/$1" 2>/dev/null
}

code=$(get "$file") || fail "download failed"
[ "$code" = 404 ] && exit 0
[ "$code" = 200 ] || fail "HTTP $code for $file"
code=$(get "$file.sha256") || fail "checksum download failed"
[ "$code" = 200 ] || fail "HTTP $code for $file.sha256"
want=$(cut -d ' ' -f 1 "$tmp/$file.sha256")
got=$({ sha256sum "$tmp/$file" 2>/dev/null || shasum -a 256 "$tmp/$file"; } | cut -d ' ' -f 1)
[ -n "$want" ] && [ "$want" = "$got" ] || fail "checksum mismatch"
mkdir "$tmp/bin" && tar -xzf "$tmp/$file" -C "$tmp/bin" 2>/dev/null || fail "unreadable archive"
ls "$tmp"/bin/cockpit-* >/dev/null 2>&1 || fail "no app in the archive"
chmod +x "$tmp"/bin/cockpit-*
printf '%s\n' "$version" > "$tmp/bin/.version"
rm -rf bin && mv "$tmp/bin" bin
