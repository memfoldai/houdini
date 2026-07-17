#!/usr/bin/env bash
#
# Sign the bare ai-usage-monitor binary with a STABLE self-signed identity, for
# the DEVELOPMENT workflow (running ./target/release/ai-usage-monitor directly).
# For a distributable app, use packaging/bundle.sh instead.
#
# Why this exists: the app needs no TCC grant anymore (it reads local transcripts
# and its own sockets), so signing is no longer required for detection to work. A
# stable self-signed identity is still useful — it keeps Gatekeeper quiet and
# gives the binary a constant identity for distribution/notarization — so this
# convenience remains for the development workflow.
#
# Usage:
#   scripts/sign.sh                 # build --release, then sign
#   scripts/sign.sh path/to/binary  # sign an already-built binary
#
# One-time: create the self-signed cert in Keychain Access:
#   Keychain Access → Certificate Assistant → Create a Certificate…
#     Name:             AI Usage Monitor Self-Signed
#     Identity Type:    Self Signed Root
#     Certificate Type: Code Signing
# Leave it in the login keychain. It does NOT need to be "trusted": a self-signed
# root shows as untrusted (CSSMERR_TP_NOT_TRUSTED) and is excluded from
# `security find-identity -v`, but `codesign` signs with it fine and TCC keys on
# it fine — trust only matters for OTHER machines verifying the signature.

set -euo pipefail

CERT_NAME="${AUM_SIGN_IDENTITY:-AI Usage Monitor Self-Signed}"
ENTITLEMENTS="$(cd "$(dirname "$0")/.." && pwd)/packaging/entitlements.plist"

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

# Match against ALL code-signing identities, not just `-v` (valid/trusted) ones:
# a self-signed root is untrusted by design, so `-v` would hide it even though
# codesign can use it. `find-identity -p codesigning` lists untrusted certs too.
if ! security find-identity -p codesigning | grep -qF "$CERT_NAME"; then
  echo "error: no code-signing certificate named '$CERT_NAME' in your keychain." >&2
  echo "       Create it (see this script's header), or set AUM_SIGN_IDENTITY to" >&2
  echo "       the name of an existing code-signing certificate." >&2
  echo >&2
  echo "       Certificates currently available for signing:" >&2
  security find-identity -p codesigning | sed 's/^/         /' >&2
  exit 1
fi

echo "Signing $BIN with '$CERT_NAME'…"
# --force replaces a prior signature on rebuild; --options runtime enables the
# hardened runtime; entitlements declare the capabilities the app uses.
codesign --force --options runtime \
  --entitlements "$ENTITLEMENTS" \
  --sign "$CERT_NAME" \
  "$BIN"

echo "Verifying signature…"
codesign --verify --verbose=2 "$BIN"

echo
echo "Signed with a stable identity. The app needs no TCC grant, so this is only"
echo "for a clean Gatekeeper/distribution identity — detection works unsigned too."
