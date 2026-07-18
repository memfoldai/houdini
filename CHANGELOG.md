# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/). While pre-1.0, minor versions may
include behavior changes.

## [0.4.9] — 2026-07-18

### Changed
- **Over-the-air updates now work on machines without `gh`.** The updater no
  longer shells out to the GitHub CLI (which needs an install, auth, and repo
  access none of a non-developer machine has). It calls the GitHub REST API
  directly with a fine-grained, **read-only** token baked in at build time
  (`AUM_UPDATE_TOKEN`, supplied via a gitignored `packaging/.update-token`). The
  token is fed to curl on stdin, never in argv, so it never appears in `ps`.
  Verified live against the private repo's release API.
- **Updates install silently to stay current.** The periodic check (on launch +
  every 6h) now downloads, verifies the signature, swaps the `/Applications`
  bundle, and relaunches automatically — no click. The menu's *Check for
  updates…* still gives a manual, visible check. Industry-standard model
  (mirrors Sparkle's automatic-install behavior).

### Added
- **The browser bridge sets itself up.** The app registers its native-messaging
  host for every installed Chromium browser on launch, idempotently. A
  non-developer now only has to install the DMG and load the extension — no
  terminal command. (Re-registering each launch also keeps the path correct
  across self-updates.)

### Fixed
- Removed the two-step "Install update" menu state and its `available_update`
  cell — one code path, less state to keep in sync.

## [0.4.8] — 2026-07-18

### Fixed
- **The app crashed on launch** (`RefCell already borrowed`). The update-check
  poller held a `borrow()` of the result channel across a `borrow_mut()` of the
  same cell, which panics the instant a check result arrives — so the menu-bar app
  aborted shortly after starting. Fixed by scoping the read borrow before the
  write. (This is a runtime-only failure a unit test can't see; the fix was
  verified by running the signed app.)

## [0.4.7] — 2026-07-18

### Changed
- **Data is now encrypted at rest.** The store is an encrypted SQLite database
  (SQLCipher / AES-256, `bundled-sqlcipher`), with the key generated once and kept
  in the **macOS Keychain** (`src/keychain.rs`) — the production-standard way to
  hold sensitive local data. Nothing readable is written to a folder anymore. On
  first launch an existing plaintext DB is moved aside (`*.pre-encryption.bak`) and
  rebuilt encrypted; validated end-to-end on a real DB. DB schema is v5.
- **Export is now on demand.** The automatic plaintext `data/interactions/…jsonl`
  day-files are gone. The menu's **Export my data…** writes a single flat,
  OLAP-ready `data/interactions.jsonl` snapshot (one row per turn) and reveals it —
  so the default at-rest state is encrypted and plaintext exists only when you ask
  for it. The `flush_ms` config knob is removed.

## [0.4.6] — 2026-07-18

### Added
- **OpenClaw / almaclaw is now tracked** as its own `openclaw` app entity (like
  Claude Code and Codex), reading `~/.openclaw*/agents/*/sessions/*.jsonl`.
  Provider comes from the model (Anthropic/OpenAI/…) or falls back to `openclaw`.
  Validated against real data: 136 sessions parsed. Handles OpenClaw's mixed
  string/numeric timestamps and strips its inbound-message envelope.

### Fixed
- **Menu-bar icon is now real-time.** Transcript changes are detected instantly
  via file-system events (FSEvents, `notify`) instead of a 2 s poll, and the
  "active" window dropped from 45 s to 6 s — so the icon lights up as soon as a
  message is recorded and clears a few seconds after you stop, instead of being
  stuck Active during a whole session and lingering.
- **The update menu no longer gets stuck** on "You're on the latest version"; the
  transient states (up-to-date, install failed) auto-revert to "Check for updates…"
  after a few seconds.

## [0.4.5] — 2026-07-18

### Added
- **Over-the-air updates from GitHub Releases.** The installed app checks on
  launch and every ~6 hours; when a newer release exists the menu offers **Install
  update X.Y.Z** (plus a manual **Check for updates…**), which downloads the
  release `.dmg`, verifies its signature, atomically replaces the app in
  `/Applications`, and relaunches. Uses the team's existing `gh` auth (no embedded
  tokens, no notarization); self-signed. Gated to an installed `.app` only (dev
  builds are skipped). Each release must attach the `.dmg` (see INSTALL.md §6).

## [0.4.4] — 2026-07-18

### Fixed
- **Menu-bar icon now updates immediately.** It previously reacted only to the
  app's own transcript poll (up to 5 s late) and **not at all to web chats** (the
  native host writes separately). The icon now reflects the store's latest
  activity every second, so a web reply lights it up within ~1 s and short
  interactions no longer go unshown. Transcript polling also tightened to 2 s.
