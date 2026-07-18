# Architecture

How Houdini works and where things live. For the data/consent model see
[privacy.md](privacy.md); for building and running see
[../CONTRIBUTING.md](../CONTRIBUTING.md).

## Bird's-eye

Houdini is a menu-bar-only macOS app (Rust) that records AI-chat prompts and
replies from two sources, normalizes them into one uniform record, and stores
them in an encrypted local database. There is no screen capture and no network
monitoring; every signal is a real message read from a place the user already
has access to.

```
CLI / agent tools ──▶ local transcripts ─┐
                                         ├─▶  Houdini app  ──▶  encrypted SQLite
web chats ──▶ browser extension ──▶ native host ─┘  (single writer)      store
```

## Codemap

Portable core (no macOS frameworks, so `cargo test` runs anywhere) lives in the
library; the menu-bar shell is the binary.

| Path | Responsibility |
|---|---|
| `src/ingest/` | Transcript adapters (`claude_code`, `codex`, `openclaw`) + the `Ingestor` that scans and normalizes them. Adding a tool is adding one adapter. |
| `src/webingest.rs` | Parse, redact, and store a web-chat message; the socket framing shared with the native host. |
| `src/store.rs` | The encrypted SQLite store (SQLCipher) and schema migrations. |
| `src/keychain.rs` | Fetches the DB key from the macOS Keychain. |
| `src/redact.rs` | Deterministic redaction of secrets/PII, applied before any text is stored. |
| `src/updater.rs` | Over-the-air update from GitHub Releases. |
| `src/config.rs` | Paths and `config.json`. |
| `src/app.rs` | The binary: tray UI, timer tick, FSEvents watcher, and the web-capture socket listener. |
| `src/nativehost.rs` | The `--native-host` forwarder invoked by the browser. |
| `src/browserhost.rs` | Registers the native-messaging host manifest for each Chromium browser. |
| `extension/` | The browser extension (`capture.js` + `background.js`). |

## Data flow

**Transcripts (CLI/agent).** `Ingestor::poll` scans each adapter's files, parses
new turns, redacts them, and writes them to the store. FSEvents (`notify`) makes
the scan fire the moment a transcript changes, so the menu-bar indicator reacts in
real time; a slow poll (`transcript_poll_ms`) is the fallback.

**Web chats (single writer).** The extension's content script (`capture.js`)
reads the rendered exchange in the user's tab and sends it to the extension's
background worker, which forwards it to the `houdini --native-host` process the
browser spawns. That process is a thin forwarder: it pipes the message over a
local Unix socket to the **running app**, and only the app opens the Keychain
(once, at launch) and writes the store. This single-writer design is why web
capture never triggers a per-message Keychain prompt and cannot race the app on
the database.

Both paths converge on the same normalized record and the same store, so an
export is uniform regardless of source (see [privacy.md](privacy.md#export) for
the row shape). Provider grouping and semantic clustering are **analysis-time**
jobs over an export, never in the daemon — see [grouping.md](grouping.md).

## The store

One encrypted SQLite database (SQLCipher, AES-256); the key is generated once and
kept in the macOS Keychain, so nothing readable sits on disk. It is the single
source of truth and stays queryable for on-device analytics.

Schema changes are **forward-only and additive**: a fresh database gets the
current schema; an existing one is stepped forward one version at a time, each
step running its DDL and the `PRAGMA user_version` bump in a single transaction.
SQLite rolls both back together on failure, so a crash or restart mid-migration
leaves the old version intact and re-runs cleanly — no data is ever dropped.

## Over-the-air updates

The installed app checks GitHub Releases on launch and every few hours. When a
newer release exists it downloads the signed `.dmg`, verifies its signature, swaps
the `/Applications` bundle, and relaunches — no token needed, because the repo is
public. Updates never touch the data directory or the Keychain, so upgrading
cannot lose data.

## Invariants

- **Single writer.** Only the app process opens the Keychain and writes the store;
  the native host only forwards. Do not make the native host open the database.
- **Redaction is a gate, not a filter.** Text is redacted before it is stored,
  never after.
- **Local-only.** No code path makes a network call except the updater (GitHub)
  and the browser extension talking to the *local* native host.
- **Migrations only add.** Never drop or rebuild a table on version bump; append a
  step to `MIGRATIONS`.
- **Identity in the clear, content redacted.** The provider/tool is the research
  signal and is stored plainly; message content is always redacted.
