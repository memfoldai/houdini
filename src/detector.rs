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
    /// can be a paste or a re-layout; sustained append across several samples
    /// is what a model streaming actually looks like.
    pub min_growth_steps: usize,
    /// A growth step must add at least this many characters. Filters caret
    /// blink / whitespace jitter that leaves length unchanged-ish.
    pub min_step_growth_chars: usize,
    /// A growth step must add at most this many characters. A whole page
    /// appearing at once (search results, a message arriving, a page load) is
    /// NOT autoregressive generation — it is one big jump, so it is excluded.
    pub max_step_growth_chars: usize,
    /// If the user was typing during a growth step, that step is the user
    /// writing, not a model generating — it cannot count toward streaming.
    /// (The caller sets `Sample::user_typing` from keystroke/focused-input
    /// signal; the detector treats it as authoritative.)
    pub exclude_typing_steps: bool,
    /// Minimum prose-vs-log score [0,1] of the *grown* text for a positive.
    /// Below this the streaming text reads as structured log output, not model
    /// prose — the build-log disambiguation.
    pub min_prose_score: f32,
    /// Rolling window of samples kept. Bounds memory; a session that keeps
    /// streaming re-confirms continuously within the window.
    pub window: usize,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        // Starting values, not magic: ~2-4 Hz sampling, so 3 steps ≈ ~1s of
        // sustained append; 2..=1200 chars/step spans word-by-word streaming
        // up to a chunky multi-line token burst while still excluding a
        // whole-screen instant paint.
        Self {
            min_growth_steps: 3,
            min_step_growth_chars: 2,
            max_step_growth_chars: 1200,
            exclude_typing_steps: true,
            min_prose_score: 0.55,
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
        if self.samples.len() < self.cfg.min_growth_steps + 1 {
            return Verdict::Idle;
        }
        // Find the longest recent run of qualifying append-growth steps.
        let mut run_steps = 0usize;
        let mut run_grown = 0usize;
        let mut best_steps = 0usize;
        let mut best_grown = 0usize;
        let samples: Vec<&Sample> = self.samples.iter().collect();
        for i in 1..samples.len() {
            let prev = samples[i - 1];
            let cur = samples[i];
            if let Some(added) = append_growth(&prev.text, &cur.text) {
                let typing_ok = !(self.cfg.exclude_typing_steps && cur.user_typing);
                let size_ok =
                    added >= self.cfg.min_step_growth_chars && added <= self.cfg.max_step_growth_chars;
                if typing_ok && size_ok {
                    run_steps += 1;
                    run_grown += added;
                    if run_steps > best_steps {
                        best_steps = run_steps;
                        best_grown = run_grown;
                    }
                    continue;
                }
            }
            run_steps = 0;
            run_grown = 0;
        }
        if best_steps < self.cfg.min_growth_steps {
            return Verdict::Idle;
        }
        // Form gate on the newest text (what actually streamed in).
        let newest = &self.samples.back().unwrap().text;
        let score = prose_score(newest);
        if score < self.cfg.min_prose_score {
            return Verdict::Idle;
        }
        Verdict::Streaming { grown_chars: best_grown, prose_score: score }
    }
}

/// If `cur` is `prev` with text APPENDED at the tail (prev is a prefix of cur,
/// ignoring pure trailing-whitespace churn), return the number of added
/// non-trivial chars. `None` when `cur` is not a tail-append of `prev` (a
/// replace, a shrink, an unrelated repaint) — those are not autoregressive
/// growth. Whitespace-tolerant so a trailing-space reflow between frames isn't
/// mistaken for a new token.
pub fn append_growth(prev: &str, cur: &str) -> Option<usize> {
    let p = prev.trim_end();
    if cur.len() < p.len() {
        return None;
    }
    if !cur.starts_with(p) {
        return None;
    }
    let added = cur[p.len()..].trim();
    if added.is_empty() {
        return Some(0);
    }
    Some(added.chars().count())
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

    #[test]
    fn append_growth_detects_tail_append() {
        assert_eq!(append_growth("Hello", "Hello world"), Some("world".chars().count()));
        // trailing-whitespace churn is not growth
        assert_eq!(append_growth("Hello", "Hello   "), Some(0));
        // replace / divergence is not append growth
        assert_eq!(append_growth("Hello world", "Goodbye world"), None);
        // shrink is not growth
        assert_eq!(append_growth("Hello world", "Hello"), None);
    }

    #[test]
    fn streams_prose_response_is_detected() {
        let mut det = StreamingDetector::new(DetectorConfig::default());
        // A model answer materializing token-by-token, user not typing.
        let v = feed(
            &mut det,
            &[
                (0, "", false),
                (300, "Sure. ", false),
                (600, "Sure. Regenerative agriculture ", false),
                (900, "Sure. Regenerative agriculture is a set of ", false),
                (1200, "Sure. Regenerative agriculture is a set of farming practices that ", false),
                (1500, "Sure. Regenerative agriculture is a set of farming practices that restore soil health.", false),
            ],
        );
        assert!(matches!(v, Verdict::Streaming { .. }), "expected streaming, got {v:?}");
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