- **Pause now covers web chats.** The pause deadline is shared via the store, so
  the browser native host drops web turns while paused (it is a separate process
  and previously kept recording).
- **Retry/edit no longer duplicate a turn.** An exact consecutive-duplicate turn
  (e.g. a regenerate that re-sends the same prompt) is suppressed; history stays
  append-only otherwise, so every prior state is still recorded.

### Changed
- **Code carries no comments.** Per the project's convention, the source is
  self-documenting and the rationale lives in the docs (README/AGENTS/CHANGELOG);
  all inline and doc comments were removed across Rust and the extension JS. New
  `settings` table (additive, no data migration) backs the shared pause state.

## [0.4.3] — 2026-07-18

### Removed
- **Network-presence collection is gone entirely** (Layer B: `libproc` process →
  AI-endpoint polling). It only produced content-free "an app was open" intervals
  with no research value and added noise, a dependency, and complexity. The
  `presence` table, its day-file table, the `libproc` dependency, and the network
  attribution code are all removed. The monitor now records only the actual
  messages: transcripts (CLI/agents) and web chats (extension). Native desktop
  apps are consequently not captured — a deliberate trade for signal quality.

### Changed
- **One standardized record shape.** Day files are a single flat table
  `data/interactions/YYYY-MM-DD.jsonl` (the `presence` table is gone); every
  source emits the identical row. Schema stays `aum/3`; DB schema is v4.
- **Menu bar UX.** No emojis — plain status text ("Watching for AI use",
  "Recording AI activity", "Paused"). Added a version header (`AI Usage Monitor
  <version>`) — the standard place a menu-bar app shows its version. Icon: hollow
  ring when quiet, filled disc while recording, bars when paused.

### Fixed
- **Web chats now capture the assistant reply and don't double-record.** The
  reply is read from the rendered DOM after the response settles (polling until it
  stabilizes) instead of the provider's undocumented streaming format, and the
  conversation id is read from the page URL *after* settling — so a new chat's
  first message no longer lands under a throwaway id separate from the rest.
- **Claude Code slash-command noise** (`<local-command-*>` / `<command-*>`) is
  filtered from prompts.

## [0.4.2] — 2026-07-17

### Changed
- **Day files are now OLAP-ready.** Export moved from one nested-`turns` record
  per session to two flat, single-grain fact tables partitioned by day:
  `data/interactions/YYYY-MM-DD.jsonl` (one row per turn) and
  `data/presence/YYYY-MM-DD.jsonl` (one row per interval). Rows are flat,
  denormalized (each turn carries its provider/tool/surface/model), and have a
  stable `event_id` for idempotent loads. A warehouse reads them with no unnest
  or join. Schema tag is now `aum/3`.
- **Turns export incrementally.** A per-session `exported_seq` high-water mark
  replaces the old per-session `exported_at` flag, so a growing session appends
  only its new turns instead of re-emitting the whole session — fixing the
  duplicate/partial rows a multi-message web chat produced.
- **Menu-bar icon is now informative.** A hollow ring when quiet, a filled disc
  while AI activity is being recorded (decaying ~45s after the last interaction).
  The old always-on ring-with-dot barely changed and read as uninformative.

### Fixed
- **ChatGPT web captured the prompt but not the reply.** The reply parser matched
  an older internal SSE shape. It now reads the completed assistant message from
  the rendered DOM (stable) after the stream ends, and takes the conversation id
  from the `/c/<id>` page URL — no dependence on the provider's undocumented,
  changing stream format. (Reverse-engineered surfaces; validated against sample
  inputs, confirm live.)
- **Claude Code slash-command noise** (`<local-command-*>`, `<command-*>` wrappers
  from `/model` etc.) was stored as user prompts. Those synthetic entries are now
  filtered; only real prompts are recorded.

## [0.4.1] — 2026-07-17

### Fixed
- **Nothing was recorded after upgrading from 0.3.x.** An existing `sessions.sqlite`
  kept the pre-0.4 schema (`source_kind`/`app_hash`, no `tool`/`provider`), and
  `CREATE TABLE IF NOT EXISTS` left it in place, so every session query failed with
  `no such column: tool` and both storage and export were dead. The store now
  tracks a `PRAGMA user_version` and migrates a pre-0.4 DB by dropping and
  rebuilding the incompatible `sessions`/`turns` tables (that data was from the
  retired screen-scrape approach; nothing to preserve).
