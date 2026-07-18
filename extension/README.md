# Houdini — browser extension (web chats)

Captures **web AI chats** (ChatGPT, Claude on the web) that leave no local
transcript, and delivers them to the local monitor. It reads the **prompt from the
site's own API request** and the **reply from the rendered message in the DOM**
after the response finishes — and it works in **background tabs** because the
page's own code and DOM update regardless of tab focus.

Reading the reply from the rendered DOM (not the provider's internal streaming
format) is deliberate: that internal format is undocumented and changes, and it
silently broke reply capture once; the rendered message is stable and is what the
user actually saw.

## How it works

```
page (MAIN world)          isolated world        service worker         native app
interceptor.js  ──window──▶ relay.js ──runtime──▶ background.js ──stdio──▶ houdini
  reads prompt (request)      forwards              connectNative           --native-host:
  + reply (rendered DOM)      the captured          per message             validate → redact
  + id (page URL)             turn                                          → store → day file
```

- `interceptor.js` runs in the page's MAIN world at `document_start`, wraps
  `window.fetch` to detect each exchange and read the prompt from the request,
  then polls the DOM until the assistant message stabilizes to read the reply. The
  conversation id comes from the page URL (`/c/<id>`). See
  [Chrome: content script `world`](https://developer.chrome.com/docs/extensions/reference/manifest/content-scripts).
- `relay.js` (isolated world) bridges the MAIN world to the extension.
- `background.js` relays each captured exchange to the native host over
  [native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)
  (32-bit length-prefixed JSON on stdio).
- The Rust native host (`--native-host`) validates the tool, resolves the provider
  canonically, redacts, and stores it as a `web` session — grouped with the same
  provider's CLI/app usage.

**Local-only.** Nothing leaves the machine: the extension talks only to the local
native host, which has no network egress. This is the same interception technique
some malicious extensions have abused to *exfiltrate* chats — it is legitimate
here only because it is local, redacted, and installed per consent.

## Install (per machine, internal study)

1. Build the app and register the native host for every Chromium browser present:
   ```bash
   cargo build --release
   ./target/release/houdini --install-browser-host
   ```
2. Load this folder as an unpacked extension: `chrome://extensions` →
   **Developer mode** → **Load unpacked** → select `extension/`. The id must be
   `jphmlmjmieilhimgemjanlkgfommlife` (fixed by the `key` in `manifest.json`, so it
   matches the host manifest's `allowed_origins`).
3. Open ChatGPT or Claude on the web and send a message. The prompt/reply lands in
   the monitor's day file as a `web` session.

Remove with `./target/release/houdini --uninstall-browser-host` and
removing the unpacked extension.

## Supported sites and the honest caveat

| Site | Prompt | Reply | Conversation id |
|---|---|---|---|
| `chatgpt.com` / `chat.openai.com` | `/backend-api/conversation` request `messages[].content.parts` | rendered `[data-message-author-role="assistant"]` | page URL `/c/<id>` |
| `claude.ai` | `…/completion` request `prompt` | rendered `.font-claude-message` | page URL `/chat/<id>` |

The request shapes and DOM selectors are **reverse-engineered, not official
contracts**, so a site redesign can need a small fix — each is an isolated function
in `interceptor.js` that captures nothing (rather than garbage) when it doesn't
match, logging a console warning so the gap is visible. **The DOM selectors need
one live confirmation in a logged-in browser.** Gemini uses an obfuscated batch
transport and is intentionally not implemented.

## Development & versioning

The extension `version` in `manifest.json` tracks the app version — they are a
matched pair (same native-messaging protocol) and are upgraded together.

Validate the parsers without a browser (CI runs this too):

```bash
node extension/test/interceptor.test.mjs
```

It drives the real `interceptor.js` against a stubbed page (request body + a
rendered assistant element) and asserts the extracted prompt/reply/conversation-id.
