# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/). While pre-1.0, minor versions may
include behavior changes.

## [0.2.2] — 2026-07-16

### Fixed
Nothing was ever captured from browser/Electron AI apps (ChatGPT web, ChatGPT
and Claude desktop). Four stacked bugs in the OCR path, each found by
instrumenting the real capture stack against a live screen (`--diagnose` and the
debug log), not by guessing:

- **OCR never ran at all.** The per-window OCR throttle used an `i64::MIN` "never
  captured" sentinel; `now - i64::MIN` overflowed and, in release builds, wrapped
  negative, so every window read as "throttled" forever. Replaced with an
  explicit, tested `ocr_due` (regression test included).
- **Empty screenshots.** `SCStreamConfiguration` defaults to 0×0 output; the
  capture is now sized to the window (2× for OCR-legible text).
- **SCStreamError -3811 ("audio/video capture failure").** `SCScreenshotManager`
  starts an internal stream that tried the audio path; `setCapturesAudio(false)`
  (and `setShowsCursor(false)`) fixes it.
- **The AI window was starved of the OCR budget.** Windows were OCR'd in
  arbitrary enumeration order, so a background editor's empty windows used up the
  budget first. Candidates are now prioritized by visible-on-screen and largest
  area, so the window you're actually looking at is read first.

### Added
- `ai-usage-monitor --diagnose`: a one-shot probe (permissions, window
  enumeration, per-app AX text) that prints why capture is or isn't working.
- Debug-level capture tracing (per-app AX/OCR outcomes, char counts) behind
  `RUST_LOG=ai_usage_monitor=debug`.

## [0.2.1] — 2026-07-16

### Fixed
- **App would not launch after upgrading.** 0.2.0 added a config field without a
  default, so an existing `config.json` (written by an earlier version) failed to
  parse and the app aborted on startup. Every config field now has a default, the
  file is rewritten on load so it gains new fields, and a genuinely corrupt config
  is backed up and reset instead of crashing. Added an upgrade regression test.
  0.2.1 supersedes 0.2.0, which could not start.

## [0.2.0] — 2026-07-16

### Added
- **Pause / resume.** Global pause with timed options (15 minutes, 1 hour, until
  you resume) so you can stop capture while typing something sensitive without
  quitting. While paused, no capture runs at all.
- **Plain-language menu.** The dropdown now reads in human terms ("Watching for
  AI use", "Recording an AI chat", "Paused") with a captured-recently summary,
  plus **Export for review…**, **Open activity log**, and **Quit**.
- **Transient "Recording" label** next to the menu-bar icon only while a chat is
  actively being captured — clear feedback, no persistent clutter.
- **Paused icon** (two bars) alongside the existing idle/watching/recording
  glyphs.
- **Diagnostics log** (metadata only — never captured text) under the data dir,
  capped and rotated, openable from the menu, so "is it working?" is answerable.
- `.github/workflows/ci.yml`: build, test, and `cargo audit` on every push.
- `SECURITY.md` documenting the data-handling guarantees and audit posture.

### Fixed
- **Detection no longer misses real sessions.** Growth detection tolerated only
  a byte-perfect prefix between frames, which never holds for OCR (it re-reads
  the whole window with jitter). It now uses a longest-common-prefix ratio, so a
  streaming answer read via OCR registers as streaming.
- **Signing.** `scripts/sign.sh` matched signing identities with
  `find-identity -v`, which hides untrusted self-signed certs; it now matches all
  code-signing identities (a self-signed cert signs fine despite showing as
  untrusted).

### Changed
- **Lower CPU.** OCR is throttled per window (decoupled from the sample rate)
  and fast ticks skip window enumeration when the frontmost app is AX-readable.
- Detector per-step growth ceiling widened to suit the throttled OCR cadence.

## [0.1.0] — 2026-07-16

### Added
- Initial release: streaming-signature AI-usage detection (no app allowlists),
  offline redaction, local SQLite store, OpenTelemetry-GenAI-aligned JSONL
  export, concurrent multi-window/Space/background capture, optional GLiNER-PII
  layer, and a signed `.app` + `.dmg` build (`packaging/bundle.sh`).

[0.2.2]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.2.2
[0.2.1]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.2.1
[0.2.0]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.2.0
[0.1.0]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.1.0