- **Detection silently never ran, and the icon stuck on "Recording an AI chat".**
  The poll timers initialized to `i64::MIN`, and `now - i64::MIN` overflows and
  wraps in release builds — so the transcript and network polls always read as
  "not due" (never ran) and the icon read "Recording" forever. All monotonic
  time-deltas now use `saturating_sub`.

## [0.4.0] — 2026-07-17

Fundamental change of approach. Screen-scraping AI detection is removed: reading
the screen with OCR and a streaming heuristic was unreliable at the mechanism
level — it produced false positives (Slack, editors), garbled content, no app
identity, and it missed the CLI/agent tools entirely. Detection is now based on
two reliable signals, and the change was validated against real data on a real
machine (184 Claude Code + 40 Codex sessions parsed; the live ChatGPT/Claude
apps, Codex, and Claude Code all detected on the network).

### Changed
- **Detection is now transcript ingestion + network presence, not screen
  capture.**
  - **Layer A — transcripts.** AI coding tools persist structured local
    transcripts; the monitor reads them directly (Claude Code
    `~/.claude/projects/*/*.jsonl`, Codex `~/.codex/**/rollout-*.jsonl`),
    yielding exact prompt/reply, timestamps, model, and session id with zero
    OCR and zero false positives, across all desktops/Spaces.
  - **Layer B — network presence.** For AI with no local transcript (web chats,
    native apps), the monitor observes process → AI-endpoint connections via
    `libproc` (no root, no entitlement). AI tools/apps are attributed by process
    identity; browsers by provider-owned IP range.
- **Records now carry real identity.** Each day-file line names the `provider`
  (`anthropic`, `openai`, …), `tool`, `surface`, and `model`, with the exchange
  as structured `turns`. The old salted app-hash is gone — for a consenting
  internal study the provider entity is the signal, not something to hide. Schema
  is now `aum/2`; `presence` records are a second kind alongside `interaction`.
- **No screen-recording or accessibility permission.** The app reads files the
  user owns and observes its own sockets; a rebuild never loses a TCC grant.
- **Menu.** Removed "Peek under the hood"; the status line reflects the new
  signals.

### Removed
- The OCR/Vision + ScreenCaptureKit capture stack, the streaming-signature
  detector, the Accessibility reader, and their dependencies. Large net code and
  dependency reduction.

### Added
- **Layer C — browser web-chat capture.** A Chromium extension (`extension/`)
  intercepts the AI site's own API calls (fetch/SSE) — reliable and working in
  background tabs — and delivers each web chat to a local native-messaging host
  (`--native-host`), which redacts and stores it as a `web` session grouped under
  the right provider. Strictly local: the extension talks only to the local host,
  which has no egress. Install the host into every Chromium browser with
  `--install-browser-host`. Covers ChatGPT and Claude web; endpoint shapes are
  reverse-engineered and each needs one live confirmation.
- `--diagnose` now reports what each layer sees live: per-tool transcript counts
  and every AI network connection active on the machine right now (content-free).

### UX
- **Menu-bar states redesigned.** Three honest states: **Monitoring** (steady),
  **Recording** (a brief flash only when an interaction is actually recorded), and
  **Paused**. The old presence-driven "in use nearby" state is gone — a
  backgrounded AI app holds a network connection indefinitely, so that state never
  cleared and read as stale. Presence still feeds the data, just not a persistent
  icon state.

## [0.3.0] — 2026-07-17

### Fixed
- **The icon/state got stuck showing "recording" forever.** The detector
  reported the best growth run anywhere in its rolling window; for a surface
  sampled only on full sweeps that window spans ~a minute, so a finished reply
  kept reading as streaming and the session never closed. It now reports on the
  run that is still active at the latest sample, so it returns to idle promptly
  when a reply finishes. The surface's detector is also reset on close.

### Changed
- **Friendlier, calmer UX.** No more "Recording" — the menu reads in plain,
  light language ("Catching an AI chat ✨", "Keeping an eye out 👀", "All quiet
  for now 🌙", "Taking a break ☕"). The menu-bar icon is now icon-only (its
  shape is the state), removing the persistent text label that could stick.
