# Security

Houdini runs on team members' own machines and reads AI tools' local transcripts
and web chats, so its safety properties matter. The **data/consent model** (what
is collected, local-only, encryption, redaction) lives in
[docs/privacy.md](docs/privacy.md); this page covers reporting and the
security-specific posture.

## Reporting

It is an internal tool. Report a concern privately to the maintainer, Rahul Biliyar
(<rahul@memfold.ai>), not via a public issue.

## Browser extension scope

The optional Chromium extension reads the conversation in the user's own tab and
sends it to the **local** native host over native messaging. It has no network
permission and no egress, and matches only the configured AI hosts (`chatgpt.com`,
`claude.ai`, `gemini.google.com`). This is the same content-reading mechanism some
malicious extensions abuse to *exfiltrate* chats. It is legitimate here only
because it is local, redacted, and installed per consent.

## Dependency auditing

Dependencies are checked against the [RustSec advisory database](https://rustsec.org/)
with [`cargo-audit`](https://github.com/rustsec/rustsec); CI runs it on every push
(`.github/workflows/ci.yml`). `cargo audit` may report unmaintained/unsound
advisories (warnings, not vulnerabilities) for GTK-stack crates that are
**Linux-only** transitive deps of `tray-icon`, absent from the macOS build graph.
Verify with:

```bash
cargo tree -e no-dev --target aarch64-apple-darwin | grep -iE 'glib|gtk'  # empty
```

## Code signing

A stable code-signing identity is recommended for a clean install (see
[docs/install.md](docs/install.md)), but the app depends on no TCC grant, so a
rebuild never silently loses capability. The bundle is menu-bar-only
(`LSUIElement`) and declares no usage-description entitlements.

## Memory & CPU

Rust's ownership model rules out use-after-free and data races; the only `unsafe`
is the AppKit/tray FFI in the app shell. In-memory state is bounded: the ingest
fingerprint map holds one small `(mtime, size)` per transcript file, and the
diagnostics log is capped (rotated past ~1 MB). CPU is low by design: the
transcript scan is a cheap file scan on a multi-second cadence
(`transcript_poll_ms`, default 2 s) that **pausing stops**; web capture is
event-driven. The store grows over time by design (text-only, single-digit MB/day).
