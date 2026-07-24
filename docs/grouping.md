# Entity grouping and analysis-time clustering

How usage is grouped by *who* the AI is (Claude app + CLI + web → one
`anthropic` entity) and, above that, by *what people are doing* (research vs
build, topic), without the daemon making any network call or hardcoding content
rules.

## Provider grouping is deterministic, at ingest

Unlike the old screen-scraping approach, the provider is now known for free from
the source, so grouping does not need an LLM or a guess:

Every record carries the provider directly. A transcript adapter or web site
knows its single-vendor tool (`claude-code` → `anthropic`, `chatgpt-web` →
`openai`); a multi-model tool is resolved by the model name it records
(`claude-*` → anthropic, `gpt-*`/`o*` → openai, `gemini-*` → google; see
`src/attribution.rs`).

So every exported row already names its `provider`, `tool`, and `surface`. Rolling
"Claude Code + claude.ai" into one `anthropic` entity is a `GROUP BY provider`
over the interactions file, with no labeling step required.

This is a small, maintained registry of provider *metadata* (which tool belongs to
which vendor), not content classification. A brand-new tool is covered by adding
one adapter.

## Semantic labeling runs on the device, against a versioned taxonomy

The higher-level question, *what* was the AI used for, is the one place
semantics are unavoidable. It runs in the app, on already-redacted text, as a
periodic background job (`src/analytics.rs`, `src/analytics_job.rs`):

1. Only **user** turns are labeled. Assistant output outnumbers user requests by
   roughly twenty to one and carries the model's words, not the person's intent.
2. Each turn is classified against a **closed, versioned taxonomy**
   (`src/taxonomy.rs`) using strict JSON-schema structured output, so the model
   cannot emit a label outside the enum.
3. Labels land in `turn_labels`, each row pinning its taxonomy version, prompt
   version and model, so two machines running the same version produce directly
   comparable rows.
4. Anything that genuinely does not fit is labeled `other` and recorded in
   `label_candidates` as a proposal, never as a freely invented category.

## Why a taxonomy instead of clustering per device

Clustering each machine independently is the failure mode this design exists to
avoid: one laptop invents "debugging code", another "fixing bugs", and merging
them afterwards is an unsolved ontology-alignment problem. Pinning the label
space by version makes the two machines structurally incapable of disagreeing,
and routes genuine novelty into a proposal queue that a human promotes into the
next taxonomy version. Nothing is missed, and nothing is reinvented.

Provider, tool and surface grouping stays exactly where it was: resolved
deterministically at ingest from source metadata, never from content.

## Known gap

**Native desktop *chat* apps** (ChatGPT.app, Claude.app) keep conversations
server-side and are out of scope. Web ChatGPT,
Claude, and Gemini are captured by the browser extension; Codex run inside the
ChatGPT desktop app is captured via its `~/.codex` transcripts.
