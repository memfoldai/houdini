//! Streaming AI-inference detector — the "world model".
//!
//! This module never judges what text *means*. Intent is invisible: "best
//! domain extensions in 2026" is identical typed into ChatGPT, WhatsApp, or
//! Google. Instead it detects the physical, app-agnostic signature of a
//! language model generating text on screen: **autoregressive streaming** —
//! text appended token-by-token at the tail of an output region, over
//! seconds, that the user is not typing.
//!
//! Input is a time-ordered series of [`Sample`]s (a snapshot of the candidate
//! output region's text plus whether the user was typing at that instant),
//! taken a few times per second while armed. The detector reports when that
//! series carries the generation signature.
//!
//! The one honest false positive is other streaming text (a build log,
//! `tail -f`). It is ruled out by FORM, not intent: model output is
//! natural-language prose/markdown; a build log is structured, repetitive,
//! path/punctuation-heavy. `prose_score` measures that, and it never inspects
//! the user's input or its meaning.

use std::collections::VecDeque;

/// Tuning thresholds. These are starting values, not settled ones: there is no
/// published prior art for streaming-signature detection to copy, so they are
/// tuned against real machines via VERIFICATION.md step 4. Every field has a why.
#[derive(Debug, Clone)]
pub struct DetectorConfig {
    /// Consecutive growth steps required before declaring streaming. One step
    /// can be a paste or a re-layout; sustained growth across several samples
    /// is what a model streaming actually looks like.
    pub min_growth_steps: usize,
    /// A growth step must add at least this many PROSE characters (letters in
    /// natural-language lines; see `prose_len`). High enough to beat OCR/AX
    /// jitter on a big window, low enough that word-by-word streaming clears it.
    pub min_step_growth_chars: usize,
    /// A jump larger than this in one step is a whole page/app painting at once
    /// (a load, a scene change), not autoregressive generation — so it resets
    /// the run rather than counting.
    pub max_step_growth_chars: usize,
    /// If the user was typing during a growth step, that step is the user
    /// writing, not a model generating — it cannot count toward streaming.
    /// (The caller sets `Sample::user_typing` from the focused-input-changed
    /// signal; the detector treats it as authoritative.)
    pub exclude_typing_steps: bool,
    /// Rolling window of samples kept. Bounds memory; a session that keeps
    /// streaming re-confirms continuously within the window.
    pub window: usize,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        // Starting values, not magic. Sampling is ~1-2 Hz per surface, so 3
        // steps ≈ a few seconds of sustained prose growth. The min beats capture
        // jitter on a cluttered window; the max excludes a whole-screen paint.
        Self {
            min_growth_steps: 3,
            min_step_growth_chars: 12,
            max_step_growth_chars: 2500,
            exclude_typing_steps: true,
            window: 24,
        }
    }
}

/// One observation of the candidate output region at an instant.
#[derive(Debug, Clone)]
pub struct Sample {
    /// Monotonic timestamp in milliseconds (only ordering + spacing matter).
    pub ts_ms: u64,
    /// The output region's full visible text at this instant.
    pub text: String,
    /// Whether the user was actively typing (a focused text input receiving
    /// keystrokes) at this instant. Authoritative — set by the capture layer.
    pub user_typing: bool,
}

/// Verdict for the current window.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// No generation signature in the current window.
    Idle,
    /// Autoregressive prose streaming is happening now. `grown_chars` is how
    /// much text the confirmed run appended; `prose_score` is its form score.
    Streaming { grown_chars: usize, prose_score: f32 },
}

/// Rolling detector fed one [`Sample`] at a time.
pub struct StreamingDetector {
    cfg: DetectorConfig,
    samples: VecDeque<Sample>,
}

impl StreamingDetector {
    pub fn new(cfg: DetectorConfig) -> Self {
        Self { cfg, samples: VecDeque::new() }
    }

    /// Feed a sample and get the verdict over the current window.
    pub fn push(&mut self, s: Sample) -> Verdict {
        self.samples.push_back(s);
        while self.samples.len() > self.cfg.window {
            self.samples.pop_front();
        }
        self.evaluate()
    }

