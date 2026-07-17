# ai-usage-monitor

A minimal, menu-bar-only macOS app that records **what people use AI for** — the
observation instrument for an internal AI-usage study. It reads AI tools' own
structured logs and observes AI network activity; it does **not** capture the
screen.

It is deliberately small and standalone: its own repo, no cloud, no dashboards,
no coupling to any other product. It runs on the study team's own machines with
per-install consent and a visible menu-bar indicator.

## How it detects AI use

Two independent, reliable signals — no screenshots, no OCR, no guessing from
pixels:

**Layer A — transcript ingestion (the rich signal).** Coding agents already
persist every interaction to a structured local transcript. The monitor reads
those directly, so it gets the exact prompt and reply, the real timestamps, the
model, and a session id — with zero false positives and full coverage across
desktops and Spaces. Adapters ship for:

| Tool | Reads |
|---|---|
| Claude Code | `~/.claude/projects/*/*.jsonl` |
| Codex | `~/.codex/**/rollout-*.jsonl` |

Adding a tool is adding one small adapter; the rest of the pipeline is shared.

**Layer B — network presence (the coverage signal).** For AI used where no local
transcript exists (web chats, native apps), the monitor observes which process
connects to which AI endpoint, read from the process table with `libproc` — the
same information `lsof` shows, **no root and no entitlement**. A known AI tool or
app is attributed by its process identity (so the ChatGPT app, the Codex CLI, and
Claude Code all register even though OpenAI's traffic rides Cloudflare); a browser
is attributed only when its destination is a provider-owned IP range. This is a
content-free "an AI tool was active" signal — who, when, and how often, never
what was said.

Everything that is not identifiably AI resolves to nothing, which is why Slack, an
editor, or a browser to an unrelated site never register.

**Layer C — browser web chats (optional extension).** Web ChatGPT/Claude leave no
local transcript and ride shared CDNs, so neither layer above reads their content.
An optional Chromium extension ([extension/](extension/README.md)) captures them
by intercepting the site's **own API calls** (the reliable technique, and it works
in background tabs), delivering each exchange to the app over local native
messaging — never over the network. Covers ChatGPT and Claude on the web today.

### Honest limits

- **The browser extension is installed per browser**, and its per-site parsers
  track reverse-engineered endpoint shapes, so a site redesign can need a small
  parser fix. Without it, web chats are uncaptured (their native apps/CLIs are
  still caught).
- **Gemini on the web** isn't parsed yet (obfuscated batch transport).
- Network presence means "an AI tool was connected/active," coarser than a
  discrete message. The transcript and extension layers supply exact interactions.

## Privacy model

- **Local-only.** No network egress anywhere in the code path. Nothing uploads.
- **No screen capture, no TCC permission.** The app reads files the user already
  owns and observes its own user's sockets — it never asks for Screen Recording
  or Accessibility.
- **Content is redacted** — offline, before anything touches disk — for secrets
  (provider API keys, private keys), emails, Luhn-checked cards, SSNs, phones.
- **Identity is kept in the clear** on purpose: for a consenting internal study
  the provider/tool (`anthropic`, `claude-code`) *is* the research signal. Only
  the message content is redacted.
- Pause anytime from the menu; while paused nothing new is recorded.

## Data format

The local store is SQLite (the source of truth). New and changed records are
written automatically — no manual export — to **day files** `data/YYYY-MM-DD.jsonl`,
one JSON object per line. Day partitioning is the standard analytics-at-scale
shape: files from any number of machines merge trivially (each line carries the
device id), and a day/week rollup is just concatenating files.

Two record kinds share the day file, told apart by `kind`:

**`interaction`** — a real session from a transcript:

```json
{"schema":"aum/2","kind":"interaction","device":"…","day":"2026-07-16",
 "provider":"anthropic","tool":"claude-code","surface":"cli",
 "model":"claude-sonnet-5","session":"…","started_ms":…,"ended_ms":…,
 "message_count":2,"turns":[{"role":"user","text":"…","ts_ms":…},…]}
```

**`presence`** — a content-free network interval:

```json
{"schema":"aum/2","kind":"presence","device":"…","day":"2026-07-16",
 "provider":"openai","process":"ChatGPT","surface":"app",
 "started_ms":…,"ended_ms":…,"observations":12}
```

Provider grouping (Claude app + CLI + web → one entity) is deterministic at
ingest; higher-level semantic clustering (research vs build, topic) is an
analysis-time job over these files — see [docs/grouping.md](docs/grouping.md).

## Menu bar & status

The icon is a monochrome template glyph whose **shape** shows state (macOS tints
it): a hollow ring when idle, a ring-with-dot when an AI is in use nearby, a
solid disc the moment a new interaction is recorded, two bars when paused. Click
it for a friendly readout and:

- **Take a break** — for 15 minutes, an hour, or until you're back. While paused
  nothing is recorded.
- **Show my data** — reveals the day-partitioned data folder in Finder.
- **Quit**.

## Develop

Requires a recent stable Rust toolchain and macOS 14+.

```bash
cargo test                       # portable core + integration (runs anywhere)
cargo build --release
./target/release/ai-usage-monitor --diagnose   # one-shot: what each layer sees now
./target/release/ai-usage-monitor              # run the menu-bar app
```

`--diagnose` is the "is it working?" answer: it prints how many interactions each
transcript adapter can read and every AI network connection live on the machine
right now — no content, just counts and endpoints.

Signing is still recommended for a stable install identity, but the app no longer
depends on any TCC grant, so a rebuild never silently loses capture.

## Configuration

`~/Library/Application Support/ai.memfold.ai-usage-monitor/config.json` is created
on first run. Operator knobs:

| Key | Default | Purpose |
|---|---|---|
| `transcript_poll_ms` | 5000 | How often to scan transcripts for new interactions |
| `network_poll_ms` | 5000 | How often to poll the process table for AI connections |
| `presence_gap_ms` | 60000 | A provider unseen this long closes its presence interval |
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
