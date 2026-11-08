#!/bin/sh
# Renders every scene headless into docs/screenshots: a desktop window, and the phone-sized
# scenes at iPhone width.
set -eu
cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
cargo build -q -p reader-mac
bin=target/debug/reader
out=docs/screenshots
mkdir -p "$out"
for scene in $("$bin" --list-scenes); do
  "$bin" --snap "$out/$scene.png" --scene "$scene" --at 1.5 >/dev/null 2>&1
  echo "$out/$scene.png"
done
for scene in library appearance pages; do
  "$bin" --snap "$out/$scene-phone.png" --scene "$scene" --size 390x844 --scale 3 --at 1.5 >/dev/null 2>&1
  echo "$out/$scene-phone.png"
done
