#!/bin/bash
# Build the CloudKit helper app (docs/plans/todo-sync.md P3).
#
#   scripts/build.sh unsigned   Compile check. Ad-hoc signed WITHOUT entitlements: it starts, serves the
#                               RPC socket and answers `hello` with problem=missingEntitlement, but cannot
#                               reach iCloud. Touches nothing remote.
#   scripts/build.sh dev        Apple Development signing via Xcode automatic signing (CloudKit Development
#                               environment). CONTACTS APPLE: may register the App ID, its iCloud / Push
#                               capabilities, the CloudKit container and a Mac development profile.
#   scripts/build.sh devid      Developer ID archive + export via Xcode automatic signing (CloudKit
#                               Production environment). CONTACTS APPLE: may additionally create a
#                               Developer ID profile. Needs the schema deployed to Production first.
#
# dev / devid refuse to run unless the caller exports HELPER_ALLOW_CLOUD_CHANGES=1: that is the explicit
# go-ahead of the account owner for the Apple-account changes above (the maintainer's step 7.1 in the
# plan). Nothing here reads passwords, keys or tokens: signing goes through the logged-in Xcode account.
#
# Result: build/out/<mode>/AskHumanTodoSync.app
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-}"
case "$mode" in unsigned | dev | devid) ;; *)
    sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac

if [ "$mode" != unsigned ] && [ "${HELPER_ALLOW_CLOUD_CHANGES:-0}" != 1 ]; then
    echo "refusing: '$mode' contacts Apple and can create App ID / container / profile resources." >&2
    echo "export HELPER_ALLOW_CLOUD_CHANGES=1 once that has been explicitly approved." >&2
    exit 3
fi

xcodegen generate --quiet
project=AskHumanTodoSync.xcodeproj
out="build/out/$mode"
rm -rf "$out"
mkdir -p "$out" build

case "$mode" in
unsigned)
    xcodebuild -project "$project" -scheme AskHumanTodoSync -configuration Debug \
        -destination 'platform=macOS' -derivedDataPath build/unsigned \
        CODE_SIGNING_ALLOWED=NO build | tail -n 5
    cp -R build/unsigned/Build/Products/Debug/AskHumanTodoSync.app "$out/"
    codesign --force --sign - "$out/AskHumanTodoSync.app"
    ;;
dev)
    xcodebuild -project "$project" -scheme AskHumanTodoSync -configuration Debug \
        -destination 'platform=macOS' -derivedDataPath build/dev \
        -allowProvisioningUpdates build 2>&1 | tee build/xcodebuild-dev.log | tail -n 25
    cp -R build/dev/Build/Products/Debug/AskHumanTodoSync.app "$out/"
    ;;
devid)
    team=$(awk '/DEVELOPMENT_TEAM:/ {print $2; exit}' project.yml)
    cat >build/ExportOptions-DeveloperID.plist <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>method</key><string>developer-id</string>
  <key>teamID</key><string>$team</string>
  <key>signingStyle</key><string>automatic</string>
  <key>destination</key><string>export</string>
</dict></plist>
EOF
    rm -rf build/devid
    xcodebuild -project "$project" -scheme AskHumanTodoSync -configuration Release \
        -destination 'generic/platform=macOS' -archivePath build/devid/AskHumanTodoSync.xcarchive \
        -allowProvisioningUpdates archive 2>&1 | tee build/xcodebuild-devid-archive.log | tail -n 25
    xcodebuild -exportArchive -archivePath build/devid/AskHumanTodoSync.xcarchive \
        -exportPath build/devid/export -exportOptionsPlist build/ExportOptions-DeveloperID.plist \
        -allowProvisioningUpdates 2>&1 | tee build/xcodebuild-devid-export.log | tail -n 25
    cp -R build/devid/export/AskHumanTodoSync.app "$out/"
    ;;
esac

echo
echo "built: $out/AskHumanTodoSync.app"
echo "--- signature ---"
codesign -dv "$out/AskHumanTodoSync.app" 2>&1 | grep -E 'Identifier|Authority|TeamIdentifier|flags|Signature' || true
echo "--- entitlements (as signed) ---"
codesign -d --entitlements :- "$out/AskHumanTodoSync.app" 2>/dev/null || true
