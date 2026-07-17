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
./target/release/ai-usage-monitor --diagnose
```

**Pass:** `--diagnose` prints two sections. Layer A lists your transcript tools
with non-zero counts if you have used them (e.g. `claude-code  N file(s) → M
session(s)`). Layer B lists AI network connections active right now (open an AI
app or run an AI CLI first, or it will be empty). No content is printed.

---

## 1. Layer A — a real interaction is ingested

1. Run a prompt in Claude Code or Codex (e.g. "explain regenerative agriculture").
2. Launch the app: `./target/release/ai-usage-monitor` (a ring icon appears in
   the menu bar).
3. Wait ~20 s (one ingest + one flush), then **Show my data**.

**Pass:** today's day file
`~/Library/Application Support/ai.memfold.ai-usage-monitor/data/YYYY-MM-DD.jsonl`
contains an `"kind":"interaction"` line whose `provider`/`tool`/`surface`/`model`
are correct and whose `turns` hold your prompt and the reply. The icon briefly
shows the solid disc when a new interaction is recorded.

---

## 2. Layer B — apps and CLIs are detected across desktops

With ChatGPT (app), Claude (app), and/or a running AI CLI open — including on
other desktops/Spaces — run `--diagnose` again (or watch the running app).

**Pass:** each open AI app/CLI appears in the Layer B list with the right
provider (`ChatGPT → openai`, `Claude → anthropic`, `codex → openai`, …).
Note the documented gap: **ChatGPT/Gemini in a browser tab will not appear** (CDN
IPs are not provider-identifying) — their native apps do.

---

## 3. Non-AI activity must NOT be recorded (false-positive gate)

With Slack, an editor, email, and unrelated browser tabs open and busy:

**Pass:** none of them appear in the Layer B list, and no `interaction`/`presence`
record is written for them. (Slack has no AI transcript and its endpoints are not
AI providers, so it is structurally invisible — this is the class of false
positive the old screen-scraper produced.)

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

- **Browser web chats** (ChatGPT/Gemini) are not attributed by the network layer;
  only their native apps/CLIs are. Reliable browser-content capture is a future
  browser-extension layer.
- **Network presence** means "an AI tool was connected/active," coarser than a
  discrete message; the transcript layer supplies exact interactions.
- **Adapters are per-tool.** A tool without an adapter is not ingested (its
  network presence is still observed generically). Adding one is a small change.
