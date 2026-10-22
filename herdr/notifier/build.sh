#!/bin/sh
# Builds ~/.config/cockpit/Cockpit.app, the macOS notifier with the cockpit icon; does nothing when it is current.
set -e
SRC=$(cd "$(dirname "$0")" && pwd -P)
APP="$HOME/.config/cockpit/Cockpit.app"
STAMP=$(cat "$SRC/main.swift" "$SRC/icon.png" "$SRC/build.sh" | shasum | cut -c1-16)
[ "$(cat "$APP/Contents/Resources/stamp" 2>/dev/null)" = "$STAMP" ] && exit 0
command -v swiftc >/dev/null || { echo "notifier: swiftc not found (xcode-select --install)" >&2; exit 1; }
tmp=$(mktemp -d)
mkdir -p "$tmp/Cockpit.app/Contents/MacOS" "$tmp/Cockpit.app/Contents/Resources" "$tmp/icon.iconset"
swiftc -O -o "$tmp/Cockpit.app/Contents/MacOS/cockpit-notify" "$SRC/main.swift" -framework Cocoa -framework UserNotifications
for s in 16 32 128 256 512; do
  sips -z $s $s "$SRC/icon.png" --out "$tmp/icon.iconset/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) "$SRC/icon.png" --out "$tmp/icon.iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$tmp/icon.iconset" -o "$tmp/Cockpit.app/Contents/Resources/AppIcon.icns"
cat > "$tmp/Cockpit.app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.cockpit.notifier</string>
  <key>CFBundleName</key><string>Cockpit</string>
  <key>CFBundleDisplayName</key><string>Cockpit</string>
  <key>CFBundleExecutable</key><string>cockpit-notify</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSUIElement</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>
PLIST
echo "$STAMP" > "$tmp/Cockpit.app/Contents/Resources/stamp"
codesign --force --sign - "$tmp/Cockpit.app" >/dev/null 2>&1
mkdir -p "$(dirname "$APP")" && rm -rf "$APP" && mv "$tmp/Cockpit.app" "$APP" && rm -rf "$tmp"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP"
