#!/usr/bin/env bash
# Builds "FrameMap Studio.app" and a drag-to-install .dmg. Run on a Mac from the repo root.
# Usage: bash packaging/macos/bundle.sh            (builds a universal Intel + Apple Silicon app)
set -euo pipefail
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
APP="dist/dmg/FrameMap Studio.app"

rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

rm -rf dist/dmg && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/framemap" \
  target/aarch64-apple-darwin/release/framemap \
  target/x86_64-apple-darwin/release/framemap
sed "s/__VERSION__/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
cp assets/icon.icns "$APP/Contents/Resources/AppIcon.icns"
# ad-hoc signature (required for Apple Silicon); not notarised
codesign --force --deep --sign - "$APP"
ln -s /Applications dist/dmg/Applications

hdiutil create -volname "FrameMap Studio" -srcfolder dist/dmg -ov -format UDZO "dist/FrameMap-Studio.dmg"
echo "Built dist/FrameMap-Studio.dmg (version $VERSION)"
