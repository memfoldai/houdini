//! Optional GLiNER-PII NER redaction layer (feature `ner`).
//!
//! This is the SECOND, optional layer described in `redact`: the deterministic
//! layer (always on) removes high-confidence secret/structured-PII shapes; this
//! layer catches free-form personal identifiers a regex cannot — a person's
//! name, a street address, a date of birth — using a GLiNER-PII ONNX model run
//! fully offline via `gliner`/`ort`. It runs on top of the deterministic
//! output, so it never re-sees an already-redacted value.
//!
//! The model is NEVER bundled: the operator provisions it (tokenizer.json +
//! model.onnx) and points the app at the directory — see docs/NER.md. On load
//! the redactor runs a SEEDED SELF-TEST: it must detect a planted name in a
//! known sentence, or `load` fails. A model that silently detects nothing (a
//! wrong export, an architecture/mode mismatch) is therefore never trusted to
//! stand in for redaction — the layer fails closed.
//!
//! Label choice is deliberate. The study keeps research CONTENT (topics, orgs,
//! places someone researched) and removes only PERSONAL identifiers, so the
//! default labels are personal-identifier classes — not generic
//! `organization`/`location`, which would shred the very signal the study wants.

use std::path::Path;

use gliner::model::input::text::TextInput;
use gliner::model::params::Parameters;
use gliner::model::pipeline::token::TokenMode;
use gliner::model::GLiNER;
use orp::params::RuntimeParameters;

use crate::redact::{self, RedactionKind};

/// Default PII labels: personal identifiers only. Research subject matter
/// (companies, places, technologies someone looked up) is intentionally NOT
/// here — redacting it would destroy the study's signal.
pub const DEFAULT_PII_LABELS: &[&str] = &[
    "person",
    "email",
    "phone number",
    "home address",
    "date of birth",
    "credit card number",
    "social security number",
    "passport number",
    "driver license number",
    "bank account number",
    "ip address",
];

/// Minimum model confidence for a span to be redacted. Below this the model is
/// too unsure to justify mutating text; the human review gate is the backstop.
const DEFAULT_MIN_PROBABILITY: f32 = 0.5;
/// Ignore ultra-short spans — a one/two-char "name" is almost always a false
/// positive, and blindly replacing it would maul ordinary prose.
const MIN_SPAN_CHARS: usize = 3;

/// Combined redaction result: deterministic layer + NER layer, with a
/// share-safe audit of both (kinds/classes and counts, never raw values).
#[derive(Debug, Clone, Default)]
pub struct NerRedaction {
    pub text: String,
    /// Deterministic-layer counts (typed secret/PII kinds).
    pub deterministic: Vec<(RedactionKind, usize)>,
    /// NER-layer counts, keyed by the model's entity class (e.g. "person").
    pub ner: Vec<(String, usize)>,
}

impl NerRedaction {
    pub fn total(&self) -> usize {
        let d: usize = self.deterministic.iter().map(|(_, n)| n).sum();
        let n: usize = self.ner.iter().map(|(_, n)| n).sum();
        d + n
    }
}

/// A loaded, self-tested GLiNER-PII redactor.
pub struct NerRedactor {
    model: GLiNER<TokenMode>,
    labels: Vec<String>,
    min_probability: f32,
}

impl NerRedactor {
    /// Load a token-mode GLiNER model from a directory containing
    /// `tokenizer.json` and `model.onnx`, using the default PII labels, then
    /// run the seeded self-test. Returns an error if the files are missing, the
    /// model fails to load, or the self-test does not detect the planted name.
    pub fn load(model_dir: &Path) -> Result<Self, String> {
        Self::load_with_labels(model_dir, DEFAULT_PII_LABELS)
    }

    pub fn load_with_labels(model_dir: &Path, labels: &[&str]) -> Result<Self, String> {
        let tokenizer = model_dir.join("tokenizer.json");
        let model_path = model_dir.join("model.onnx");
        if !tokenizer.exists() || !model_path.exists() {
            return Err(format!(
                "NER model not provisioned: expected tokenizer.json and model.onnx under {} (see docs/NER.md)",
                model_dir.display()
            ));
        }

        let model = GLiNER::<TokenMode>::new(
            Parameters::default(),
            RuntimeParameters::default(),
            tokenizer,
            model_path,
        )
        .map_err(|e| format!("failed to load GLiNER model: {e}"))?;

        let redactor = Self {
            model,
            labels: labels.iter().map(|s| s.to_string()).collect(),
            min_probability: DEFAULT_MIN_PROBABILITY,
        };
        redactor.self_test()?;
        Ok(redactor)
    }

