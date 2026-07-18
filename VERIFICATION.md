# Verification — human-gated steps

The portable core (ingestion, attribution, redaction, store, export) is covered
by `cargo test` and runs anywhere. This checklist is what a person runs once on a
real Mac to confirm the two live detectors work end-to-end and the data is safe
to share. It is much shorter than before: there is **no permission to grant and
no false-positive tuning loop**, because detection no longer reads the screen.

Do these in order. Each has an explicit pass condition.

---

## 0. Build and probe

```bash
cargo test                 # portable core + integration must be green
cargo build --release
./target/release/houdini --diagnose
```

**Pass:** `--diagnose` lists your transcript tools with non-zero counts if you
have used them (e.g. `claude-code  N file(s) → M session(s)`). No content is
printed.

---

## 1. A CLI/agent interaction is ingested

1. Run a prompt in Claude Code or Codex (e.g. "explain regenerative agriculture").
2. Launch the app: `./target/release/houdini` (a ring icon appears in
   the menu bar).
3. Wait ~20 s (one ingest + one flush), then **Show my data**.

**Pass:** today's file
`~/Library/Application Support/ai.memfold.houdini/data/interactions/YYYY-MM-DD.jsonl`
contains flat `"kind":"interaction"` rows (one per turn) whose
`provider`/`tool`/`surface`/`model` are correct, with your prompt (`role":"user"`)
and the reply (`role":"assistant"`). The icon fills to a disc while recording.

---

## 2. A web chat is captured (needs the extension)

With the browser extension installed (see [extension/README.md](extension/README.md)),
send a message in ChatGPT or Claude on the web.

**Pass:** the interactions file gains a `tool":"chatgpt-web"` (or `claude-web`) row
pair — **both** a `role":"user"` and a `role":"assistant"` row for the exchange,
grouped under one `session_id`. If the assistant row is missing, the DOM selector
needs updating for the current site (see the extension README).

---

## 3. Non-AI activity must NOT be recorded (false-positive gate)

With Slack, an editor, email, and unrelated browser tabs open and busy:

**Pass:** no `interaction` row is written for any of them. Only real AI transcripts
and matched web-chat pages are recorded — everything else is structurally
invisible.

---

## 4. Redaction audit — seeded secret (safety gate)

Before sharing any data, prove redaction catches planted values.

1. In Claude Code or Codex, send a prompt containing a **fake** AWS-shaped key and
   a **fake** email, e.g. "note key AKIAIOSFODNN7EXAMPLE, mail jane@example.com".
2. Let it ingest (~20 s), then **Show my data**.

**Pass:** in today's day file, neither `AKIAIOSFODNN7EXAMPLE` nor
`jane@example.com` appears as raw text; each is a `[REDACTED:…]` placeholder. If
any raw value survives, **do not share the data** — file the gap first.
(`src/redact.rs` unit-tests these exact shapes; this confirms it end-to-end.)

---

## 5. Optional: NER redaction layer (feature `ner`)

Skip unless you enabled the NER layer — [docs/NER.md](docs/NER.md) owns the setup.
When running, it should replace a planted person name with `[REDACTED:NER:PERSON]`
and, pointed at a missing model, fail closed (log the failure, continue with
deterministic-only redaction, never crash).

---

## Known limits (state these when sharing results)

- **Native desktop apps** (ChatGPT.app, Claude.app) keep their content
  server-side and are not captured — use the CLI/agent tools or the web extension.
- **Web extraction is per-site and reverse-engineered**; a site redesign can need
  a small selector fix. Gemini web is not parsed yet.
- **Adapters are per-tool.** A CLI tool without an adapter is not ingested; adding
  one is a small change.
