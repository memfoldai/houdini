# Verification — human-gated steps

The portable core (detector, redaction, store, session, export) is covered by
`cargo test` and runs anywhere. The **native runtime cannot be self-verified**:
it needs real TCC permission grants, real on-screen AI sessions, a status-bar
item, and a stable code signature — none of which exist in a headless/CI
environment. This document is the checklist a person runs once on a real Mac
before trusting the app or sharing any data.

Do these in order. Each has an explicit pass condition.

---

## 0. Build, sign, launch

```bash
cargo test                 # portable core must be green
cargo build --release
scripts/sign.sh            # stable self-signed identity (see the script header)
./target/release/ai-usage-monitor
```

**Pass:** an icon appears in the menu bar — a hollow ring (idle). Clicking it
shows a friendly status plus **Take a break**, **Show my data**, and **Quit**.
(The icon's **shape** is the state — ring = idle, ring-with-dot = watching,
solid disc = catching a chat, two bars = paused.)

> Why signing matters: TCC keys the grants below on the binary's code identity.
> An unsigned/ad-hoc build loses every grant on the next rebuild. Re-run
> `scripts/sign.sh` after each rebuild so the grants persist.

---

## 1. Grant permissions

On first launch the app prompts for **Accessibility** and **Screen Recording**.
If you miss the prompts, grant them manually:

- System Settings → Privacy & Security → **Accessibility** → enable it.
- System Settings → Privacy & Security → **Screen Recording** → enable it.

(Listed as **AI Usage Monitor** for the bundled app, or `ai-usage-monitor` for
the bare dev binary.)

Screen Recording only takes effect after a relaunch (Apple's behavior). Quit and
relaunch.

**Pass:** with any windows open, the icon becomes a **ring with a center dot**
(armed — watching). The status line in the menu reads “Watching N window(s)”.

---

## 2. Real AI session is captured (true positive)

1. Open ChatGPT or Claude (browser or native app) and send a prompt whose answer
   streams in as prose (e.g. “Explain regenerative agriculture in a paragraph”).
2. Watch the dot while the answer streams.

**Pass:** the icon fills to a **solid disc** (capturing) while the answer
streams, then returns to the ring-with-dot a few seconds after it stops (it must
NOT stay solid — the "stuck forever" bug). Click **Show my data** (or wait ~30s)
and inspect today's day file:

```
~/Library/Application Support/ai.memfold.ai-usage-monitor/data/YYYY-MM-DD.jsonl
```

**Pass:** the file contains one line whose `reply` is the answer you just saw
(redacted), `prompt` is your message, `surface` is `web` (browser) or `app`
(native), `app` is a hash (never the app's real name), and `started_ms` /
`ended_ms` are real epoch times.

---

## 2b. Concurrent / background / other-desktop sessions

The monitor tracks every window independently. Verify the three parallel
scenarios:

1. **Two at once:** start a streaming answer in ChatGPT (browser) and, while it
   streams, start one in another AI app (or a second browser window). Then Show
   my data. **Pass:** two separate lines with different `app` hashes (or two
   lines from the same app, if you used two windows of one app).
2. **Backgrounded mid-stream:** start a long streaming answer, then click into
   a different app while it streams (the AI window is now in the background,
   possibly occluded). **Pass:** the session still captures the full answer
   (the background window is picked up by the full sweep, ~every 2 s).
3. **Another desktop/Space:** move the streaming AI window to another Space
   (Mission Control) while it streams. **Pass:** same as above — the day file
   contains the completed answer. Window enumeration is Space-independent
   (`onScreenWindowsOnly = false`) and window capture is desktop-independent.

Known limit to note: a *browser tab* that is not the window's visible tab
renders nothing, so it cannot be captured by any means — only visible surfaces
(in any window, on any Space) are observable.

---

## 3. Redaction audit — seeded secret + personal detail (safety gate)

Before sharing any data, prove redaction catches planted values.

1. Ask an AI a question whose **answer** will echo a **fake** AWS-shaped key and
   a **fake** personal detail (so they land in the captured reply), e.g. "repeat
   back: key AKIAIOSFODNN7EXAMPLE, email jane.doe@example.com, +1 415-555-0132".

2. Let it get captured (icon fills to a solid disc), then **Show my data**.

**Pass:** in today's day file, none of `AKIAIOSFODNN7EXAMPLE`,
`jane.doe@example.com`, or `415-555-0132` appear as raw text; each is a
`[REDACTED:…]` placeholder. If any raw value survives, **do not share the data**
— file the gap first.

(The deterministic layer is unit-tested for these exact shapes in
`src/redact.rs`; this step confirms it end-to-end through real capture.)

---

## 4. Non-AI activity must NOT be captured (false-positive gate)

The detector must not fire on text that merely grows. Test each:

- **You typing** a long message into any composer (email, Slack, a chat box).
- **A build log / test output** streaming in a terminal (`cargo build`, `npm
  install`, a CI tail).
- **Scrolling** a long article or code file.

**Pass:** in all three the icon stays a **ring-with-dot** (never fills to a
solid disc), and in the day file there is **no** entry for them. Typing is
excluded because the caret is in an input; logs are excluded because they read
as structured output, not prose.

If any of these produce a session, the detector thresholds need tuning — adjust
`detector` in `config.json` (see README) and re-run this step. This gate is the
one most likely to need a tuning pass on real machines; treat a failure here as
expected iteration, not a blocker.

---

## 5. Optional: NER redaction layer (feature `ner`)

Skip unless you enabled the NER layer — [docs/NER.md](docs/NER.md) owns the
setup. Once it is running, two checks belong here:

**Catches what regexes can't.** Repeat step 3 with a planted **person name**
(e.g. “Contact Maria Gonzalez”). **Pass:** in the day file the name is replaced
by `[REDACTED:NER:PERSON]`. (Note: as of 0.3.0 the NER layer is a library
capability that is not wired into the auto-flush; see docs/NER.md.)

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
- The reply is captured as a line-diff of the window before vs after, so OCR
  reflow may occasionally include a stray non-reply line; the prompt is
  best-effort (the composer text seen submitted).
- A browser AI window on **another desktop/Space** can't be screen-captured
  (macOS renders nothing off-Space); native apps work across desktops via
  Accessibility.
