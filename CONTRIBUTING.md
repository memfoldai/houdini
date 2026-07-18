# Contributing

Houdini is a small, internal macOS tool. This is the developer's guide: build it,
run it, verify it, and the conventions to follow. What it is → [README.md](README.md);
how it works → [docs/architecture.md](docs/architecture.md); building the installer
→ [docs/install.md](docs/install.md).

## Setup

Requires a recent stable Rust toolchain and macOS 14+.

```bash
cargo test                      # portable core + integration; runs anywhere
cargo build --release
./target/release/houdini --diagnose   # one-shot: transcript counts, no content
./target/release/houdini              # run the menu-bar app
node --test extension/test/capture.test.mjs   # extension capture logic
```

To work on web capture, load the extension unpacked (`chrome://extensions` →
Developer mode → Load unpacked → `extension/`) with the app running.

## Conventions

Read [AGENTS.md](AGENTS.md) — it is the source of truth for build/run commands and
code style, and it is kept lean for coding agents. The load-bearing ones:

- **Code carries no comments.** The code is self-documenting; rationale lives in
  docs, the changelog, or commit messages.
- **Respect the architecture invariants** in
  [docs/architecture.md](docs/architecture.md#invariants) — single writer,
  redaction-before-store, additive-only migrations, local-only.
- American spelling; prefer real types over `any`-equivalents; small, focused
  files.

## Verifying a build (human-gated)

`cargo test` covers the portable core (ingestion, redaction, store, migrations,
export). This checklist is what a person runs once on a real Mac to confirm the
two live detectors work end-to-end and the data is safe to share. Do them in
order; each has an explicit pass condition. Inspect data with **Export my data…**,
which writes `data/interactions.jsonl`.

1. **CLI/agent ingest.** Run a prompt in Claude Code or Codex, launch the app,
   wait ~20 s, Export. *Pass:* the export has flat `"kind":"interaction"` rows for
   the exchange with correct `provider`/`tool`/`surface`/`model`, a `user` and an
   `assistant` row. The icon fills to a disc while recording.
2. **Web chat capture** (extension loaded). Send a message in ChatGPT, Claude, or
   Gemini on the web. *Pass:* the export gains a `chatgpt-web`/`claude-web`/
   `gemini-web` row pair under one `session_id`. A missing assistant row means that
   site's selector needs updating (`extension/capture.js`).
3. **False-positive gate.** With Slack, an editor, email, and unrelated tabs busy.
   *Pass:* no `interaction` row is written for any of them.
4. **Redaction audit** (safety gate, before sharing any data). Send a prompt with a
   **fake** AWS-shaped key and email, e.g.
   `note key AKIAIOSFODNN7EXAMPLE, mail jane@example.com`; ingest; Export.
   *Pass:* neither raw value appears — each is a `[REDACTED:…]` placeholder. If any
   raw value survives, **do not share the data**; file the gap first.
5. **Optional NER layer** (`--features ner`): a planted person name becomes
   `[REDACTED:NER:PERSON]`, and a missing model fails closed (logs, continues with
   deterministic redaction, never crashes). Setup: [docs/NER.md](docs/NER.md).

## Pull requests

Keep changes focused and the tests green (`cargo test` + the extension test).
Describe the behavior change and how you verified it. The extension and app share
a version — bump both when the native-messaging message shape changes.
