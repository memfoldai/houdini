# AGENTS.md

macOS menu-bar daemon that records what people use AI for, for an internal usage
study. It reads AI tools' **own local transcripts** (Layer A) and observes
**process → AI-endpoint network connections** (Layer B). No screen capture. Rust,
no framework. Read this before changing code.

## Commands

```bash
cargo test                       # portable core + integration; runs on any platform
cargo build && cargo build --release
cargo check --features ner       # optional NER layer (needs onnxruntime to link)
cargo audit                      # RustSec check (CI runs this; must stay clean)
./target/release/ai-usage-monitor --diagnose   # what each layer sees live
```

Debugging ("is it working?"): the app writes a metadata-only log to
`<data-dir>/ai-usage-monitor.log`. `--diagnose` prints per-tool transcript counts
and live AI network connections (no content). Never log ingested text — it is
pre-redaction (see `logging` module docs).

Keep builds at **zero warnings**.

## Non-negotiables

1. **Documentation-first. Never assume an API.** Before using any framework,
   crate, or system API, read its real docs/source (Apple SDK headers, the
   vendored crate source under `~/.cargo/registry/src/*/<crate>/`, docs.rs). Cite
   what you read. The `libproc` socket-union access and the transcript formats
   were both verified against real data before coding — keep that bar.
2. **Attribute by source metadata, never by content meaning.** Provider comes
   from the tool/model the transcript names, the process identity, or a
   provider-owned IP range (`src/attribution.rs`) — never from classifying what
   the text *says*. No keyword/embedding classification in the daemon. A per-tool
   adapter or a provider range is metadata (which binary belongs to which
   vendor), which is allowed; guessing intent from message content is not.
3. **Redaction is a hard gate, not a feature.** Raw transcript text must never
   reach disk. `redact_deterministic` runs before every turn is stored, offline.
   No redactor may make a network call. Presence records carry no content at all.
4. **Local-only, no egress.** The daemon makes zero network calls. It reads files
   the user owns and observes its own user's sockets. Any semantic/LLM labeling
   is an analysis-time job over the exported files (see docs/grouping.md), never
   in the app.
5. **No cheap fallbacks.** No hardcoded one-off fixes in prod code. Fix with a
   general rule or say it is unfixed. Adapters/attribution rules are a maintained
   registry, not per-case patches.
6. **Config fields are upgrade-sensitive.** Every `AppConfig` field carries a
   `serde(default)`; `load_or_init` rewrites the file to add new fields and backs
   up an unparseable one rather than crashing.
7. **Fail closed.** If a layer cannot do its job (e.g. the NER model fails its
   self-test), disable it loudly — never silently claim coverage it lacks.

## Architecture

Portable core (lib `ai_usage_monitor`, fully tested, no macOS deps) + macOS
native shell (the binary, `cfg(target_os = "macos")`). This split is why
`cargo test` runs everywhere; keep native code out of the lib.

Two detectors, one store:

- **Layer A — `ingest/`** (portable). One `Adapter` per tool discovers and parses
  its transcripts into a canonical `IngestedSession` (provider, tool, surface,
  model, turns). `Ingestor` polls adapters, skips unchanged files by mtime+size
  and files older than launch, and upserts by `(tool, external_id)` so a growing
  transcript appends only new turns. Redaction runs in `persist`.
- **Layer B — `netpresence.rs`** (macOS). Enumerates own-user processes' TCP
  connections via `libproc` (`proc_pidfdinfo`), attributes each with
  `attribution.rs`, and coalesces observations into presence intervals. No root,
  no entitlement. The pure coalescing step (`apply`) is split from the syscall
  path (`observe_connections`) so it is testable.
- **Layer C — `nativehost.rs` + `browserhost.rs` + `extension/`** (macOS +
  Chromium). The extension intercepts web AI sites' own fetch/SSE (MAIN-world
  content script, works in background tabs) and relays each exchange over native
  messaging to `--native-host`, which validates the tool, redacts, and appends a
  `web` session. `--install-browser-host` writes the host manifest into every
  Chromium browser. Framing is 32-bit native-endian length + JSON (Chrome spec).
  Per-site parsers are reverse-engineered — keep them defensive (fail to nothing,
  never store garbage) and validate live.
- **`store.rs`** — `sessions` + `turns` (interactions) and `presence` (network),
  SQLite, source of truth. **`export.rs`** — day files `data/YYYY-MM-DD.jsonl`,
  schema `aum/2`, records tagged `kind: interaction | presence`.
- **`main.rs` → `app.rs`** (NSApplication Accessory, tray, timer). The tray is
  created in `applicationDidFinishLaunching:` — tray-icon requires a *running*
  run loop. One base timer gates each detector on its own cadence.

Two clocks: **monotonic** drives cadence, **wall-clock** is stored. Read the
module doc comment atop each file — that is where per-module rationale lives.

## Conventions

- Comments state **why** a thing exists / what breaks without it. Never narrate
  syntax. Cite the header/doc that justifies a non-obvious API call (e.g. the
  `SAFETY` note on the libproc socket union).
- Prefer typed shapes over positional tuples crossing a boundary.
- Config over constants for anything an operator may tune; see `config.rs`.
- Bound every loop that touches the system; log what you skipped.

## Tests

Behavioral only. Each test asserts a real guarantee (adapters parse the real
transcript shapes; ingest appends only new turns; a known CLI process attributes
regardless of a CDN endpoint; a non-AI process never attributes; redaction
catches a seeded secret before storage). Validate parsers against **real** files
(`~/.claude`, `~/.codex`), not only fixtures.

## What you cannot verify here

The tray, the run loop, and the live `libproc` snapshot need a real Mac. But
Layer A parsing is verifiable against real transcript files, and `--diagnose`
proves Layer B on a real machine — use both. `cargo test` proves the portable
core + the ingest integration. Do not claim the tray path works from a green
build alone.

## Further reading

- [VERIFICATION.md](VERIFICATION.md) — the human-gated checklist.
- [INSTALL.md](INSTALL.md) — build/sign/notarize the .app, distribute, install.
- [docs/grouping.md](docs/grouping.md) — entity grouping + analysis-time clustering.
- [docs/NER.md](docs/NER.md) — optional GLiNER-PII layer.
- [README.md](README.md) — what this is and why, for humans.
