# AI Usage Monitor — browser extension (Layer C)

Captures **web AI chats** (ChatGPT, Claude on the web) that leave no local
transcript, and delivers them to the local monitor. It intercepts the AI site's
**own API calls** — the reliable, documented technique — rather than scraping the
rendered DOM, and it works in **background tabs** because the page's own code runs
regardless of tab focus.

## How it works

```
page (MAIN world)          isolated world        service worker         native app
interceptor.js  ──window──▶ relay.js ──runtime──▶ background.js ──stdio──▶ ai-usage-monitor
  wraps fetch, reads          forwards              connectNative           --native-host:
  the conversation API        the captured          per message             validate → redact
  request + SSE reply         turn                                          → store → day file
```

- `interceptor.js` runs in the page's MAIN world at `document_start`, wraps
  `window.fetch`, and reads a **clone** of the response stream (never affecting
  the page). Per-site parsers extract the prompt (request body) and the streamed
  reply (SSE). See [Chrome: content script `world`](https://developer.chrome.com/docs/extensions/reference/manifest/content-scripts).
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
   ./target/release/ai-usage-monitor --install-browser-host
   ```
2. Load this folder as an unpacked extension: `chrome://extensions` →
   **Developer mode** → **Load unpacked** → select `extension/`. The id must be
   `jphmlmjmieilhimgemjanlkgfommlife` (fixed by the `key` in `manifest.json`, so it
   matches the host manifest's `allowed_origins`).
3. Open ChatGPT or Claude on the web and send a message. The prompt/reply lands in
   the monitor's day file as a `web` session.

Remove with `./target/release/ai-usage-monitor --uninstall-browser-host` and
removing the unpacked extension.

## Supported sites and the honest caveat

| Site | Status |
|---|---|
| `chatgpt.com` / `chat.openai.com` | prompt from request, reply from the cumulative SSE `message.content.parts` |
| `claude.ai` | prompt from request `prompt`, reply from SSE `completion` / `text_delta` |

These endpoint/SSE shapes are **reverse-engineered, not official contracts**, so a
site redesign can break a parser — each is a small, isolated, defensive function
in `interceptor.js` that fails silently (captures nothing) rather than storing
garbage. The parsers are validated against the documented shapes; **each needs one
live confirmation in a logged-in browser**, and adjusting one is a localized edit.
Gemini uses an obfuscated batch transport and is intentionally not implemented.

## Development & versioning

The extension `version` in `manifest.json` tracks the app version — they are a
matched pair (same native-messaging protocol) and are upgraded together.

Validate the parsers without a browser (CI runs this too):

```bash
node extension/test/interceptor.test.mjs
```

It drives the real `interceptor.js` against realistic ChatGPT/Claude SSE and
asserts the extracted prompt/reply/conversation-id.
