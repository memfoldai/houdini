# Security & resource notes

This app runs on team members' own machines and reads AI tools' local transcripts
and their network activity, so its safety properties matter. This documents the
posture and the standing review.

## Reporting

It is an internal tool. Report a concern privately to the maintainer
(design@memfold.ai), not via a public issue.

## Data handling (the core guarantees)

- **Local only.** No network egress anywhere in the code path; nothing uploads.
  The daemon makes zero network calls — it only reads local files and observes
  its own user's sockets.
- **No screen capture, no TCC permission.** The app never reads the screen and
  never requests Screen Recording or Accessibility. It reads transcript files the
  user already owns and enumerates its own processes' connections via `libproc`.
- **Redaction is a hard gate.** Secrets and structured PII are removed *before*
  any interaction text reaches disk (`src/redact.rs`); the store never holds raw
  content. Network presence records carry **no content** at all.
- **Identity is intentionally in the clear.** For a consenting internal study the
  provider/tool (`anthropic`, `claude-code`) is the research signal, so it is
  stored plainly; only message content is redacted. The one per-install value is
  a random `install_id` (device id), which reveals nothing.
- **Logs are metadata-only.** The diagnostics log records counts and provider
  names — never ingested text (`src/logging.rs`).

## Network observation scope

Layer B reads, per own-user process, the remote IP of established TCP sockets —
the same visibility `lsof` has without `sudo`. It does **not** read packet
contents, does not decrypt TLS, and cannot see other users' or system processes'
connections. It is a read-only enumeration; nothing is injected or blocked. No
`com.apple.developer.networking.networkextension` entitlement or system extension
is used.

## Dependency auditing

Dependencies are checked against the [RustSec advisory database](https://rustsec.org/)
with [`cargo-audit`](https://github.com/rustsec/rustsec); CI runs it on every
push (`.github/workflows/ci.yml`). The heavy Vision/ScreenCaptureKit frameworks
are gone as of 0.4.0, shrinking the graph. `cargo audit` may still report
unmaintained/unsound advisories (warnings, not vulnerabilities) for GTK-stack
crates that are **Linux-only** transitive deps of `tray-icon` and not in the macOS
build graph — verify with:

```bash
cargo tree -e no-dev --target aarch64-apple-darwin | grep -iE 'glib|gtk'  # empty
```

## Code signing

Signing with a stable identity is still recommended for a clean install and
distribution, but the app no longer depends on any TCC grant, so a rebuild never
silently loses capability. See INSTALL.md. The bundled app keeps `LSUIElement`
(menu-bar-only); it no longer needs `NSScreenCaptureUsageDescription`.

## Memory & CPU

Rust's ownership model rules out use-after-free and data races; the only `unsafe`
is the documented `libproc` socket-union access (`src/netpresence.rs`), guarded by
the socket kind/flag it reads. In-memory state is bounded:

- The ingest fingerprint map holds one small `(mtime, size)` entry per transcript
  file; the presence map holds one entry per currently-active provider/process.
- The diagnostics log is **capped** (rotated past ~1 MB).

CPU is low by design: the two detectors poll on a multi-second cadence
(`transcript_poll_ms`, `network_poll_ms`, default 5 s), each is a cheap file scan
or process-table walk, and **pausing stops both** — CPU drops to idle. There is no
per-frame work anymore.

The one store that grows over time is the SQLite database, by design (the study
keeps everything). It is **text-only** — single-digit MB/day. Prune it by deleting
old rows or the DB file if a machine runs the pilot for months.