    /// Seeded self-test: a planted person name in a fixed sentence must be
    /// detected, or the model is not trusted (fail closed). This is the gate
    /// that stops a wrong/mismatched model from silently redacting nothing.
    fn self_test(&self) -> Result<(), String> {
        let probe = "My name is Jonathan Aldenberg and I work here.";
        let hits = self.detect(probe).map_err(|e| format!("NER self-test errored: {e}"))?;
        if hits.is_empty() {
            return Err(
                "NER self-test failed: model detected no PII in the probe sentence — refusing to \
                 trust it (check the model is a token-mode GLiNER-PII export; see docs/NER.md)"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// Full redaction: deterministic layer first, then the NER layer over what
    /// remains. This is the function the export/pre-share sweep calls.
    pub fn redact(&self, input: &str) -> Result<NerRedaction, String> {
        let base = redact::redact_deterministic(input);
        let hits = self.detect(&base.text).map_err(|e| format!("NER inference failed: {e}"))?;

        // Replace longest spans first so a longer name isn't half-consumed by a
        // shorter overlapping one.
        let mut spans: Vec<DetectedSpan> = hits;
        spans.sort_by(|a, b| b.text.chars().count().cmp(&a.text.chars().count()));

        let mut text = base.text;
        let mut ner_counts: Vec<(String, usize)> = Vec::new();
        for span in spans {
            let needle = span.text.trim();
            if needle.chars().count() < MIN_SPAN_CHARS {
                continue;
            }
            // Already gone (consumed by a longer overlapping span or the
            // deterministic layer): nothing to do.
            let occurrences = text.matches(needle).count();
            if occurrences == 0 {
                continue;
            }
            let placeholder = format!("[REDACTED:NER:{}]", class_tag(&span.class));
            text = text.replace(needle, &placeholder);
            bump(&mut ner_counts, &span.class, occurrences);
        }

        Ok(NerRedaction { text, deterministic: base.counts, ner: ner_counts })
    }

    /// Run the model over one text and return its accepted spans (above the
    /// probability threshold).
    fn detect(&self, text: &str) -> Result<Vec<DetectedSpan>, String> {
        let labels: Vec<&str> = self.labels.iter().map(String::as_str).collect();
        let input = TextInput::from_str(&[text], &labels)
            .map_err(|e| format!("failed to build model input: {e}"))?;
        let output = self.model.inference(input).map_err(|e| format!("{e}"))?;

        let mut out = Vec::new();
        // One input text → one span list.
        if let Some(spans) = output.spans.into_iter().next() {
            for span in spans {
                if span.probability() >= self.min_probability {
                    out.push(DetectedSpan {
                        text: span.text().to_string(),
                        class: span.class().to_string(),
                    });
                }
            }
        }
        Ok(out)
    }
}

/// A model-detected entity reduced to what redaction needs.
struct DetectedSpan {
    text: String,
    class: String,
}

/// Normalize an entity class into an uppercase placeholder tag
/// ("phone number" → "PHONE_NUMBER").
fn class_tag(class: &str) -> String {
    class
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect()
}

fn bump(counts: &mut Vec<(String, usize)>, class: &str, n: usize) {
    if let Some(entry) = counts.iter_mut().find(|(c, _)| c == class) {
        entry.1 += n;
    } else {
        counts.push((class.to_string(), n));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_tag_normalizes() {
        assert_eq!(class_tag("phone number"), "PHONE_NUMBER");
        assert_eq!(class_tag("person"), "PERSON");
        assert_eq!(class_tag("date of birth"), "DATE_OF_BIRTH");
    }

    // Real model round-trip. Ignored by default: it needs a provisioned
    // token-mode GLiNER-PII model. Run with the directory in AUM_NER_MODEL_DIR:
    //   AUM_NER_MODEL_DIR=/path/to/model cargo test --features ner ner_ -- --ignored
    #[test]
    #[ignore]
    fn ner_layer_redacts_a_planted_name_over_deterministic() {
        let dir = std::env::var("AUM_NER_MODEL_DIR").expect("set AUM_NER_MODEL_DIR");
        let redactor = NerRedactor::load(Path::new(&dir)).expect("load + self-test");
        let out = redactor
            .redact("Contact Maria Gonzalez about the vendor at ops@acme.com.")
            .expect("redact");
        // Deterministic layer took the email; NER took the person name.
        assert!(!out.text.contains("ops@acme.com"));
        assert!(!out.text.contains("Maria Gonzalez"));
        assert!(out.ner.iter().any(|(_, n)| *n > 0));
    }
}
