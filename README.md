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

## Quick start

Requires a recent stable Rust toolchain and macOS 14+.

```bash
cargo test                 # portable core (runs anywhere)
cargo build --release
scripts/sign.sh            # stable self-signed identity — REQUIRED, see below
./target/release/ai-usage-monitor
```

A dot appears in the menu bar: **gray** = idle, **blue** = watching, **green** =
capturing an AI session right now. Its menu has two items — *Export extract for
review…* and *Quit*.

**Signing is not optional for real use.** macOS keys the Accessibility and
Screen Recording grants to the binary's code identity, so an unsigned rebuild
silently loses both and captures nothing. `scripts/sign.sh` gives it a stable
identity; see the script header for the one-time certificate setup.

Then run **[VERIFICATION.md](VERIFICATION.md)** — the checklist that proves the
app actually works on your machine (grants, real capture, concurrent/background
windows, the redaction audit, and the false-positive gate). Do not trust or
share data before it passes.

## Configuration

`~/Library/Application Support/ai.memfold.ai-usage-monitor/config.json` is
created on first run with a random salt and install id. Operator knobs:

| Key | Default | Purpose |
|---|---|---|
| `sample_interval_ms` | 350 | Frontmost-app sampling cadence |
| `full_sweep_every_ticks` | 6 | Every Nth tick sweeps *all* windows (≈2.1 s) |
| `min_surface_area` | 40000 | Skip windows below this pt² (too small to hold a chat) |
| `max_ocr_per_sweep` | 6 | OCR budget per sweep; excess is logged and retried |
| `session_idle_gap_ms` | 4000 | No-growth gap that ends a session |
| `detector` | — | Streaming thresholds; VERIFICATION.md step 4 is the tuning loop |
| `ner_model_dir` | unset | Enables the [NER export sweep](docs/NER.md) (`--features ner` build) |

`salt` and `install_id` are generated per install. The salt never leaves the
machine.

## Documentation

- **[AGENTS.md](AGENTS.md)** — for coding agents: commands, non-negotiables,
  architecture invariants.
- **[VERIFICATION.md](VERIFICATION.md)** — the human-gated proof checklist.
- **[docs/NER.md](docs/NER.md)** — the optional NER redaction layer.

Per-module design and rationale live in the doc comment at the top of each file
in `src/`, next to the code they explain.

## Scope

The study's observation instrument, nothing more: internal, consenting
participants only. Not for monitoring end-users, not for distribution, not
notarized.