    /// Drop all history (call on window/app switch — a new surface is a new
    /// candidate, and stale text must not be compared across the boundary).
    pub fn reset(&mut self) {
        self.samples.clear();
    }

    fn evaluate(&self) -> Verdict {
        let lens: Vec<i64> = self.samples.iter().map(|s| prose_len(&s.text) as i64).collect();
        let typing: Vec<bool> = self.samples.iter().map(|s| s.user_typing).collect();
        match growth_run(&lens, &typing, &self.cfg) {
            Some(grown) => {
                let score = prose_score(&self.samples.back().unwrap().text);
                Verdict::Streaming { grown_chars: grown, prose_score: score }
            }
            None => Verdict::Idle,
        }
    }
}

/// How many consecutive flat/jittery frames a growth run tolerates before it is
/// considered stalled and reset. Streaming makes a new prose high almost every
/// sample, so a short OCR hiccup is fine; a genuine stop (or the gaps between
/// discrete message arrivals) exceeds this and resets — which is what keeps a
/// busy chat's incoming messages from reading as one continuous generation.
const STALL_TOLERANCE: usize = 2;

/// The core signal: a SUSTAINED run of new prose highs.
///
/// `lens[i]` is `prose_len` at sample `i` (letters in natural-language lines —
/// fixed window chrome cancels, only the streamed reply moves it). A step
/// counts when the prose amount reaches a NEW HIGH by at least
/// `min_step_growth_chars` (measuring against the running max, not the previous
/// frame, so an OCR dip-then-recover is not mistaken for a shrink). A jump
/// bigger than `max_step_growth_chars`, or a large drop (below 60% of the peak),
/// is a page/scene change and resets the run. `STALL_TOLERANCE` flat frames also
/// reset it. Returns the grown prose chars if ≥ `min_growth_steps` were reached.
fn growth_run(lens: &[i64], typing: &[bool], cfg: &DetectorConfig) -> Option<usize> {
    if lens.len() < cfg.min_growth_steps + 1 {
        return None;
    }
    let min = cfg.min_step_growth_chars as i64;
    let max = cfg.max_step_growth_chars as i64;

    let mut steps = 0usize;
    let mut grown = 0i64;
    let mut best = 0usize;
    let mut best_grown = 0i64;
    let mut peak = lens[0];
    let mut stall = 0usize;

    for i in 1..lens.len() {
        if cfg.exclude_typing_steps && typing[i] {
            // The user typing (their own input changing) can't be generation.
            steps = 0;
            grown = 0;
            stall = 0;
            peak = peak.max(lens[i]);
            continue;
        }
        let rise = lens[i] - peak;
        if rise >= min && rise <= max {
            steps += 1;
            grown += rise;
            peak = lens[i];
            stall = 0;
            if steps > best {
                best = steps;
                best_grown = grown;
            }
        } else if rise > max || lens[i] * 5 < peak * 3 {
            // Whole-screen paint (page/app load) or a big drop (scroll/scene
            // change): not autoregressive growth — reset to the new content.
            steps = 0;
            grown = 0;
            stall = 0;
            peak = lens[i];
        } else {
            // Flat frame or jitter dip: hold the peak, tolerate a few in a row.
            peak = peak.max(lens[i]);
            stall += 1;
            if stall > STALL_TOLERANCE {
                steps = 0;
                grown = 0;
            }
        }
    }
    (best >= cfg.min_growth_steps).then_some(best_grown as usize)
}

/// Fraction of `prev` that must survive as a common prefix of `cur` for the
/// The amount of natural-language PROSE on screen: the count of alphabetic
/// characters in lines that are NOT structured (log/code/chrome-symbol lines,
/// per `looks_structured`). This is the quantity the detector watches grow.
///
/// Why letters-in-prose-lines and not total length: a real app window is mostly
/// fixed chrome (sidebar entries, button labels, timestamps, model name). That
/// chrome is either structured (excluded) or CONSTANT, so it cancels when we
/// difference consecutive samples — only the streamed reply changes the count.
/// Counting only letters also shrugs off OCR punctuation/whitespace jitter.
pub fn prose_len(text: &str) -> usize {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !looks_structured(l))
        .map(|l| l.chars().filter(|c| c.is_alphabetic()).count())
        .sum()
}

