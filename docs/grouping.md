# Entity grouping (ChatGPT vs Claude, across web / app / CLI)

How the study groups sessions by *who* the AI is and *what surface* it was used
through — ChatGPT-in-a-browser, the ChatGPT app, and a `codex` CLI should roll
up to one provider — **without hardcoding provider names** and **without
weakening the app's local-only guarantee.**

## The principle: capture provider-agnostic, group at analysis time

The monitor never decides "this is ChatGPT." That would mean an allowlist of
providers (the exact hardcoding the whole approach rejects) or an LLM call from
the always-on daemon (which would break the [local-only, no-network guarantee](../SECURITY.md)).
Instead the daemon captures the **signals** needed to group, and grouping /
labeling happens later, at analysis time, over the redacted export.

## What the daemon captures (the grouping inputs)

Every session in the export already carries enough to group:

| Signal | Field | What it enables |
|---|---|---|
| Same app within a machine | `aum.app.hash` (salted bundle id) | Group all sessions from one app; stable per install, reveals no app name |
| Surface class | `aum.capture.source` = `ax` \| `ocr` | `ax` ≈ native app / terminal, `ocr` ≈ web — a coarse, non-hardcoded surface signal from *how* it was read, not what it is |
| The conversation itself | `gen_ai.output.messages` (redacted) | The semantic signal a labeler uses to identify the provider |

That is: **which app**, **which kind of surface**, and **the (redacted) text** —
the three things any grouping needs.

## How grouping happens (analysis time, optional LLM)

Grouping runs on the exported corpus, after the two-gate redaction + human
review — never in the daemon:

1. **Within-install, same-app** grouping is free: bucket by `aum.app.hash`.
2. **Cross-surface provider** grouping (ChatGPT app + web + CLI → "OpenAI")
   needs semantics. Run a batch labeler over each session's redacted text +
   surface class and have it emit a provider/surface label. An LLM is a good fit
   here and matches the "async job, not every frame" intuition — but it runs
   **once per session, offline, on already-redacted and human-reviewed text**,
   so it sees no raw content and the daemon still makes zero network calls.
3. Roll the labels up into provider entities for the study's clustering.

This keeps the always-on app lean and local-only, puts the one place semantics
are unavoidable (naming a provider) outside the privacy boundary, and never
hardcodes a provider list into capture.

## Not done in the daemon, on purpose

- No LLM/network in the monitor — it would violate the local-only guarantee for
  a labeling task that belongs at analysis time.
- No provider allowlist anywhere in capture — detection stays behavioral.

## A possible future signal (with a privacy cost)

Window titles often name the provider and the conversation topic ("ChatGPT —
Trip to Kyoto"). Capturing titles would sharpen grouping, but titles carry
sensitive topics, so they would need the same redaction gate as message text
before export. Deferred until the analysis loop shows the current signals are
insufficient.
