#!/bin/bash
# Archives the iPhone app in Release and uploads it to App Store Connect, where it shows up in
# TestFlight. Signs with the Apple ID signed in to Xcode. The build number is the commit count,
# so commit before uploading. `scripts/testflight.sh export` writes the .ipa without uploading.
set -euo pipefail
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

destination="${1:-upload}"
case "$destination" in
  upload | export) ;;
  *) echo "usage: scripts/testflight.sh [upload|export]" >&2; exit 1 ;;
esac

work="$root/target/xcode"
archive="$work/Reader.xcarchive"
mkdir -p "$work"

xcodegen generate --spec ios/project.yml --project ios --quiet

rm -rf "$archive"
xcodebuild archive -project ios/Reader.xcodeproj -scheme Reader -configuration Release \
  -destination 'generic/platform=iOS' -archivePath "$archive" -allowProvisioningUpdates \
  CURRENT_PROJECT_VERSION="$(git rev-list --count HEAD)" -quiet

options="$work/ExportOptions.plist"
cat > "$options" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>method</key>
	<string>app-store-connect</string>
	<key>destination</key>
	<string>$destination</string>
	<key>teamID</key>
	<string>Q6NAWA42Y2</string>
	<key>signingStyle</key>
	<string>automatic</string>
	<key>manageAppVersionAndBuildNumber</key>
	<false/>
</dict>
</plist>
PLIST

xcodebuild -exportArchive -archivePath "$archive" -exportOptionsPlist "$options" \
  -exportPath "$work/export" -allowProvisioningUpdates

echo "$archive"