/// Form score in [0,1]: how much the text reads as natural-language prose /
/// markdown (high) vs structured log/code output (low). This is the
/// build-log disambiguation and is deliberately about SHAPE, never meaning —
/// it does not decide "is this for an AI", only "is this prose".
///
/// Signals (each a soft vote):
///  - share of alphabetic characters (prose is letter-dense; logs are
///    punctuation/number/path-dense)
///  - average word length in a human band (logs have long tokens: paths,
///    hashes, identifiers)
///  - low share of lines that look like log/structured records (timestamps,
///    `key=value`, leading `[`/`{`, deep `/paths`, all-caps level tags)
///  - presence of sentence punctuation
pub fn prose_score(text: &str) -> f32 {
    let t = text.trim();
    if t.chars().count() < 12 {
        // Too little to judge; don't claim prose. Streaming re-confirms as
        // more arrives.
        return 0.0;
    }
    let total = t.chars().count() as f32;
    let alpha = t.chars().filter(|c| c.is_alphabetic()).count() as f32;
    let spaces = t.chars().filter(|c| *c == ' ').count() as f32;
    let alpha_share = alpha / total;

    // Average token length.
    let words: Vec<&str> = t.split_whitespace().collect();
    let avg_word = if words.is_empty() {
        99.0
    } else {
        words.iter().map(|w| w.chars().count()).sum::<usize>() as f32 / words.len() as f32
    };

    // Structured-line share.
    let lines: Vec<&str> = t.lines().filter(|l| !l.trim().is_empty()).collect();
    let structured = lines.iter().filter(|l| looks_structured(l)).count() as f32;
    let structured_share = if lines.is_empty() { 1.0 } else { structured / lines.len() as f32 };

    let _ = spaces; // spacing is a weak, noisy signal — deliberately unused.
    let has_sentence_punct = t.contains(". ")
        || t.contains(", ")
        || t.contains("? ")
        || t.contains("! ")
        || t.ends_with('.')
        || t.ends_with('?')
        || t.ends_with('!');

    // Combine. Structured-line share is the dominant separator between prose
    // and log/code output; alpha density and sentence structure corroborate;
    // the word-band is a small nudge (a tight human band, since log tokens —
    // paths, hashes, key=value — run long).
    let mut score = 0.0f32;
    score += 0.55 * (1.0 - structured_share);
    score += 0.25 * clamp01((alpha_share - 0.35) / 0.35); // ~0.35 low, ~0.70 high
    score += if has_sentence_punct { 0.12 } else { 0.0 };
    score += 0.08 * band(avg_word, 3.0, 6.5); // human words ~3-6.5 chars avg
    clamp01(score)
}

/// True when a single line reads like a structured/log/code record rather than
/// a prose sentence. Conservative — only clear structural markers count.
fn looks_structured(line: &str) -> bool {
    let l = line.trim();
    if l.is_empty() {
        return false;
    }
    // Leading structural bracket / bullet-of-symbols / shell prompt.
    let first = l.chars().next().unwrap();
    if matches!(first, '[' | '{' | '<' | '$' | '#' | '>' | '|') {
        return true;
    }
    // key=value with no sentence spaces (config/log line).
    if l.contains('=') && !l.contains(", ") && !l.contains(". ") && l.split_whitespace().count() <= 4 {
        return true;
    }
    // Deep path or URL-ish token (standalone).
    if l.matches('/').count() >= 3 && !l.contains(' ') {
        return true;
    }
    // HTTP access-log line: a method verb followed by a /path token.
    let first_word = l.split_whitespace().next().unwrap_or("");
    if matches!(first_word, "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD" | "OPTIONS")
        && l.split_whitespace().any(|w| w.starts_with('/'))
    {
        return true;
    }
    // A line carrying a /path token AND a bare numeric token reads as a log
    // record (status/size/duration), not a prose sentence.
    let has_path = l.split_whitespace().any(|w| w.contains('/') && !w.contains(' '));
    let has_bare_number = l.split_whitespace().any(|w| {
        let core = w.trim_end_matches(|c: char| !c.is_ascii_digit());
        !core.is_empty() && core.chars().all(|c| c.is_ascii_digit())
    });
    if has_path && has_bare_number && !l.contains(", ") && !l.contains(". ") {
        return true;
    }
    // Timestamp-ish or level tag at the start.
    let head: String = l.chars().take(24).collect();
    if head.contains("::") || head.contains(":") && head.chars().filter(|c| c.is_ascii_digit()).count() >= 4 {
        // e.g. "12:04:37.221" or "2026-07-16T..."
        return true;
    }
    let upper_head: String = l.split_whitespace().next().unwrap_or("").to_string();
    if matches!(upper_head.as_str(), "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE" | "FATAL") {
        return true;
    }
    false
}

