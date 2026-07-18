# AGENTS.md

macOS menu-bar daemon that records what people use AI for, for an internal usage
study. It reads the actual prompts/replies from AI tools' **own local
transcripts** and from **web chats via a browser extension** (native messaging).
No screen capture, no network monitoring. Rust, no framework. Read this before
changing code.

## Commands

```bash
cargo test                       # portable core + integration; runs on any platform
cargo build && cargo build --release
cargo check --features ner       # optional NER layer (needs onnxruntime to link)
cargo audit                      # RustSec check (CI runs this; must stay clean)
./target/release/houdini --diagnose   # transcript counts
```

Debugging ("is it working?"): the app writes a metadata-only log to
`<data-dir>/houdini.log`. `--diagnose` prints per-tool transcript counts. Never log ingested text — it
is pre-redaction (see `logging` module docs).

Keep builds at **zero warnings**.

## Non-negotiables

1. **Documentation-first. Never assume an API.** Before using any framework,
   crate, or system API, read its real docs/source (Apple SDK headers, the
   vendored crate source under `~/.cargo/registry/src/*/<crate>/`, docs.rs). Cite
   what you read. The transcript formats and the native-messaging framing were
   verified against real data/specs before coding — keep that bar.
2. **Attribute by source metadata, never by content meaning.** Provider comes
   from the tool the transcript/site names, or its model prefix
   (`src/attribution.rs`) — never from classifying what the text *says*. No
   keyword/embedding classification in the daemon. A per-tool adapter is metadata
   (which tool belongs to which vendor); guessing intent from content is not.
3. **Redaction is a hard gate, not a feature.** Raw transcript text must never
   reach disk. `redact_deterministic` runs before every turn is stored, offline.
   No redactor may make a network call.
4. **Local-only, no egress.** The daemon makes zero network calls. It reads files
   the user owns; the extension reads the page in the user's browser. Any semantic/LLM labeling
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

Portable core (lib `houdini`, fully tested, no macOS deps) + macOS
native shell (the binary, `cfg(target_os = "macos")`). This split is why
`cargo test` runs everywhere; keep native code out of the lib.

Two content sources, one store:

- **Transcripts — `ingest/`** (portable). One `Adapter` per tool discovers and
  parses its transcripts into a canonical `IngestedSession` (provider, tool,
  surface, model, turns). `Ingestor` polls adapters, skips unchanged files by
  mtime+size and files older than launch, and upserts by `(tool, external_id)` so
  a growing transcript appends only new turns. Redaction runs in `persist`.
- **Web chats — `nativehost.rs` + `browserhost.rs` + `extension/`** (macOS +
  Chromium). The extension reads each exchange in the page (prompt from the site's
  own API request, reply from the rendered DOM — NOT the undocumented SSE) and
  relays it over native messaging to `--native-host`, which validates the tool,
  redacts, and appends a `web` session. `--install-browser-host` writes the host
  manifest into every Chromium browser. Framing is 32-bit native-endian length +
  JSON (Chrome spec). Per-site extraction is reverse-engineered — keep it
  defensive (fail to nothing, never store garbage) and validate live.
- **`store.rs`** — `sessions` + `turns`, SQLite, source of truth, with an
  `exported_seq` per-session high-water mark. **`export.rs`** — one flat row per
  turn to `data/interactions/YYYY-MM-DD.jsonl`, schema `aum/3`. No content-free
  "presence" signal is collected (it had no research value).
- **Updates — `updater.rs`** (macOS). Checks GitHub Releases via `gh` (team auth, no embedded token), compares to `CARGO_PKG_VERSION`, and on the menu action downloads the release `.dmg`, verifies signing, atomically swaps the `/Applications` bundle, and relaunches. Gated to an installed `.app`.
- **`main.rs` → `app.rs`** (NSApplication Accessory, tray, timer). The tray is
  created in `applicationDidFinishLaunching:` — tray-icon requires a *running*
  run loop. The menu shows the app version (`CARGO_PKG_VERSION`).

Two clocks: **monotonic** drives cadence, **wall-clock** is stored.

## Conventions

- **Code carries no comments.** The code is self-documenting (clear names, small
  functions, obvious control flow); the "why" lives in these docs (README,
  AGENTS, CHANGELOG). Do not add inline or doc comments — if a piece of code needs
  explaining, restructure or rename it, or record the rationale in the docs.
- Prefer typed shapes over positional tuples crossing a boundary.
- Registry-driven, source-agnostic: a new transcript tool is one `Adapter` (Claude Code, Codex, OpenClaw); a new
  web site is one `SITES` entry + one `resolve_tool` arm. No content classification
  or hardcoded per-app behavior — provider comes from tool/model metadata only.
- Config over constants for anything an operator may tune; see `config.rs`.
- Bound every loop that touches the system; log what you skipped.

## Tests

Behavioral only. Each test asserts a real guarantee (adapters parse the real
transcript shapes; ingest appends only new turns; the native host redacts a
seeded secret before storing; the interceptor extracts prompt+reply). Validate parsers against **real** files
(`~/.claude`, `~/.codex`), not only fixtures.

## What you cannot verify here

The tray and the run loop need a real Mac, and the browser extension's per-site
extraction needs a live logged-in page to confirm. But transcript parsing is
verifiable against real files (and `--diagnose` shows the counts), the native
host is proven with framed bytes, and the interceptor parsers run against sample
inputs in `extension/test/`. `cargo test` proves the portable
core + the ingest integration. Do not claim the tray path works from a green
build alone.

## Further reading

- [CONTRIBUTING.md](CONTRIBUTING.md) — build, run, and the human-gated verification checklist.
- [docs/architecture.md](docs/architecture.md) — how it works: codemap, data flow, invariants.
- [docs/privacy.md](docs/privacy.md) — the data/consent model.
- [docs/install.md](docs/install.md) — build/sign/notarize the .app, distribute, install.
- [docs/grouping.md](docs/grouping.md) — entity grouping + analysis-time clustering.
- [docs/NER.md](docs/NER.md) — optional GLiNER-PII layer.
- [README.md](README.md) — what this is and why, for humans.
