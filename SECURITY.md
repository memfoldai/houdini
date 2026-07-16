# Security & resource notes

This app runs on team members' own machines and reads on-screen text, so its
safety properties matter. This documents the posture and the standing review.

## Reporting

It is an internal tool. Report a concern privately to the maintainer
(design@memfold.ai), not via a public issue.

## Data handling (the core guarantees)

- **Local only.** No network egress anywhere in the code path; nothing uploads.
- **Redaction is a hard gate.** Secrets and structured PII are removed *before*
  any text reaches disk (`src/redact.rs`); the store never holds raw text.
- **Text only.** Screenshots are used transiently for OCR and never stored.
- **Anonymized.** The source app is stored as a per-install salted hash; the
  salt never leaves the machine.
- **Logs are metadata-only.** The diagnostics log records lengths, counts, and
  hashed identifiers — never captured text (`src/logging.rs`).
- **Two-gate export.** Automatic redaction, then human review before sharing.

## Dependency auditing

Dependencies are checked against the [RustSec advisory database](https://rustsec.org/)
with [`cargo-audit`](https://github.com/rustsec/rustsec); CI runs it on every
push (`.github/workflows/ci.yml`).

Current status: **0 vulnerabilities** across the dependency graph. `cargo audit`
also reports a few *unmaintained/unsound* advisories (warnings, not
vulnerabilities) for GTK-stack crates (`glib`, `gtk`, `paste`, …). These are
**Linux-only** transitive dependencies of `tray-icon` and are **not in the
macOS build graph** — verify with:

```bash
cargo tree -e no-dev --target aarch64-apple-darwin | grep -iE 'glib|gtk'  # empty
```

They never compile into the shipped binary, so they are left visible (not
suppressed) rather than hidden behind an ignore list.

## Code signing

Distributed builds are signed with a stable identity (hardened runtime); an
unsigned build would lose its TCC grants on every launch. See INSTALL.md. The
app requests only the two grants it uses (Accessibility, Screen Recording) and
declares them in `Info.plist`.

## Memory & CPU

Rust's ownership model rules out use-after-free and data races; there is no
`unsafe` outside the documented native-FFI boundary. In-memory state is bounded:

- Per-surface trackers, the AX-window registry, and the OCR-throttle map are all
  **pruned** when a window disappears (a full sweep prunes anything not
  re-seen), so none grows without bound.
- The detector keeps a fixed-size rolling window of samples.
- The diagnostics log is **capped** (rotated past ~1 MB).

CPU is kept low by design:

- **OCR is throttled** per window (`ocr_min_interval_ms`, ~800 ms) and bounded
  per sweep (`max_ocr_per_sweep`) — the screenshot+Vision pass is the only
  expensive step, and it is decoupled from the sample rate.
- **Fast ticks skip window enumeration** when the frontmost app is AX-readable.
- **Pausing stops all capture work** — CPU drops to idle.

The one store that grows over time is the SQLite database, by design (the study
keeps everything). It is **text-only** — single-digit MB/day — not screenshots.
Prune it by deleting old rows or the DB file if a machine runs the pilot for
months; nothing depends on unbounded history at runtime.
