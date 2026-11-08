#!/bin/sh
# Builds the Mac app in release and writes an ad-hoc signed Reader.app, with its icon made
# from assets/icon.svg. Prints the bundle's path. Usage: scripts/mac-app.sh [OUT_DIR]
set -eu
cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
out_dir="${1:-target/release}"
app="$out_dir/Reader.app"

cargo build -q --release -p reader-mac

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
qlmanage -t -s 1024 -o "$work" assets/icon.svg >/dev/null 2>&1
png="$work/icon.svg.png"
[ -s "$png" ] || { echo "couldn't render assets/icon.svg" >&2; exit 1; }
iconset="$work/Reader.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$work/Reader.icns"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/reader "$app/Contents/MacOS/reader"
cp "$work/Reader.icns" "$app/Contents/Resources/Reader.icns"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Reader</string>
  <key>CFBundleDisplayName</key><string>Reader</string>
  <key>CFBundleIdentifier</key><string>com.tobygodat.reader</string>
  <key>CFBundleExecutable</key><string>reader</string>
  <key>CFBundleIconFile</key><string>Reader</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.books</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$app" >/dev/null 2>&1
codesign --verify "$app"
echo "$(cd "$(dirname "$app")" && pwd)/Reader.app"
