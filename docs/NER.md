# Optional NER redaction layer (`ner`)

The always-on deterministic redactor (`src/redact.rs`) removes high-confidence
**shapes**: provider-prefixed API keys, private-key blocks, emails, Luhn-valid
cards, dashed SSNs, phone numbers. It cannot catch free-form personal
identifiers that have no fixed shape — a person's name, a street address, a date
of birth in prose.

The optional NER layer (`src/ner.rs`, Cargo feature `ner`) fills that gap with a
**GLiNER-PII** model run fully offline via [`gline-rs`](https://crates.io/crates/gline-rs)
(crate lib name `gliner`) over ONNX Runtime. It runs at **export time**, on top
of the already-deterministically-redacted text — never on the hot capture path.

The model is **never bundled**. You provision it; the app points at it.

## Why it's off by default

- It adds a ~hundreds-of-MB ONNX model and the ONNX Runtime native library.
- It's a probabilistic layer; the deterministic layer is the guaranteed one.
- The study's default posture is to keep research **content** (the topics,
  companies, and places someone researched) and remove only **personal**
  identifiers. The default label set reflects that — see below.

## 1. Provision the model

Download a **token-mode** GLiNER-PII export (tokenizer + ONNX). For example the
Knowledgator PII models:

- <https://huggingface.co/knowledgator/gliner-pii-base-v1.0>
- <https://huggingface.co/knowledgator/gliner-pii-large-v1.0>

Place the two files in one directory:

```
<model-dir>/
  tokenizer.json
  model.onnx
```

> The loader expects exactly those two filenames. If your download nests the
> ONNX under `onnx/model.onnx`, copy or symlink it to `<model-dir>/model.onnx`.
> The model must be a **token-mode** export — the app loads `GLiNER<TokenMode>`.
> A mismatch is caught by the self-test (below), not silently ignored.

## 2. ONNX Runtime

`gline-rs` pulls `ort` (ONNX Runtime). Building `--features ner` links the
native runtime; on most setups `ort` provisions a prebuilt library at build
time. If your environment blocks that, install ONNX Runtime yourself and point
`ort` at it per its docs (<https://ort.pyke.io/>). This is an operator step —
the default build does not include any of it.

## 3. Enable it

```bash
cargo build --release --features ner
```

Set the model directory in
`~/Library/Application Support/ai.memfold.ai-usage-monitor/config.json`:

```json
{
  "ner_model_dir": "/absolute/path/to/model-dir"
}
```

Relaunch. On startup you should see:

```
NER redaction layer loaded from /absolute/path/to/model-dir
```

That line means the model **passed its seeded self-test**.

## The seeded self-test (fail-closed)

When the model loads, the app runs a fixed probe sentence with a planted name
through it and requires at least one detection. If the model detects nothing —
a wrong export, a span/token mismatch, a corrupt file — `load` returns an error,
the app logs it, and it continues with **deterministic-only** redaction. A model
that would silently redact nothing is therefore never trusted to stand in for
the redaction layer. Verify this yourself with the fail-closed check in
[../VERIFICATION.md](../VERIFICATION.md) step 5.

## Default labels (personal identifiers only)

From `DEFAULT_PII_LABELS` in `src/ner.rs`:

```
person, email, phone number, home address, date of birth,
credit card number, social security number, passport number,
driver license number, bank account number, ip address
```

Generic `organization` and `location` are intentionally **excluded** — redacting
the company or city someone researched would destroy the study's signal. If your
threat model needs them, pass a custom label set via
`NerRedactor::load_with_labels`.

## How redaction composes

`NerRedactor::redact(text)`:

1. runs `redact_deterministic` (the shapes) — so NER never re-sees a value the
   deterministic layer already removed;
2. runs the model over what remains, with the label set;
3. replaces each accepted span (probability ≥ 0.5, ≥ 3 chars) with
   `[REDACTED:NER:<CLASS>]`, e.g. `[REDACTED:NER:PERSON]`;
4. returns the combined text plus a share-safe audit (counts per kind and per
   class, never raw values).

Spans are replaced longest-first so a longer name isn't half-consumed by a
shorter overlapping one. Replacement is by matched substring (not byte offsets),
which sidesteps the model's internal sentence-splitting and errs toward removing
more — the safe direction for redaction.

## Testing against a real model

The unit test that exercises a real model is `#[ignore]` (it needs the model):

```bash
AUM_NER_MODEL_DIR=/absolute/path/to/model-dir \
  cargo test --features ner ner_ -- --ignored
```

It confirms a planted person name is removed on top of a deterministically
redacted email.
