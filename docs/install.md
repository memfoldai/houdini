# Install & distribute

How to build, sign, distribute, and install Houdini. For what the app is, see
[../README.md](../README.md); for how it works, [architecture.md](architecture.md);
for verifying a build, [../CONTRIBUTING.md](../CONTRIBUTING.md).

There are two audiences: the **maintainer** who builds and signs, and
**teammates** who install the `.dmg`. Teammates only need [§4](#4-install-teammate)
and [§5](#5-optional-browser-capture-browser-extension).

---

## 1. One-time: a signing certificate (maintainer)

Houdini needs no TCC permission: it reads local transcript files and its own
sockets. A stable code-signing identity is still worth having, so a rebuild never
re-triggers a Gatekeeper prompt. Create a self-signed one (free, internal use):

1. Keychain Access → Certificate Assistant → **Create a Certificate…**
2. Name `Houdini Self-Signed`, Identity Type **Self Signed Root**, Certificate
   Type **Code Signing**. Create it; leave it in the login keychain.

It shows as untrusted (`CSSMERR_TP_NOT_TRUSTED`). That is expected; `codesign`
uses it regardless. Trust only matters for *other* machines, which notarization
handles.

## 2. Build the app + installer (maintainer)

```bash
packaging/bundle.sh            # → dist/Houdini.app + dist/Houdini-<v>.dmg
packaging/bundle.sh --no-dmg   # just the signed .app
HOUDINI_FEATURES=ner packaging/bundle.sh   # include the optional NER layer
```

The script builds the release binary, generates the icon, writes `Info.plist`,
signs with hardened runtime, and produces the `.dmg` (which also bundles the
browser extension + its guide). Override the identity with `HOUDINI_SIGN_IDENTITY`.

## 3. Choose a distribution path (maintainer)

| | Self-signed (default) | Developer ID + notarized |
|---|---|---|
| Cost | Free | Apple Developer Program ($99/yr) |
| Teammate's first launch | Approve once in System Settings | Double-click, no prompt |
| Best for | A few internal machines | Smoother rollout / many machines |

Self-signed is enough for a small internal team; notarize when the right-click
step becomes a support burden.

### Notarizing (only if you have a Developer ID)

Sign with a **Developer ID Application** certificate and a secure timestamp (in
`bundle.sh` set `HOUDINI_SIGN_IDENTITY` to the Developer ID cert and change
`--timestamp=none` to `--timestamp`), then:

```bash
# One-time: store an app-specific password (from appleid.apple.com).
xcrun notarytool store-credentials "houdini-notary" \
  --apple-id "you@example.com" --team-id "YOURTEAMID" --password "app-specific-pw"

# Per release: submit, wait, staple.
xcrun notarytool submit "dist/Houdini-<v>.dmg" --keychain-profile "houdini-notary" --wait
xcrun stapler staple "dist/Houdini.app"
xcrun stapler staple "dist/Houdini-<v>.dmg"
```

---

## 4. Install (teammate)

1. Open the `.dmg` and drag **Houdini** to **Applications**.
2. Launch it. A self-signed build is blocked on first open, so go to
   **System Settings → Privacy & Security**, scroll to Security, and click
   **Open Anyway** next to Houdini, then confirm (first launch only; notarized
   builds just double-click).
3. A ring appears in the menu bar. **No permission prompt**; see
   [privacy.md](privacy.md) for why.

There is no window; click the icon for status and the menu. Data handling and
export are described in [privacy.md](privacy.md).

## 5. Optional: browser capture (browser extension)

To also capture web ChatGPT/Claude/Gemini and Google Workspace app actions, load
the Chromium extension. The DMG ships it in a **Browser Extension** folder with a step-by-step
**INSTALL-ME-FIRST.md** (the teammate guide is
[extension.md](extension.md)). In short: `chrome://extensions` →
**Developer mode** → **Load unpacked** → select the folder, then send one web AI
message and click one recognized Workspace control to confirm.

**No terminal step is needed**: the app registers the local native-messaging host
on every launch. The extension and app **share a version** (the extension's fixed
id `jphmlmjmieilhimgemjanlkgfommlife` is allowlisted by the host manifest); this
build isn't on the Chrome Web Store, so re-load the folder on a new version.

## 6. Over-the-air updates (maintainer release step)

The installed app updates itself from **GitHub Releases** (see the mechanism in
[architecture.md](architecture.md#over-the-air-updates)). It works only while the
`memfoldai/houdini` repo is **public** (the updater reads the releases API
unauthenticated). **Each release MUST attach the `.dmg`:**

```bash
packaging/bundle.sh
gh release upload vX.Y.Z dist/Houdini-X.Y.Z.dmg
```

Bump the extension version in lockstep when the native-messaging message shape
changes (it rarely does).

## Uninstall

Quit from the menu, drag the app to the Trash, and remove
`~/Library/Application Support/ai.memfold.houdini/` (and the Keychain item
`ai.memfold.houdini`). There are no permissions to revoke.
