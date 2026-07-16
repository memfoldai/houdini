#!/usr/bin/env bash
#
# Sign the ai-usage-monitor binary with a STABLE self-signed identity.
#
# Why this exists: macOS TCC (the Accessibility + Screen Recording grants this
# app needs) keys its grants on the binary's code-signing identity / CDHash. An
# unsigned or ad-hoc-signed (`codesign -s -`) binary gets a NEW hash on every
# rebuild, so every rebuild silently drops the grants and the app captures
# nothing. Signing with a stable self-signed certificate gives the binary a
# constant identity, so a grant made once survives rebuilds.
#
# This is for INTERNAL, consenting installs only (the study's own team machines).
# It is NOT notarized and NOT for distribution.
#
# Usage:
#   scripts/sign.sh                 # build --release, then sign
#   scripts/sign.sh path/to/binary  # sign an already-built binary
#
# One-time: create the self-signed cert in Keychain Access:
#   Keychain Access → Certificate Assistant → Create a Certificate…
#     Name:            AI Usage Monitor Self-Signed
#     Identity Type:   Self Signed Root
#     Certificate Type: Code Signing
#   (Leave it in the login keychain; trust defaults are fine for local use.)
# Then keep CERT_NAME below in sync with that certificate's name.

set -euo pipefail

CERT_NAME="${AUM_SIGN_IDENTITY:-AI Usage Monitor Self-Signed}"
ENTITLEMENTS="$(cd "$(dirname "$0")/.." && pwd)/scripts/entitlements.plist"

BIN="${1:-}"
if [[ -z "$BIN" ]]; then
  echo "Building release binary…"
  cargo build --release
  BIN="target/release/ai-usage-monitor"
fi

if [[ ! -f "$BIN" ]]; then
  echo "error: binary not found at $BIN" >&2
  exit 1
fi

if ! security find-identity -v -p codesigning | grep -q "$CERT_NAME"; then
  echo "error: code-signing identity '$CERT_NAME' not found in your keychain." >&2
  echo "       Create it first (see the header of this script), or set" >&2
  echo "       AUM_SIGN_IDENTITY to the name of an existing code-signing cert." >&2
  exit 1
fi

echo "Signing $BIN with '$CERT_NAME'…"
# --force so re-signing a rebuilt binary replaces the previous signature.
# --options runtime enables the hardened runtime; entitlements declare the
# capabilities the app actually uses.
codesign --force --options runtime \
  --entitlements "$ENTITLEMENTS" \
  --sign "$CERT_NAME" \
  "$BIN"

echo "Verifying signature…"
codesign --verify --verbose=2 "$BIN"
codesign --display --entitlements - "$BIN" >/dev/null

echo
echo "Signed. Because the identity is stable, the Accessibility and Screen"
echo "Recording grants you make for this binary will persist across rebuilds"
echo "(as long as you re-run this script after each build)."
