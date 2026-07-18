# Privacy & data

Houdini records AI usage for a consenting internal study. This is the single,
authoritative description of what it collects and what happens to that data. For
how the pieces fit together see [architecture.md](architecture.md); for how to
report a concern see [../SECURITY.md](../SECURITY.md).

## What is recorded

For each AI turn Houdini captures a single normalized record: the **provider**
(`anthropic`, `openai`, …), **tool** (`claude-code`, `chatgpt-web`, …),
**surface** (`cli`/`web`), **model**, a **session id**, a timestamp, the **role**
(user/assistant), and the **redacted message text**. Nothing that is not a real
AI message is recorded.

Sources: CLI/agent transcripts the user already has on disk (Claude Code, Codex,
OpenClaw) and — if the extension is loaded — web chats (ChatGPT, Claude, Gemini).

## What is *not* recorded

- **No screen capture, no OCR, no Accessibility, no network monitoring.** Houdini
  requests no TCC permission and reads no pixels or packets.
- **No native desktop *chat* apps.** ChatGPT.app/Claude.app conversations are
  server-side/encrypted and out of scope (see the README's honest limits).
- **No raw secrets or structured PII** — those are removed before storage (below).

## The guarantees

- **Local-only.** No network egress anywhere except the updater (GitHub) and the
  extension talking to the *local* app. Nothing uploads.
- **Encrypted at rest.** The store is an encrypted SQLite database (SQLCipher,
  AES-256). The key is generated once and held in the **macOS Keychain**, never on
  disk beside the data. Nothing readable sits in a folder.
- **Redaction is a hard gate.** Content is redacted *before* it is written —
  provider API keys, private keys, emails, Luhn-checked card numbers, SSNs,
  phones (`src/redact.rs`). The store never holds raw content, web chats included.
- **Identity is in the clear on purpose.** For a consenting study the provider/tool
  *is* the research signal, so it is stored plainly; only message content is
  redacted. The one per-install value is a random `install_id` (device id), which
  reveals nothing.
- **Consent and pause.** Installed per person with a visible menu-bar indicator;
  **Take a break** stops all recording — transcripts and web chats — while paused.

## Export

The menu's **Export my data…** writes a flat, OLAP-ready snapshot to
`data/interactions.jsonl` (one row per message) and reveals it — decrypted on
demand, never written automatically:

```json
{"schema":"aum/3","kind":"interaction","event_id":"<device>:<session>:0",
 "device":"…","day":"2026-07-16","ts_ms":…,"provider":"anthropic",
 "tool":"claude-code","surface":"cli","model":"claude-sonnet-5",
 "session_id":"…","turn_index":0,"role":"user","text":"…","text_chars":42}
```

Every source produces this exact row shape, and each row carries a stable
`event_id` and the device id, so exports from any number of machines merge with a
plain `read_json_auto`. Provider grouping and clustering happen at analysis time,
never in the app — see [grouping.md](grouping.md).

## Retention & deletion

The database is the only durable store and grows over time by design (the study
keeps everything; it is text-only, single-digit MB/day). To delete data, quit the
app and remove the data directory
(`~/Library/Application Support/ai.memfold.houdini/`) and the Keychain item
`ai.memfold.houdini`; the app starts fresh on next launch. There are no
permissions to revoke.
