#!/bin/sh
# The site as GitHub Pages serves it, in _site/, so it can be opened locally: the pages from site/ and the pictures
# from docs/, which pages.yml copies in the same way. Pass a port to serve it instead of only building it.
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
out=$repo/_site
rm -rf "$out"
cp -R "$repo/site" "$out"
mkdir -p "$out/img/app"
cp "$repo"/docs/*.svg "$repo"/docs/*.png "$out/img/"
cp "$repo"/docs/app/*.png "$out/img/app/"
cp "$repo/docs/cockpit.mp4" "$out/img/"
echo "site: $out/index.html"

[ $# -eq 0 ] && exit 0
echo "serving on http://localhost:$1"
exec python3 -m http.server "$1" --directory "$out"
