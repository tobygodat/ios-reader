#!/bin/bash
# Builds the Rust library for the simulator and a device, generates the Xcode project, builds
# the app for an iOS simulator, and prints the .app path.
set -euo pipefail
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
export IPHONEOS_DEPLOYMENT_TARGET=18.0
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

configuration="${CONFIGURATION:-Debug}"
profile=()
[ "$configuration" = Release ] && profile=(--release)

cargo build -p reader-ios --target aarch64-apple-ios-sim ${profile[@]+"${profile[@]}"}
cargo build -p reader-ios --target aarch64-apple-ios ${profile[@]+"${profile[@]}"}

xcodegen generate --spec ios/project.yml --project ios --quiet

derived="$root/target/xcode"
xcodebuild -project ios/Reader.xcodeproj -scheme Reader -configuration "$configuration" \
  -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath "$derived" CODE_SIGNING_ALLOWED=NO -quiet build

echo "$derived/Build/Products/$configuration-iphonesimulator/Reader.app"
