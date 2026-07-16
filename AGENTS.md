# AGENTS.md

macOS menu-bar daemon that detects AI usage on screen by its **streaming
signature** and stores redacted session text for an internal usage study.
Rust, no framework. Read this before changing code.

## Commands

```bash
cargo test                       # portable core; runs on any platform
cargo build && cargo build --release
cargo check --features ner       # optional NER layer (needs onnxruntime to link)
cargo audit                      # RustSec check (CI runs this; must stay clean)
scripts/sign.sh                  # sign the bare dev binary (REQUIRED, see Signing)
packaging/bundle.sh              # build the signed .app + .dmg (see INSTALL.md)
```

Debugging capture ("is it working?"): the app writes a metadata-only log to
`<data-dir>/ai-usage-monitor.log` (menu → Open activity log). Run with
`RUST_LOG=ai_usage_monitor=debug` for per-sweep window counts and text lengths.
Never log captured text — it is pre-redaction (see `logging` module docs).

There is no lint/format config beyond rustfmt defaults. Keep builds at
**zero warnings** — they are currently zero, do not add any.

## Non-negotiables

These are the rules that make this project what it is. Violating one is a bug
even if it compiles.

1. **Documentation-first. Never assume an API.** Before using any framework,
   crate, or system API, read its real docs/source: Apple SDK headers under
   `xcrun --show-sdk-path`, the vendored crate source under
   `~/.cargo/registry/src/*/<crate>/`, or official docs. Cite what you read in
   the code comment. Do not guess signatures, defaults, or behavior.
2. **Detect AI by behavior, never by content or app name.** No allowlists of AI
   apps, no domain lists, no keyword/embedding classification of what text
   means. The only signal is: prose grows incrementally, on its own, while the
   user is not typing. Disambiguate from logs by *form* (prose vs structured),
   never by intent. Adding `if app == "ChatGPT"` defeats the entire premise.
3. **Redaction is a hard gate, not a feature.** Raw text must never reach disk.
   `redact_deterministic` runs before every store write, offline. No redactor
   may make a network call.
4. **No cheap fallbacks.** No hardcoded names/ids/log strings in prod code to
   fix an observed case. Fix it with a general rule or say it is unfixed.
4b. **Config fields are upgrade-sensitive.** Every field in `AppConfig` MUST
   carry a `serde(default)` — a new required field crashes every existing
   install on load (this shipped once as the 0.2.0 launch crash). `load_or_init`
   rewrites the file on load to add new fields; keep it that way.
5. **Fail closed.** If a layer cannot do its job (e.g. the NER model fails its
   self-test), disable it loudly — never silently claim coverage it lacks.

## Architecture

Portable core (lib `ai_usage_monitor`, fully tested, no macOS deps) + macOS
native shell (the binary, `cfg(target_os = "macos")`). This split is why
`cargo test` runs everywhere; keep native code out of the lib.

Pipeline: `capture → detector → monitor → session → redact → store → export`.

- Every **window is an independent "surface"** with its own detector and
  session. Concurrent AI chats across displays/Spaces/background are separate
  sessions. Do not reintroduce frontmost-only assumptions.
- Two clocks: **monotonic** drives detection timing, **wall-clock** is stored.
  See `monitor.rs` `TickClock`. Mixing them is a bug we already fixed once.
- `main.rs` → `app.rs` (NSApplication Accessory, tray, timer). The tray must be
  created in `applicationDidFinishLaunching:` — tray-icon requires a *running*
  run loop.

Read the module doc comment at the top of each file in `src/` — that is where
per-module design and its rationale live. Do not duplicate it elsewhere.

## Conventions

- Comments state **why** a thing exists / what breaks without it. Never narrate
  syntax. Cite the header/doc that justifies a non-obvious API call.
- Prefer typed shapes over positional tuples crossing a boundary
  (`store::SessionRow`, not `(i64, String, ...)`).
- Config over constants for anything an operator may tune; see `config.rs`.
- Bound every sweep/loop that touches the system, and **log what you skipped** —
  no silent truncation.

## Tests

Behavioral only. Each test must assert a real guarantee (concurrent surfaces
become separate sessions; typing never captures; redaction catches a seeded
secret). Do **not** add tests for incidental branches or to raise a count.
Tests needing a real model/machine are `#[ignore]`d with the command in a
comment.

## Signing

macOS TCC keys the Accessibility + Screen Recording grants to the code identity.
An unsigned rebuild silently loses both and captures nothing. Always sign after
building (`scripts/sign.sh` for the bare binary, `packaging/bundle.sh` for the
app), and never suggest `codesign -s -` (ad-hoc changes identity every build).
A self-signed cert shows as untrusted but signs fine — trust only matters for
other machines verifying it. The bundled app's Info.plist MUST keep
`NSScreenCaptureUsageDescription` (ScreenCaptureKit terminates a bundled app
that lacks it) and `LSUIElement` (menu-bar-only).

## What you cannot verify here

Capture, permissions, and the tray need a real signed Mac with grants and real
windows. `cargo test` proves the core only. Do not claim the native path works
from a green build — route it to VERIFICATION.md and say what is unproven.

## Further reading

- [VERIFICATION.md](VERIFICATION.md) — the human-gated checklist (permissions,
  real capture, redaction audit, false-positive gate). Run before trusting data.
- [INSTALL.md](INSTALL.md) — build/sign/notarize the .app, distribute, install.
- [docs/NER.md](docs/NER.md) — optional GLiNER-PII layer: provisioning, labels,
  self-test.
- [README.md](README.md) — what this is and why, for humans.
