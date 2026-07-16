# Verification — human-gated steps

The portable core (detector, redaction, store, session, export) is covered by
`cargo test` and runs anywhere. The **native runtime cannot be self-verified**:
it needs real TCC permission grants, real on-screen AI sessions, a status-bar
item, and a stable code signature — none of which exist in a headless/CI
environment. This document is the checklist a person runs once on a real Mac
before trusting the app or sharing any extract.

Do these in order. Each has an explicit pass condition.

---

## 0. Build, sign, launch

```bash
cargo test                 # portable core must be green
cargo build --release
scripts/sign.sh            # stable self-signed identity (see the script header)
./target/release/ai-usage-monitor
```

**Pass:** a dot appears in the menu bar. Gray = idle. Clicking it shows
“Export extract for review…” and “Quit”.

> Why signing matters: TCC keys the grants below on the binary's code identity.
> An unsigned/ad-hoc build loses every grant on the next rebuild. Re-run
> `scripts/sign.sh` after each rebuild so the grants persist.

---

## 1. Grant permissions

On first launch the app prompts for **Accessibility** and **Screen Recording**.
If you miss the prompts, grant them manually:

- System Settings → Privacy & Security → **Accessibility** → enable
  `ai-usage-monitor`.
- System Settings → Privacy & Security → **Screen Recording** → enable
  `ai-usage-monitor`.

Screen Recording only takes effect after a relaunch (Apple's behavior). Quit and
relaunch.

**Pass:** with any windows open, the menu-bar dot turns **blue** (armed —
watching open windows).

---

## 2. Real AI session is captured (true positive)

1. Open ChatGPT or Claude (browser or native app) and send a prompt whose answer
   streams in as prose (e.g. “Explain regenerative agriculture in a paragraph”).
2. Watch the dot while the answer streams.

**Pass:** the dot turns **green** (capturing) while the answer streams, then
returns to blue a few seconds after it stops. Click **Export extract for
review…**, then inspect the newest file under the export dir:

```
~/Library/Application Support/ai.memfold.ai-usage-monitor/exports/extract-*.jsonl
```

**Pass:** the file contains one session line whose message content is the
conversation you just saw (redacted), with `aum.capture.source` of `ocr`
(browser) or `ax` (native app) and a hashed `aum.app.hash` (never the app's
real name). Timestamps (`aum.session.start_time_unix_ms`) are real wall-clock
epoch times.

---

## 2b. Concurrent / background / other-desktop sessions

The monitor tracks every window independently. Verify the three parallel
scenarios:

1. **Two at once:** start a streaming answer in ChatGPT (browser) and, while it
   streams, start one in another AI app (or a second browser window). Export.
   **Pass:** two separate sessions with different `aum.app.hash` values (or two
   sessions of the same app, if you used two windows of one app).
2. **Backgrounded mid-stream:** start a long streaming answer, then click into
   a different app while it streams (the AI window is now in the background,
   possibly occluded). **Pass:** the session still captures the full answer
   (the background window is picked up by the full sweep, ~every 2 s).
3. **Another desktop/Space:** move the streaming AI window to another Space
   (Mission Control) while it streams. **Pass:** same as above — the extract
   contains the completed answer. Window enumeration is Space-independent
   (`onScreenWindowsOnly = false`) and window capture is desktop-independent.

Known limit to note: a *browser tab* that is not the window's visible tab
renders nothing, so it cannot be captured by any means — only visible surfaces
(in any window, on any Space) are observable.

---

## 3. Redaction audit — seeded secret + personal detail (safety gate)

Before ANY real extract is shared, prove redaction catches planted values.

1. In an AI chat, paste a message containing a **fake** AWS-shaped key and a
   **fake** personal detail, e.g.:

   ```
   Here is my key AKIAIOSFODNN7EXAMPLE and email jane.doe@example.com,
   call +1 415-555-0132.
   ```

2. Let it get captured (dot goes green), then Export.

**Pass:** in the exported JSONL, none of `AKIAIOSFODNN7EXAMPLE`,
`jane.doe@example.com`, or `415-555-0132` appear as raw text; each is replaced by
a `[REDACTED:…]` placeholder. If any raw value survives, **do not share the
extract** — file the gap first.

(The deterministic layer is unit-tested for these exact shapes in
`src/redact.rs`; this step confirms it end-to-end through real capture.)

---

## 4. Non-AI activity must NOT be captured (false-positive gate)

The detector must not fire on text that merely grows. Test each:

- **You typing** a long message into any composer (email, Slack, a chat box).
- **A build log / test output** streaming in a terminal (`cargo build`, `npm
  install`, a CI tail).
- **Scrolling** a long article or code file.

**Pass:** in all three the dot stays **blue** (never green), and after Export
there is **no** session for them. Typing is excluded because the caret is in an
input; logs are excluded because they read as structured output, not prose.

If any of these produce a session, the detector thresholds need tuning — adjust
`detector` in `config.json` (see README) and re-run this step. This gate is the
one most likely to need a tuning pass on real machines; treat a failure here as
expected iteration, not a blocker.

---

## 5. Optional: NER redaction layer (feature `ner`)

Skip unless you enabled the NER layer — [docs/NER.md](docs/NER.md) owns the
setup. Once it is running, two checks belong here:

**Catches what regexes can't.** Repeat step 3 with a planted **person name**
(e.g. “Contact Maria Gonzalez”). **Pass:** after Export the name is replaced by
`[REDACTED:NER:PERSON]`.

**Fails closed.** Point `ner_model_dir` at a directory with no valid model and
relaunch. **Pass:** the app logs the load failure and continues with
deterministic-only redaction — it must not crash, and must not silently claim
NER coverage it doesn't have.

---

## Known limits (state these when sharing results)

- Detector thresholds are seeded from reasoning, not yet tuned on a real corpus.
  Step 4 is the tuning loop; expect one or two passes.
- OCR quality bounds browser capture: tiny fonts or heavy theming degrade the
  captured text. AX-readable native apps are exact.
- The app captures the visible conversation snapshot per session (role
  `unknown`); it does not split user vs assistant turns. That's deliberate —
  structural turn segmentation is a later, separate build step.