- **Simpler menu.** Trimmed to a status line, a captured-today count, **Take a
  break** (15 min / an hour / until you're back), **Show my data**, and Quit.
- **No more manual "Export for review."** Finished sessions are stored
  automatically, redacted, into **day files** `data/YYYY-MM-DD.jsonl` — the
  standard shape for analytics at scale and trivial to merge across machines
  (each line carries the device id). "Show my data" reveals the folder.
- **Much cleaner captured data.** Instead of dumping the whole window (history +
  UI chrome), each record stores just the **prompt** and the **reply** (the
  reply is the text that appeared during the session, diffed from the pre-reply
  baseline) as separate fields, with a lean schema: `device, day, app, surface,
  started_ms, ended_ms, prompt, reply`.

## [0.2.5] — 2026-07-16

### Fixed
- **Native apps: "detects the app but not the message."** The Accessibility path
  monitored only the single LARGEST text region of a window. In a chat app each
  message is its own element, so a newly-streaming reply is small and never the
  largest — the monitored region stayed static while a *different* region grew,
  and the reply went undetected. It now concatenates ALL non-input text regions
  (what OCR already does for the whole window), so any growing region raises the
  total. Measured against the live Claude desktop app: the window read jumped
  from 5041 chars (a chrome-heavy block, prose score 0.40) to **63535 chars of
  the actual conversation (prose score 0.95)** — and a new message now moves that
  number. Verified end-to-end via Accessibility on the real app (reads the full
  conversation; correctly idle when nothing is streaming).

### Added
- A periodic INFO **heartbeat** in the activity log ("reading N window(s) this
  tick; most prose seen: X chars"). Opening the log now tells capture apart from
  detection without turning on debug: `reads = 0` → permissions/capture;
  `reads > 0` but no sessions while an AI streams → detection.

## [0.2.4] — 2026-07-16

### Fixed
- **Real app windows were rejected by the prose-score gate.** Diagnosing against
  the *actual* open apps (not a synthetic page) showed a real chat window — e.g.
  Claude's desktop app, 5000+ chars via Accessibility — scores ~0.40 on the
  whole-window prose gate because UI chrome (sidebar, buttons, timestamps)
  dominates. The old detector required 0.55, so it rejected genuine AI windows
  even while a reply streamed. This was the core "detects apps but not messages"
  cause on real apps.

### Changed
- **Detection reworked to track growth of PROSE content, not the whole window.**
  `prose_len` counts letters only in natural-language lines (structured/log
  lines excluded), so a window's fixed chrome cancels between frames and only
  the streamed reply moves the number. Detection is a sustained run of new prose
  highs, measured against a running peak so an OCR dip-then-recover isn't read as
  a shrink, with a stall tolerance so a busy chat's discrete incoming messages
  don't accumulate into one false "generation". The whole-window prose-score gate
  is gone. This is validated by unit tests over realistic prose-length
  trajectories (jitter, message arrivals, scene changes) — the behaviors that
  can't be reproduced reliably by scripting a GUI.

### Notes on scope (honest limitations)
- **Native AI apps work across Spaces/desktops** (Accessibility reads them even
  when off-screen). A **browser** AI window on *another* desktop cannot be
  screen-captured (macOS renders nothing to capture off-Space) — keep it on the
  current desktop, or use the native app.
- After installing, macOS may hold a **stale Screen Recording grant** from an
  earlier build: if capture seems dead, remove the app under System Settings →
  Screen Recording and re-add it.

## [0.2.3] — 2026-07-16

### Fixed
- **Chat replies weren't detected because the composer stays focused.** The
  "user is typing" signal treated *any focused text input* as typing — but chat
  apps (ChatGPT, Claude, …) keep the message box focused while the model streams
  its reply, so every frame of the reply was excluded and no session was ever
  created. Typing now means the focused input's **text is changing** (an actual
  keystroke), not merely that it is focused. Verified end-to-end against a real
  streaming page with the composer focused: the session is now captured.

### Validated
- The streaming-signature approach is confirmed correct end-to-end by driving a
  real browser through the live capture path: OCR/AX text grows, the detector
  fires `Streaming`, and a session is created and persisted. It works for any
  surface that streams prose — no per-provider logic.

### Added
- [docs/grouping.md](docs/grouping.md): how sessions are grouped by provider and
  surface (web/app/CLI) at analysis time — without hardcoding providers and
  without adding any network/LLM call to the local-only daemon.
- Content-free per-surface detection tracing (`RUST_LOG=ai_usage_monitor=debug`):
  text length + verdict, no captured text — for tuning and support.

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

[0.4.9]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.4.9
[0.4.8]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.4.8
[0.4.7]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.4.7
[0.4.6]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.4.6
[0.4.5]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.4.5
[0.4.4]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.4.4
[0.4.3]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.4.3
[0.4.2]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.4.2
[0.4.1]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.4.1
[0.4.0]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.4.0
[0.3.0]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.3.0
[0.2.5]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.2.5
[0.2.4]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.2.4
[0.2.3]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.2.3
[0.2.2]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.2.2
[0.2.1]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.2.1
[0.2.0]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.2.0
[0.1.0]: https://github.com/memfoldai/ai-usage-monitor/releases/tag/v0.1.0
