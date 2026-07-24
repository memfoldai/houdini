# Houdini: browser extension

Captures **web AI chats** (ChatGPT, Claude, Gemini) that leave no local
transcript, plus recognized **Google Workspace app actions** (Gmail, Drive,
Docs, Sheets, Slides, Calendar), and delivers them to the local Houdini app.
For chats it reads each exchange from the **rendered page** (the prompt and
reply the user actually saw) and works in **background tabs** because the DOM
updates regardless of tab focus. For Workspace actions it uses clicked control
labels only to classify the action, then records only the normalized verb.

Reading from the rendered DOM rather than each site's internal, un-versioned
network API is deliberate: the API shapes are undocumented and change per deploy;
the rendered message is stable and uniform across sites.

## How it works

```
page (content scripts)       service worker        native app (forwarder)     app
capture*.js  ──runtime────▶ background.js ──stdio──▶ houdini --native-host ──socket──▶ Houdini
 chat: stable exchange       connectNative           forward only              redact → store
 action: recognized click    per message             (no DB, no keychain)      (single writer)
```

- `capture.js` (one ISOLATED content script) picks the adapter for the current
  host, polls the latest assistant message until its text stops changing, then
  sends the latest `{user, assistant}` exchange to the background worker. The
  conversation id comes from the page URL.
- `capture_actions.js` listens on allowlisted Workspace hosts and emits only
  recognized state-changing controls, normalized to action verbs such as `send`,
  `archive`, or `delete`.
- `background.js` forwards each exchange to the native host over
  [native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)
  (32-bit length-prefixed JSON on stdio).
- The native host is a thin **forwarder**: it pipes the message over a local
  socket to the running Houdini app, which redacts and stores it. Only the app
  touches the database and Keychain (see
  [../docs/architecture.md](../docs/architecture.md)).

**Local-only.** The extension has no network permission and talks only to the
local native host. This is the same content-reading mechanism some malicious
extensions abuse to *exfiltrate* chats. It is legitimate here only because it is
local, redacted, and installed per consent.

## Registry: one adapter per site

The core in `capture.js` is site-agnostic; each site is one descriptor. Adding a
site is adding one entry.

| Site | User selector | Assistant selector | Conversation id |
|---|---|---|---|
| `chatgpt.com` / `chat.openai.com` | `[data-message-author-role="user"]` | `[data-message-author-role="assistant"]` | URL `/c/<id>` |
| `claude.ai` | `[data-testid="user-message"]` | `.font-claude-message` | URL `/chat/<id>` |
| `gemini.google.com` | `user-query .query-text` | `message-content .markdown-main-panel` | URL `/app/<id>` |

These selectors are **reverse-engineered, not official contracts**, so a redesign
can need a small fix. An adapter captures nothing (rather than garbage) when it
doesn't match. **They need one live confirmation in a logged-in browser** whenever
a site changes.

## Workspace action capture

`capture_actions.js` runs only on the Workspace hosts listed in
`manifest.json`. It classifies clicked controls by accessible labels
(`aria-label` / `data-tooltip`) and ignores unrecognized controls. These labels
are also reverse-engineered UI contracts and need live confirmation when Google
changes the toolbar or compose controls.

## Install

The app auto-registers the native-messaging host for every Chromium browser on
launch, so you only load the extension: `chrome://extensions` → **Developer mode**
→ **Load unpacked** → select `extension/`. The id is fixed
(`jphmlmjmieilhimgemjanlkgfommlife`, from the `key` in `manifest.json`) so it
matches the host manifest's `allowed_origins`. The teammate-facing guide is
[../docs/extension.md](../docs/extension.md).

## Development & versioning

The extension `version` tracks the app version: a matched pair on the same
native-messaging protocol, upgraded together. Validate the capture logic without a
browser (CI runs this too):

```bash
node --test extension/test/*.test.mjs
```
