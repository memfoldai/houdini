# ai-usage-monitor

A minimal, menu-bar-only macOS app that detects when **any** AI model is being
used on screen and stores a redacted, reviewable record of the session — the
observation instrument for an internal AI-usage study.

It is deliberately small and standalone: its own repo, no cloud, no dashboards,
no coupling to any other product. It runs on the study team's own machines with
per-install consent and a visible capture indicator.

## The idea: detect AI by behavior, not by app

There is no allowlist of "AI apps" and no content classifier. The app watches
on-screen text and fires on the one signature every autoregressive model shares
and nothing else produces: **prose that grows a bit at a time, on its own, while
you are not typing** — token-by-token streaming. That is a physical property of
model inference, so it works for ChatGPT, Claude, a local model, a coding agent,
or a tool that doesn't exist yet, without knowing any of their names.

Other things that grow are excluded by **form, never by meaning**:

- **You typing** — the caret is in a text input, so growth is yours, not a model's.
- **Build logs / structured output** — brackets, paths, `key=value`, level tags.
  The detector scores prose-ness and requires it.

This is the whole point: `"best domain extensions in 2026"` is indistinguishable
from a Google search or a WhatsApp message by content, so content is never used.
The *act of a model streaming a reply* is what's detectable, and that's what's
detected.

It watches **every window concurrently** — all displays, all Spaces, background
and occluded windows. Each window is an independent surface with its own
detector, so two AI chats streaming at once become two separate sessions.

## Privacy model

- **Local-only.** No network egress anywhere in the code path. Nothing uploads.
- **Text only.** Images are used transiently for OCR and never stored.
- **Redaction is a hard gate**, applied offline before anything touches disk —
  secrets (provider API keys, private keys), emails, Luhn-checked cards, SSNs,
  phones. An optional [NER layer](docs/NER.md) catches free-form names at export.
- **App identity is salted-hashed** per install — extracts group "same app"
  without revealing which apps a person used.
- **Two-gate export:** automatic redaction, then the person reviews their own
  extract before sharing it.
- Quit anytime from the menu; the dot always shows the current state.

## Extract format

The local store is SQLite. The shareable extract is **JSON Lines** (one session
per line) — the standard interchange for pooled multi-device analytics.

