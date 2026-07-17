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
  phones.
- **App identity is salted-hashed** per install — data groups "same app"
  without revealing which apps a person used.
- **Only the exchange is kept** — the prompt and the reply, not the whole
  conversation history or the surrounding UI.
- Quit anytime from the menu; the icon always shows the current state.

## Data format

The local store is SQLite (the source of truth). Finished sessions are written
automatically — no manual export — to **day files** `data/YYYY-MM-DD.jsonl`, one
JSON object per line. Day partitioning is the standard shape for analytics at
scale: files from any number of machines merge trivially (each line carries the
device id and date), and a day/week rollup is just concatenating files.

Each record is deliberately lean, with the prompt and reply as separate fields:

| Field | Meaning |
|---|---|
| `schema` | Record schema tag (`aum/1`) |
| `device` | Per-install UUID — keeps machines distinguishable in a pooled dataset |
| `day` | `YYYY-MM-DD`, matching the file |
| `app` | Salted app hash (never the app name) |
| `surface` | Coarse class: `web` (read via OCR) or `app` (read via Accessibility) |
| `started_ms` / `ended_ms` | Session bounds (unix ms) |
| `prompt` | The user's message, if captured |
| `reply` | The model's reply (redacted; just the reply, not the history) |

Provider grouping (ChatGPT app + web + CLI → one entity) happens at analysis
time over these files — see [docs/grouping.md](docs/grouping.md).

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
disc while catching an AI chat, two bars when paused. It's icon-only — no text
label to get stuck.

Click it for a friendly readout ("Keeping an eye out 👀", "Catching an AI chat
✨", how many chats were caught today) and:

- **Take a break** — for 15 minutes, an hour, or until you're back. While paused
  nothing is captured (handy before typing something sensitive). Global by
  design: it protects whatever you're doing, in any window.
- **Show my data** — reveals the day-partitioned data folder in Finder.
- **Peek under the hood** — the metadata-only activity log (no captured text).
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
