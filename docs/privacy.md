# Privacy & data

Houdini records AI usage for a consenting internal study. This is the single,
authoritative description of what it collects and what happens to that data. For
how the pieces fit together see [architecture.md](architecture.md); for how to
report a concern see [../SECURITY.md](../SECURITY.md).

## What is recorded

Houdini keeps two kinds of local record.

**AI turns.** For each AI turn: the **provider** (`anthropic`, `openai`, …),
**tool** (`claude-code`, `chatgpt-web`, …), **surface** (`cli`/`web`), **model**,
a **session id**, a timestamp, the **role** (user/assistant), and the **redacted
message text**.

**App actions (agent vs. human).** For each recognized action in a tracked app: the
**actor** (`agent` or `human`), the **app** (e.g. `mail.google.com`), the
**action** verb (`send`, `archive`, …), whether it changed state
(`mutating`/`read_only`), a timestamp, and a **redacted** detail. This is what
powers agent-vs-human attribution. Agent actions are read from the agent's own
local transcripts; human actions are reported by the browser extension on the
tracked sites. Only the action itself is recorded — its verb and the control's
label — never the emails, files, or documents it acts on.

Sources: CLI/agent transcripts already on disk (Claude Code, Codex,
OpenClaw/almaclaw) and, if the extension is loaded, web chats (ChatGPT, Claude,
Gemini) plus Google Workspace app actions (Gmail, Drive, Docs, Sheets, Slides,
Calendar).

## What is *not* recorded

- **No screen capture, no OCR, no Accessibility, no network monitoring.** Houdini
  requests no TCC permission and reads no pixels or packets.
- **No native desktop *chat* apps.** ChatGPT.app/Claude.app conversations are
  server-side/encrypted and out of scope.
- **No raw secrets or structured PII.** Those are removed before storage (below).

## The guarantees

- **Local-only.** No network egress anywhere except the updater (GitHub) and the
  extension talking to the *local* app. Nothing uploads.
- **Encrypted at rest.** The store is an encrypted SQLite database (SQLCipher,
  AES-256). The key is generated once and held in the **macOS Keychain**, never on
  disk beside the data. Nothing readable sits in a folder.
- **Redaction is a hard gate.** Content is redacted *before* it is written:
  provider API keys, private keys, emails, Luhn-checked card numbers, SSNs,
  phones (`src/redact.rs`). The store never holds raw content, web chats included.
- **Identity is in the clear on purpose.** The provider/tool *is* the research
  signal, so it is stored plainly; only message content is redacted. The one
  per-install value is a random `install_id` (device id), which reveals nothing.
- **Consent and pause.** Installed per person with a visible menu-bar indicator;
  **Take a break** stops all recording (transcripts and web chats) while paused.

## Export

The menu's **Export my data…** writes a flat, OLAP-ready snapshot, decrypted on
demand, never written automatically. It produces two files:

**`data/interactions.jsonl`** — one row per AI-chat message:

```json
{"schema":"aum/3","kind":"interaction","event_id":"<device>:<session>:0",
 "device":"…","day":"2026-07-16","ts_ms":…,"provider":"anthropic",
 "tool":"claude-code","surface":"cli","model":"claude-sonnet-5",
 "session_id":"…","turn_index":0,"role":"user","text":"…","text_chars":42}
```

**`data/actions.jsonl`** — one row per attributed app action (agent vs. human):

```json
{"schema":"aum/3","kind":"action","event_id":"<device>:<source>:<ext_id>",
 "device":"…","day":"2026-07-16","ts_ms":…,"actor":"agent",
 "app":"mail.google.com","source":"almaclaw","tool":"bdc__cua",
 "action":"type_text","action_kind":"mutating","session_id":"…","target":"…"}
```

`actor` is `agent`, `human`, or `unknown`; `action_kind` is `mutating` or
`read_only`; `app` and `target` are omitted when unknown. Every row in both files
carries a stable `event_id` and the device id, so exports from any number of
machines merge with a plain `read_json_auto`. Provider grouping and clustering
happen at analysis time, never in the app. See [grouping.md](grouping.md).

## Retention & deletion

The database is the only durable store and grows over time by design (it keeps
everything; it is text-only, single-digit MB/day). To delete data, quit the
app and remove the data directory
(`~/Library/Application Support/ai.memfold.houdini/`) and the Keychain item
`ai.memfold.houdini`; the app starts fresh on next launch. There are no
permissions to revoke.