Field names follow the [OpenTelemetry GenAI semantic conventions](https://github.com/open-telemetry/semantic-conventions-genai)
where a matching concept exists, so extracts speak the industry vocabulary
rather than an invented one:

| Field | Meaning |
|---|---|
| `service.instance.id` | Per-install UUID v4 — distinguishes machines in a pooled dataset (OTel resource semconv) |
| `gen_ai.conversation.id` | Session identifier |
| `gen_ai.input.messages` / `gen_ai.output.messages` | Content as `{role, parts:[{type:"text", content}]}`; *opt-in* attributes in the convention, matching this app's consent-gated design |
| `aum.schema`, `aum.app.hash`, `aum.capture.source`, `aum.session.*` | App-specific facts, namespaced per OTel custom-attribute guidance |

Two deliberate deviations: turns whose speaker cannot be attributed from
observation carry role `"unknown"`; and attributes the convention defines for
in-process instrumentation (model name, token counts) are **absent** — they are
not observable from a screen, and inventing them would be fabrication.

## Install

To install the finished app (or hand it to a teammate), build the signed
`.app`/`.dmg` and follow **[INSTALL.md](INSTALL.md)**:

```bash
packaging/bundle.sh        # → dist/AI Usage Monitor.app + a .dmg installer
```

Open the `.dmg`, drag to Applications, grant **Accessibility** + **Screen
Recording**, relaunch. A dot appears in the menu bar; click it for live status.
INSTALL.md covers the one-time signing certificate and the notarization path for
zero-friction install on other machines.

## Menu bar & status

The icon is a monochrome template glyph whose **shape** shows state (macOS tints
it for you): a hollow ring when idle, a ring-with-dot while watching, a solid
disc while recording an AI chat, two bars when paused. A small **"Recording"**
label appears next to it only while a chat is actively being captured.

Click it for a plain-language readout ("Watching for AI use", "Recording an AI
chat", how many were captured recently, when the last one was) — that is how you
confirm it is working. The menu also has:

- **Pause watching** — for 15 minutes, 1 hour, or until you resume. While paused
  nothing is captured (handy before typing something sensitive). Global by
  design: it protects whatever you're doing, in any window.
- **Export for review…** — writes the redacted extract and reveals it in Finder.
- **Open activity log** — a metadata-only diagnostics log (no captured text).
- **Quit**.

## Develop

Requires a recent stable Rust toolchain and macOS 14+.

```bash
cargo test                 # portable core (runs anywhere)
cargo build --release
scripts/sign.sh            # sign the bare binary with a stable identity
./target/release/ai-usage-monitor
```

**Signing is not optional**, even in dev: macOS keys the Accessibility and
Screen Recording grants to the code identity, so an unsigned rebuild silently
loses both and captures nothing. `scripts/sign.sh` (bare binary) and
`packaging/bundle.sh` (app) both handle it; see INSTALL.md for the certificate.

Before trusting or sharing any data, run **[VERIFICATION.md](VERIFICATION.md)** —
the checklist that proves capture, concurrent/background windows, the redaction
audit, and the false-positive gate.

## Configuration

`~/Library/Application Support/ai.memfold.ai-usage-monitor/config.json` is
created on first run with a random salt and install id. Operator knobs:

| Key | Default | Purpose |
|---|---|---|
| `sample_interval_ms` | 350 | Frontmost-app sampling cadence |
| `full_sweep_every_ticks` | 6 | Every Nth tick sweeps *all* windows (≈2.1 s) |
| `min_surface_area` | 40000 | Skip windows below this pt² (too small to hold a chat) |
| `max_ocr_per_sweep` | 6 | OCR budget per sweep; excess is logged and retried |
| `ocr_min_interval_ms` | 800 | Min time between OCR captures of the same window (CPU throttle) |
| `session_idle_gap_ms` | 4000 | No-growth gap that ends a session |
| `detector` | — | Streaming thresholds; VERIFICATION.md step 4 is the tuning loop |
| `ner_model_dir` | unset | Enables the [NER export sweep](docs/NER.md) (`--features ner` build) |

`salt` and `install_id` are generated per install. The salt never leaves the
machine.

## Documentation

- **[INSTALL.md](INSTALL.md)** — build the app, distribute it, install it.
- **[VERIFICATION.md](VERIFICATION.md)** — the human-gated proof checklist.
- **[SECURITY.md](SECURITY.md)** — data-handling guarantees, dependency audit,
  memory/CPU posture.
- **[CHANGELOG.md](CHANGELOG.md)** — what changed in each version.
- **[docs/grouping.md](docs/grouping.md)** — how sessions are grouped by
  provider/surface at analysis time (no hardcoding, no LLM in the daemon).
- **[docs/NER.md](docs/NER.md)** — the optional NER redaction layer.
- **[AGENTS.md](AGENTS.md)** — for coding agents: commands, non-negotiables,
  architecture invariants.

If it seems like it isn't capturing, **Open activity log** from the menu (or
`tail -f "~/Library/Application Support/ai.memfold.ai-usage-monitor/ai-usage-monitor.log"`).
It shows, without any captured text, how many windows each sweep saw, the text
lengths it read, and when sessions start and end — enough to tell whether the
issue is permissions, capture, or detection tuning.

Per-module design and rationale live in the doc comment at the top of each file
in `src/`, next to the code they explain.

## Scope

The study's observation instrument, nothing more: for internal, consenting
participants only. Distributed privately to the team's own machines — not for
monitoring end-users, and not published.
