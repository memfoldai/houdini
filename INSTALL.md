# Install & distribute

How to turn the source into an installable app and get it onto teammates'
machines. For what the app is, see [README.md](README.md); for proving a build
works, see [VERIFICATION.md](VERIFICATION.md).

There are two audiences here: the **maintainer** who builds and signs the app,
and **teammates** who install the `.dmg`. Teammates only need the last section.

---

## 1. One-time: a signing certificate (maintainer)

The app no longer needs any TCC permission (no Screen Recording, no
Accessibility) — it reads local transcript files and observes its own sockets. A
stable code-signing identity is still worth having for a clean install, a smooth
Gatekeeper experience, and notarization, so a rebuild never re-triggers a
first-launch prompt.

Create a self-signed one (free, internal use):

1. Keychain Access → Certificate Assistant → **Create a Certificate…**
2. Name `AI Usage Monitor Self-Signed`, Identity Type **Self Signed Root**,
   Certificate Type **Code Signing**. Create it; leave it in the login keychain.

It will show as untrusted (`CSSMERR_TP_NOT_TRUSTED`) — that is expected and fine.
`codesign` uses it regardless; trust only matters for *other* machines verifying
the signature, which notarization (below) handles.

## 2. Build the app + installer (maintainer)

```bash
packaging/bundle.sh            # → dist/AI Usage Monitor.app + dist/AI-Usage-Monitor-<v>.dmg
packaging/bundle.sh --no-dmg   # just the signed .app
AUM_FEATURES=ner packaging/bundle.sh   # include the optional NER layer
```

The script builds the release binary, generates the app icon (`.icns` from
`packaging/appicon-1024.png` — regenerate with `python3 packaging/make_appicon.py`
if the design changes), writes `Info.plist`, signs with hardened runtime, and
produces the `.dmg`. Override the identity with `AUM_SIGN_IDENTITY`.

## 3. Choose a distribution path (maintainer)

| | Self-signed (default) | Developer ID + notarized |
|---|---|---|
| Cost | Free | Apple Developer Program ($99/yr) |
| Teammate's first launch | Right-click → **Open** once (Gatekeeper) | Double-click, no prompt |
| Best for | A few internal machines | Smoother rollout / many machines |

Both are legitimate. Self-signed is enough for a small consenting team; notarize
when the Gatekeeper right-click step becomes a support burden.

### Notarizing (only if you have a Developer ID)

Sign with a **Developer ID Application** certificate and a secure timestamp
(edit `bundle.sh`: set `AUM_SIGN_IDENTITY` to the Developer ID cert and change
`--timestamp=none` to `--timestamp`), then:

```bash
# One-time: store an app-specific password (from appleid.apple.com) in the keychain.
xcrun notarytool store-credentials "AUM-notary" \
  --apple-id "you@example.com" --team-id "YOURTEAMID" --password "app-specific-pw"

# Per release: submit the .dmg, wait for the result, then staple the ticket.
xcrun notarytool submit "dist/AI-Usage-Monitor-<v>.dmg" --keychain-profile "AUM-notary" --wait
xcrun stapler staple "dist/AI Usage Monitor.app"
xcrun stapler staple "dist/AI-Usage-Monitor-<v>.dmg"
```

Stapling lets Gatekeeper verify offline, so teammates launch with no warning.

---

## 4. Install (teammate)

1. Open the `.dmg` and drag **AI Usage Monitor** to **Applications**.
2. Launch it:
   - Notarized build: double-click.
   - Self-signed build: **right-click → Open**, then **Open** in the dialog.
     (Only the first launch; macOS remembers thereafter.)
3. A ring appears in the menu bar. **No permission prompt** — the app reads the
   AI tools' own local logs and observes its own network connections; it never
   asks for Screen Recording or Accessibility.

There is no window. Click the menu-bar icon to see live status — current state,
AI sessions recorded in the last 24 h, and when the last activity was. The icon
briefly fills to a solid disc when a new interaction is recorded and shows a dot
when an AI is in use nearby. Data is stored automatically (redacted) to day
files; **Show my data** reveals the folder, **Quit** stops it.

To confirm detection end-to-end (and audit redaction before trusting any data),
run [VERIFICATION.md](VERIFICATION.md).

## 5. Optional: web-chat capture (browser extension)

The app catches AI apps and CLIs on its own. To also capture **web** ChatGPT/Claude,
install the Chromium extension (see [extension/README.md](extension/README.md)):

```bash
ai-usage-monitor --install-browser-host    # registers the local host for every Chromium browser
```

Then in each browser: `chrome://extensions` → **Developer mode** → **Load unpacked**
→ select the `extension/` folder. Send one web AI message to confirm it appears in
your day file.

The extension and app are a matched pair and **share a version** (both `0.4.0`):
the extension's fixed id (`jphmlmjmieilhimgemjanlkgfommlife`) is allowlisted by the
host manifest `--install-browser-host` writes, and they talk only over local native
messaging. Upgrade them together. Remove with `--uninstall-browser-host` and by
removing the unpacked extension.

## Uninstall

Quit from the menu, drag the app to the Trash, and remove its local data:
`~/Library/Application Support/ai.memfold.ai-usage-monitor/`. There are no
permissions to revoke.
