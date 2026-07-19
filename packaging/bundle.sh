#!/usr/bin/env bash
#
# Build a distributable macOS app: Houdini.app (+ a .dmg).
#
# Uses only Apple's own tools (sips, iconutil, codesign, hdiutil) plus cargo -
# no bundler dependency, full control over Info.plist. Output lands in dist/.
#
# Usage:
#   packaging/bundle.sh            # build, bundle, sign, and make a .dmg
#   packaging/bundle.sh --no-dmg   # stop after the signed .app
#   HOUDINI_FEATURES=ner packaging/bundle.sh   # build with the NER feature
#
# Signing identity: same self-signed cert as scripts/sign.sh (override with
# HOUDINI_SIGN_IDENTITY). A self-signed build runs on other Macs after a one-time
# right-click -> Open (Gatekeeper). For zero-friction install, sign with a
# "Developer ID Application" cert and notarize - see INSTALL.md.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

APP_NAME="Houdini"
BUNDLE_ID="ai.memfold.houdini"
EXE="houdini"
CERT_NAME="${HOUDINI_SIGN_IDENTITY:-Houdini Self-Signed}"
MIN_MACOS="14.0"
ENTITLEMENTS="$ROOT/packaging/entitlements.plist"
MASTER_ICON="$ROOT/packaging/appicon-1024.png"

MAKE_DMG=1
[[ "${1:-}" == "--no-dmg" ]] && MAKE_DMG=0

VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')"
FEATURES_ARG=()
[[ -n "${HOUDINI_FEATURES:-}" ]] && FEATURES_ARG=(--features "$HOUDINI_FEATURES")

DIST="$ROOT/dist"
APP="$DIST/$APP_NAME.app"
CONTENTS="$APP/Contents"

echo "==> Building release binary (v$VERSION)…"
# ${arr[@]+…} guard: expands to nothing when empty (bash 3.2 + set -u safe).
cargo build --release ${FEATURES_ARG[@]+"${FEATURES_ARG[@]}"}

echo "==> Assembling $APP_NAME.app…"
rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"
cp "target/release/$EXE" "$CONTENTS/MacOS/$EXE"

# --- App icon: master PNG -> .iconset -> AppIcon.icns ---
if [[ ! -f "$MASTER_ICON" ]]; then
  echo "error: $MASTER_ICON missing (run: python3 packaging/make_appicon.py)" >&2
  exit 1
fi
ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
for sz in 16 32 128 256 512; do
  sips -z "$sz" "$sz" "$MASTER_ICON" --out "$ICONSET/icon_${sz}x${sz}.png" >/dev/null
  sips -z $((sz * 2)) $((sz * 2)) "$MASTER_ICON" --out "$ICONSET/icon_${sz}x${sz}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$CONTENTS/Resources/AppIcon.icns"

# --- Info.plist ---
# LSUIElement=true -> menu-bar-only agent (no Dock icon, no app menu).
# The app reads local transcripts and observes its own sockets, so it needs NO
# TCC usage-description keys (no Screen Recording, no Accessibility).
cat >"$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>$EXE</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>$MIN_MACOS</string>
  <key>LSUIElement</key><true/>
  <key>NSHumanReadableCopyright</key><string>Houdini. Built by Rahul Biliyar, Memfold AI.</string>
</dict>
</plist>
PLIST

echo "==> Signing (identity: $CERT_NAME)…"
if ! security find-identity -p codesigning | grep -qF "$CERT_NAME"; then
  echo "error: no code-signing certificate named '$CERT_NAME'. See scripts/sign.sh header." >&2
  exit 1
fi
# One Mach-O, no nested frameworks, so signing the bundle seals the executable
# and resources. Hardened runtime + our entitlements.
codesign --force --options runtime --timestamp=none \
  --entitlements "$ENTITLEMENTS" \
  --sign "$CERT_NAME" \
  "$APP"
codesign --verify --strict --verbose=2 "$APP"
echo "    signed: $APP"

if [[ "$MAKE_DMG" -eq 1 ]]; then
  echo "==> Building .dmg…"
  DMG="$DIST/Houdini-$VERSION.dmg"
  STAGE="$(mktemp -d)/dmg"
  mkdir -p "$STAGE"
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications" # drag-to-install target
  # Ship the browser extension + its guide alongside the app so a teammate has
  # everything in one download (web-chat capture is opt-in; see docs/extension.md).
  cp -R "$ROOT/extension" "$STAGE/Browser Extension"
  cp "$ROOT/docs/extension.md" "$STAGE/Browser Extension/INSTALL-ME-FIRST.md"
  rm -f "$DMG"
  hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
  echo "    wrote: $DMG"
fi

echo
echo "Done. Gatekeeper note: a self-signed build is not notarized, so first"
echo "launch on another Mac needs right-click -> Open once. See docs/install.md."
