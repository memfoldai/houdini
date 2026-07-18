# ai-usage-monitor

A minimal, menu-bar-only macOS app that records **what people use AI for** — the
observation instrument for an internal AI-usage study. It reads the actual
prompts and replies from AI tools' own local logs and web chats; it does **not**
capture the screen and does **not** watch network traffic.

It is deliberately small and standalone: its own repo, no cloud, no dashboards,
no coupling to any other product. It runs on the study team's own machines with
per-install consent and a visible menu-bar indicator.

## How it detects AI use

Two content sources, both reading the real messages — no screenshots, no OCR, no
guessing from pixels, and no content-free "an app was open" noise:

**Transcript ingestion (CLI/agent tools).** Coding agents already persist every
interaction to a structured local transcript. The monitor reads those directly,
so it gets the exact prompt and reply, real timestamps, the model, and a session
id — zero false positives, full coverage across desktops and Spaces. Adapters:

| Tool | Reads |
|---|---|
| Claude Code | `~/.claude/projects/*/*.jsonl` |
| Codex | `~/.codex/**/rollout-*.jsonl` |

Adding a tool is adding one small adapter; the rest of the pipeline is shared.

**Browser extension (web chats).** Web ChatGPT/Claude leave no local transcript.
An optional Chromium extension ([extension/](extension/README.md)) reads each
exchange — the prompt from the site's own API request, the reply from the rendered
message — and delivers it to the app over local native messaging, never over the
network. It works in background tabs. Covers ChatGPT and Claude web today.

Both sources produce the **same standardized record**: the actual prompt/response
turns with provider/tool/surface/model. Nothing that isn't a real AI message is
recorded.

### Honest limits

- **The browser extension is installed per browser**, and its per-site extraction
  tracks reverse-engineered page shapes, so a site redesign can need a small fix.
  Without it, web chats are uncaptured (CLI/agent tools are still captured).
- **Native desktop apps** (ChatGPT.app, Claude.app) keep their content
  server-side and aren't captured — use the CLI/agent tools or the web with the
  extension.
- **Gemini on the web** isn't parsed yet (obfuscated batch transport).

## Privacy model

- **Local-only.** No network egress anywhere in the code path. Nothing uploads.
- **No screen capture, no TCC permission, no network monitoring.** The app reads
  files the user already owns; the extension reads the page in the user's own
  browser. It never asks for Screen Recording or Accessibility.
- **Content is redacted** — offline, before anything touches disk — for secrets
  (provider API keys, private keys), emails, Luhn-checked cards, SSNs, phones.
- **Identity is kept in the clear** on purpose: for a consenting internal study
  the provider/tool (`anthropic`, `claude-code`) *is* the research signal. Only
  the message content is redacted.
- Pause anytime from the menu; while paused nothing new is recorded.

## Data format (OLAP-ready)

The local store is SQLite (the source of truth). New records are written
automatically — no manual export — into a day-partitioned JSONL **flat fact
table**, one row per message, so a warehouse reads it with no unnesting or joins:

```
data/interactions/YYYY-MM-DD.jsonl   # one row per message (turn)
```

Each row is flat and denormalized, has a stable `event_id` (so a re-load dedupes
trivially), and carries the day + device (so files from any number of machines
merge by concatenation). Turns are written **incrementally** — a growing session
appends only its new turns, never re-emitting the whole thing.

`data/interactions/YYYY-MM-DD.jsonl` — one row per turn:

```json
{"schema":"aum/3","kind":"interaction","event_id":"<device>:<session>:0",
 "device":"…","day":"2026-07-16","ts_ms":…,"provider":"anthropic",
 "tool":"claude-code","surface":"cli","model":"claude-sonnet-5",
 "session_id":"…","turn_index":0,"role":"user","text":"…","text_chars":42}
```

Every source (Claude Code, Codex, ChatGPT web, Claude web) produces this exact
row shape, so the table is uniform regardless of where the AI was used.

A query is a flat scan — e.g. DuckDB:
`SELECT provider, count(*) FROM read_json_auto('data/interactions/*.jsonl') WHERE role='assistant' GROUP BY 1`.
Provider grouping (Claude app + CLI + web → one entity) is deterministic at
ingest; higher-level semantic clustering (research vs build, topic) is an
analysis-time job over these files — see [docs/grouping.md](docs/grouping.md).

## Menu bar & status

The icon is a monochrome template glyph whose **shape** shows state (macOS tints
it): a **hollow ring** when quiet, a **filled disc** while AI activity is being
recorded (it decays back to the ring a while after the last interaction), and
**two bars** when paused. Click it for a header showing the app version, a plain
status line, a count of sessions recorded today, and:

- **Take a break** — for 15 minutes, an hour, or until you're back. While paused
  nothing is recorded.
- **Show my data** — reveals the day-partitioned data folder in Finder.
- **Quit**.

## Develop

Requires a recent stable Rust toolchain and macOS 14+.

```bash
cargo test                       # portable core + integration (runs anywhere)
cargo build --release
./target/release/ai-usage-monitor --diagnose   # one-shot: transcript counts
./target/release/ai-usage-monitor              # run the menu-bar app
```

`--diagnose` prints how many interactions each transcript adapter can read right
now — no content, just counts. (Web chats come via the extension; run the app.)

Signing is recommended for a stable install identity, but the app depends on no
TCC grant, so a rebuild never silently loses anything.

## Configuration

`~/Library/Application Support/ai.memfold.ai-usage-monitor/config.json` is created
on first run. Operator knobs:

| Key | Default | Purpose |
|---|---|---|
| `transcript_poll_ms` | 5000 | How often to scan transcripts for new interactions |
| `flush_ms` | 15000 | How often to write new records to day files |
| `ner_model_dir` | unset | Enables the [NER redaction layer](docs/NER.md) (`--features ner`) |

`install_id` is a random per-install device id, stable across runs.

## Documentation

- **[INSTALL.md](INSTALL.md)** — build the app, distribute it, install it.
- **[VERIFICATION.md](VERIFICATION.md)** — the human-gated proof checklist.
- **[SECURITY.md](SECURITY.md)** — data-handling guarantees and posture.
- **[CHANGELOG.md](CHANGELOG.md)** — what changed in each version.
- **[docs/grouping.md](docs/grouping.md)** — entity grouping + analysis-time
  clustering.
- **[docs/NER.md](docs/NER.md)** — the optional NER redaction layer.
- **[AGENTS.md](AGENTS.md)** — for coding agents: commands, invariants.

Per-module design and rationale live in the doc comment at the top of each file
in `src/`, next to the code they explain.

## Scope

The study's observation instrument, nothing more: for internal, consenting
participants only. Distributed privately to the team's own machines — not for
monitoring end-users, and not published.
