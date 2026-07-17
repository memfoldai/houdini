# Entity grouping and analysis-time clustering

How the study groups usage by *who* the AI is (Claude app + CLI + web → one
`anthropic` entity) and, above that, by *what people are doing* (research vs
build, topic) — without the daemon making any network call or hardcoding content
rules.

## Provider grouping is deterministic, at ingest

Unlike the old screen-scraping approach, the provider is now known for free from
the source, so grouping does not need an LLM or a guess:

- **Transcript sessions (Layer A)** carry the provider directly. The adapter
  reads a single-vendor tool (`claude-code` → `anthropic`); a multi-model tool is
  resolved by the model name it records (`claude-*` → anthropic, `gpt-*`/`o*` →
  openai, `gemini-*` → google — see `src/attribution.rs`).
- **Network presence (Layer B)** resolves the provider from the process identity
  (the `codex`/`ChatGPT`/`Claude` binaries) or a provider-owned destination
  range.

So every exported record already names its `provider`, `tool`, and `surface`.
Rolling "Claude app + Claude Code + claude.ai" into one `anthropic` entity is a
`GROUP BY provider` over the day files — no labeling step required.

This is a small, maintained registry of provider *metadata* (which tool/binary/
range belongs to which vendor), not content classification. A brand-new tool is
covered by adding one adapter/rule; until then Layer B still catches its network
presence generically.

## Semantic clustering is an analysis-time job (optional LLM)

The higher-level question — *what* was the AI used for (research, coding,
writing; which topic) — is the one place semantics are unavoidable. It runs
**offline, over the exported and redacted day files, never in the daemon**:

1. Batch a labeler over each `interaction` record's redacted `turns`, emitting a
   task/topic label.
2. An LLM fits here and matches the "async job, not every frame" intuition: it
   runs **once per session, offline, on already-redacted text**, so it sees no
   raw content and the always-on app still makes zero network calls.
3. Roll the labels up for the study's clustering.

## Kept out of the daemon, on purpose

- No LLM or network call in the monitor — it would break the
  [local-only guarantee](../SECURITY.md) for a labeling task that belongs at
  analysis time.
- No content classifier in capture — provider comes from source metadata, task/
  topic comes from the offline analysis pass.

## Known gap

ChatGPT/Gemini used **in a browser tab** are not attributed by the network layer
(shared CDNs; see the README). Their transcript-less content is only reachable
via a browser extension, a later layer; today those show up only when used
through their native apps or CLIs.