fn clamp01(x: f32) -> f32 {
    x.max(0.0).min(1.0)
}

/// 1.0 inside [lo,hi], tapering to 0 outside. Soft membership for a band.
fn band(x: f32, lo: f32, hi: f32) -> f32 {
    if x >= lo && x <= hi {
        1.0
    } else if x < lo {
        clamp01(1.0 - (lo - x) / lo)
    } else {
        clamp01(1.0 - (x - hi) / hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(det: &mut StreamingDetector, frames: &[(u64, &str, bool)]) -> Verdict {
        let mut last = Verdict::Idle;
        for (ts, text, typing) in frames {
            last = det.push(Sample { ts_ms: *ts, text: text.to_string(), user_typing: *typing });
        }
        last
    }

    /// A model reply materializing inside a real app window: fixed chrome
    /// (sidebar, header, disclaimer, composer) with the assistant message
    /// growing. The whole-window prose score is LOW (chrome dominates), so the
    /// old score gate rejected it — this is the exact real-app failure.
    fn chat_window(reply: &str) -> String {
        format!(
            "New chat\nRegenerative soil\nTrip to Kyoto\nRust lifetimes\n\
             Assistant\n\
             You\nExplain how photosynthesis works in simple terms\n\
             Assistant\n{reply}\n\
             This assistant can make mistakes. Check important information.\n\
             Message the assistant"
        )
    }

    #[test]
    fn streaming_reply_in_a_cluttered_window_is_detected() {
        let mut det = StreamingDetector::new(DetectorConfig::default());
        let frames: Vec<String> = [
            "",
            "Photosynthesis is how plants make food",
            "Photosynthesis is how plants make food from sunlight, using a green pigment",
            "Photosynthesis is how plants make food from sunlight, using a green pigment called chlorophyll to absorb light",
            "Photosynthesis is how plants make food from sunlight, using a green pigment called chlorophyll to absorb light and build sugars from air and water",
        ]
        .iter()
        .map(|r| chat_window(r))
        .collect();
        let mut t = 0u64;
        let mut last = Verdict::Idle;
        for f in &frames {
            last = det.push(Sample { ts_ms: t, text: f.clone(), user_typing: false });
            t += 700;
        }
        assert!(matches!(last, Verdict::Streaming { .. }), "cluttered window must detect, got {last:?}");
    }

    #[test]
    fn scrolling_a_static_page_is_not_streaming() {
        // Scrolling replaces visible content (some leaves the top as new arrives),
        // so net prose does not steadily grow → not streaming.
        let mut det = StreamingDetector::new(DetectorConfig::default());
        let a = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
        let b = "nu xi omicron pi rho sigma tau upsilon phi chi psi omega again more";
        let v = feed(&mut det, &[(0, a, false), (700, b, false), (1400, a, false), (2100, b, false)]);
        assert_eq!(v, Verdict::Idle, "scrolling is content replacement, not growth");
    }

    // The growth-run core, tested directly on prose_len trajectories — this is
    // the real-world behavior (OCR jitter, message arrivals) I could not
    // reliably reproduce by driving a GUI, so it is pinned here instead.
    fn run(lens: &[i64]) -> Option<usize> {
        let typing = vec![false; lens.len()];
        growth_run(lens, &typing, &DetectorConfig::default())
    }

    #[test]
    fn growth_run_detects_streaming_even_through_ocr_jitter() {
        // Steady climb → streaming.
        assert!(run(&[100, 130, 160, 190, 220]).is_some());
        // Climb with OCR dips/wobble between frames → still streaming (peak-based,
        // so a dip-then-recover is not read as a shrink).
        assert!(run(&[100, 130, 125, 160, 155, 190, 188, 220]).is_some());
    }

    #[test]
    fn growth_run_ignores_static_and_single_arrivals() {
        // Static window with OCR wobble → not streaming.
        assert_eq!(run(&[200, 203, 198, 201, 199, 202, 200]), None);
        // One message/answer appearing at once, then flat → a single jump, not a
        // sustained generation.
        assert_eq!(run(&[100, 100, 160, 160, 160, 160, 160]), None);
        // A busy chat: discrete messages arriving with idle gaps between them
        // must NOT accumulate into one "generation" (the stall between resets).
        assert_eq!(run(&[100, 150, 150, 150, 150, 205, 205, 205, 205, 260]), None);
    }

    #[test]
    fn growth_run_resets_on_scene_change() {
        // A big drop (navigating away / clearing) resets; a couple of later
        // frames aren't enough to re-confirm.
        assert_eq!(run(&[300, 330, 360, 40, 60]), None);
    }

    #[test]
    fn instant_full_paint_is_not_streaming() {
        // Search results / a message arriving: one big jump, not token append.
        let mut det = StreamingDetector::new(DetectorConfig::default());
        let big = "A".repeat(4000);
        let v = feed(
            &mut det,
            &[(0, "", false), (300, &big, false), (600, &big, false), (900, &big, false)],
        );
        assert_eq!(v, Verdict::Idle);
    }

    #[test]
    fn user_typing_growth_is_excluded() {
        // The user composing a long message: growth, but user_typing=true.
        let mut det = StreamingDetector::new(DetectorConfig::default());
        let v = feed(
            &mut det,
            &[
                (0, "", true),
                (300, "best domain ", true),
                (600, "best domain extensions ", true),
                (900, "best domain extensions in 2026", true),
            ],
        );
        assert_eq!(v, Verdict::Idle, "user's own typing must not read as AI generation");
    }

    #[test]
    fn streaming_build_log_is_disambiguated_by_form() {
        // A build log streams too (no typing), but its FORM is structured.
        let mut det = StreamingDetector::new(DetectorConfig::default());
        let v = feed(
            &mut det,
            &[
                (0, "", false),
                (300, "[build] compiling src/main.rs\n", false),
                (600, "[build] compiling src/main.rs\n[build] compiling src/lib.rs\n", false),
                (900, "[build] compiling src/main.rs\n[build] compiling src/lib.rs\n[warn] unused import at src/x.rs:12\n", false),
                (1200, "[build] compiling src/main.rs\n[build] compiling src/lib.rs\n[warn] unused import at src/x.rs:12\n[build] linking target/debug/app\n", false),
            ],
        );
        assert_eq!(v, Verdict::Idle, "structured log form must not read as AI prose");
    }

    #[test]
    fn prose_scores_higher_than_logs() {
        let prose = "Regenerative agriculture is a set of farming practices that aim to restore soil health, increase biodiversity, and capture carbon in the ground.";
        let log = "[build] compiling src/main.rs\n[warn] unused import at src/x.rs:12\nGET /api/v1/users 200 4ms\nkey=value host=127.0.0.1 port=8080";
        assert!(prose_score(prose) > 0.6, "prose={}", prose_score(prose));
        assert!(prose_score(log) < 0.45, "log={}", prose_score(log));
    }

    #[test]
    fn reset_clears_cross_surface_history() {
        let mut det = StreamingDetector::new(DetectorConfig::default());
        feed(&mut det, &[(0, "a", false), (300, "ab", false)]);
        det.reset();
        // After reset, a single new sample can't retro-confirm streaming.
        let v = det.push(Sample { ts_ms: 600, text: "abc".into(), user_typing: false });
        assert_eq!(v, Verdict::Idle);
    }
}
