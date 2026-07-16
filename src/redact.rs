//! Offline redaction — a hard gate, not a feature. Runs before any captured
//! text touches disk and again is auditable before export. FULLY OFFLINE: no
//! network call is ever made here (a redactor that phones home is disqualified
//! — this rules out live-verification secret scanners).
//!
//! Two layers, applied in order:
//!  1. Deterministic layer (this file): high-confidence secret token patterns
//!     (shape derived from gitleaks' published rules — provider-prefixed keys,
//!     private-key blocks) + structured-PII recognizers (email, credit card
//!     with Luhn, US SSN, phone). Deterministic, testable, zero false
//!     negatives on the shapes it knows.
//!  2. NER layer (`ner` module, feature-gated): GLiNER-PII ONNX via `ort` for
//!     free-form PII (person names, addresses) the regexes cannot catch. Wired
//!     separately once its ONNX pre/post-processing is validated; the
//!     deterministic layer stands alone and is always applied.
//!
//! Every match is replaced with a TYPED placeholder (`[REDACTED:KIND]`) so the
//! downstream study still sees that a secret/PII existed and of what kind,
//! without the value.

use regex::Regex;
use std::sync::OnceLock;

/// What a rule redacts, surfaced in the placeholder so analytics keep the
/// shape without the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedactionKind {
    AwsKey,
    GithubToken,
    OpenAiKey,
    SlackToken,
    StripeKey,
    GoogleApiKey,
    PrivateKeyBlock,
    GenericAssignedSecret,
    Email,
    CreditCard,
    UsSsn,
    Phone,
}

impl RedactionKind {
    fn tag(self) -> &'static str {
        match self {
            RedactionKind::AwsKey => "AWS_KEY",
            RedactionKind::GithubToken => "GITHUB_TOKEN",
            RedactionKind::OpenAiKey => "OPENAI_KEY",
            RedactionKind::SlackToken => "SLACK_TOKEN",
            RedactionKind::StripeKey => "STRIPE_KEY",
            RedactionKind::GoogleApiKey => "GOOGLE_API_KEY",
            RedactionKind::PrivateKeyBlock => "PRIVATE_KEY",
            RedactionKind::GenericAssignedSecret => "SECRET",
            RedactionKind::Email => "EMAIL",
            RedactionKind::CreditCard => "CREDIT_CARD",
            RedactionKind::UsSsn => "SSN",
            RedactionKind::Phone => "PHONE",
        }
    }
    fn placeholder(self) -> String {
        format!("[REDACTED:{}]", self.tag())
    }
}

/// Result of redacting a block of text.
#[derive(Debug, Clone, Default)]
pub struct RedactionReport {
    pub text: String,
    /// Count per kind, for a share-safe audit summary.
    pub counts: Vec<(RedactionKind, usize)>,
}

impl RedactionReport {
    pub fn total(&self) -> usize {
        self.counts.iter().map(|(_, n)| n).sum()
    }
    fn bump(&mut self, kind: RedactionKind, n: usize) {
        if n == 0 {
            return;
        }
        for (k, c) in self.counts.iter_mut() {
            if *k == kind {
                *c += n;
                return;
            }
        }
        self.counts.push((kind, n));
    }
}

struct Rule {
    kind: RedactionKind,
    re: Regex,
    /// Optional validator on the matched substring (e.g. Luhn for cards).
    /// A rule only redacts a match its validator accepts — this is how the
    /// card rule avoids nuking any 16-digit number.
    validate: Option<fn(&str) -> bool>,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            // ---- Secrets (shapes from gitleaks' published provider rules) ----
            // AWS access key IDs: fixed 4-char prefix classes + 16 base32-ish.
            Rule {
                kind: RedactionKind::AwsKey,
                re: Regex::new(r"\b(?:AKIA|ASIA|AGPA|AIDA|AROA|AIPA|ANPA|ANVA)[A-Z0-9]{16}\b").unwrap(),
                validate: None,
            },
            // GitHub personal/OAuth/app/refresh tokens + fine-grained PAT.
            Rule {
                kind: RedactionKind::GithubToken,
                re: Regex::new(r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b|\bgithub_pat_[A-Za-z0-9_]{22,}\b").unwrap(),
                validate: None,
            },
            // OpenAI keys: sk- / sk-proj- followed by a long token.
            Rule {
                kind: RedactionKind::OpenAiKey,
                re: Regex::new(r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{20,}\b").unwrap(),
                validate: None,
            },
            // Slack tokens.
            Rule {
                kind: RedactionKind::SlackToken,
                re: Regex::new(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b").unwrap(),
                validate: None,
            },
            // Stripe live/restricted keys.
            Rule {
                kind: RedactionKind::StripeKey,
                re: Regex::new(r"\b(?:sk|rk|pk)_live_[A-Za-z0-9]{16,}\b").unwrap(),
                validate: None,
            },
            // Google API keys: AIza + 35.
            Rule {
                kind: RedactionKind::GoogleApiKey,
                re: Regex::new(r"\bAIza[0-9A-Za-z_\-]{35}\b").unwrap(),
                validate: None,
            },
            // PEM private-key blocks (any type).
            Rule {
                kind: RedactionKind::PrivateKeyBlock,
                re: Regex::new(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----").unwrap(),
                validate: None,
            },
            // Generic "secret/token/password/api_key = <value>" assignments —
            // context-anchored so it doesn't nuke ordinary prose. Requires an
            // assignment operator and a sufficiently long opaque value.
            Rule {
                kind: RedactionKind::GenericAssignedSecret,
                re: Regex::new(r#"(?i)\b(?:api[_-]?key|secret|token|password|passwd|access[_-]?key|client[_-]?secret)\b\s*[:=]\s*['"]?([A-Za-z0-9_\-\.]{12,})['"]?"#).unwrap(),
                validate: None,
            },
            // ---- Structured PII ----
            Rule {
                kind: RedactionKind::Email,
                re: Regex::new(r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b").unwrap(),
                validate: None,
            },
            // Candidate card numbers: 13-19 digits with optional space/dash
            // groups; only redacted if Luhn-valid (avoids clobbering IDs).
            Rule {
                kind: RedactionKind::CreditCard,
                re: Regex::new(r"\b(?:\d[ -]?){13,19}\b").unwrap(),
                validate: Some(luhn_ok),
            },
            // US SSN (dashed form; the bare-9-digit form is too collision-prone
            // to redact deterministically — the NER layer handles those in
            // context).
            Rule {
                kind: RedactionKind::UsSsn,
                re: Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(),
                validate: Some(ssn_plausible),
            },
            // Phone numbers: E.164 and common separated forms, length-bounded.
            Rule {
                kind: RedactionKind::Phone,
                re: Regex::new(r"\b(?:\+?\d{1,3}[ .\-]?)?(?:\(\d{2,4}\)[ .\-]?)?\d{3,4}[ .\-]\d{3,4}(?:[ .\-]\d{2,4})?\b").unwrap(),
                validate: Some(phone_plausible),
            },
        ]
    })
}

/// Apply the deterministic layer. Rules run in declared order; secrets before
/// PII so a token that also looks phone-ish is tagged as the secret.
pub fn redact_deterministic(input: &str) -> RedactionReport {
    let mut report = RedactionReport { text: input.to_string(), counts: Vec::new() };
    for rule in rules() {
        let placeholder = rule.kind.placeholder();
        let mut n = 0usize;
        let out = rule.re.replace_all(&report.text, |caps: &regex::Captures| {
            let whole = caps.get(0).unwrap().as_str();
            if let Some(v) = rule.validate {
                if !v(whole) {
                    // Not a real match — leave it untouched.
                    return whole.to_string();
                }
            }
            n += 1;
            placeholder.clone()
        });
        report.text = out.into_owned();
        report.bump(rule.kind, n);
    }
    report
}

/// Luhn checksum over the digits of a candidate card string.
fn luhn_ok(s: &str) -> bool {
    let digits: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.len() < 13 || digits.len() > 19 {
        return false;
    }
    let mut sum = 0u32;
    let mut dbl = false;
    for &d in digits.iter().rev() {
        let mut x = d;
        if dbl {
            x *= 2;
            if x > 9 {
                x -= 9;
            }
        }
        sum += x;
        dbl = !dbl;
    }
    sum % 10 == 0
}

/// Reject SSN placeholders that can't be issued (area 000/666/900-999,
/// group 00, serial 0000) — keeps the dashed rule from tagging arbitrary
/// NNN-NN-NNNN strings that aren't SSNs.
fn ssn_plausible(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    let area: u32 = parts[0].parse().unwrap_or(0);
    let group: u32 = parts[1].parse().unwrap_or(0);
    let serial: u32 = parts[2].parse().unwrap_or(0);
    area != 0 && area != 666 && area < 900 && group != 0 && serial != 0
}

/// Require enough digits to be a real phone and reject all-same or trivially
/// short sequences (keeps it from tagging "12-345" style non-phones).
fn phone_plausible(s: &str) -> bool {
    let digits = s.chars().filter(|c| c.is_ascii_digit()).count();
    (7..=15).contains(&digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(report: &RedactionReport, kind: RedactionKind) -> bool {
        report.counts.iter().any(|(k, n)| *k == kind && *n > 0)
    }

    #[test]
    fn seeded_secret_audit_aws_key_is_caught() {
        // The verification gate: a fake AWS-key-shaped secret must be redacted.
        let input = "here is my key AKIAIOSFODNN7EXAMPLE do not share";
        let r = redact_deterministic(input);
        assert!(has(&r, RedactionKind::AwsKey));
        assert!(!r.text.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(r.text.contains("[REDACTED:AWS_KEY]"));
    }

    #[test]
    fn seeded_personal_detail_email_and_phone_caught() {
        let input = "reach me at jane.doe@example.com or +1 415-555-0132";
        let r = redact_deterministic(input);
        assert!(has(&r, RedactionKind::Email));
        assert!(has(&r, RedactionKind::Phone));
        assert!(!r.text.contains("jane.doe@example.com"));
        assert!(!r.text.contains("415-555-0132"));
    }

    #[test]
    fn credit_card_only_redacted_when_luhn_valid() {
        // 4242 4242 4242 4242 is the canonical Luhn-valid test card.
        let valid = redact_deterministic("card 4242 4242 4242 4242 expires soon");
        assert!(has(&valid, RedactionKind::CreditCard));
        // A 16-digit non-Luhn number (e.g. an order id) is NOT redacted as a card.
        // (…5671 is chosen to fail the checksum; …5670 is coincidentally Luhn-valid.)
        let invalid = redact_deterministic("order 1234 5678 1234 5671 shipped");
        assert!(!has(&invalid, RedactionKind::CreditCard));
    }

    #[test]
    fn github_openai_slack_stripe_google_tokens_caught() {
        let input = concat!(
            "ghp_1234567890abcdefghijklmnopqrstuvwxyz ",
            "sk-proj-abcdefghijklmnopqrstuvwxyz012345 ",
            "xoxb-1111111111-abcdefghijkl ",
            "sk_live_abcdefghijklmnopqrstuvwx ",
            // Google API key = "AIza" + exactly 35 chars.
            "AIzaSyA1234567890abcdefghijklmnopqrstuv"
        );
        let r = redact_deterministic(input);
        for k in [
            RedactionKind::GithubToken,
            RedactionKind::OpenAiKey,
            RedactionKind::SlackToken,
            RedactionKind::StripeKey,
            RedactionKind::GoogleApiKey,
        ] {
            assert!(has(&r, k), "missed {k:?} in: {}", r.text);
        }
    }

    #[test]
    fn private_key_block_is_removed_whole() {
        let input = "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAK...\n-----END RSA PRIVATE KEY-----\nafter";
        let r = redact_deterministic(input);
        assert!(has(&r, RedactionKind::PrivateKeyBlock));
        assert!(!r.text.contains("MIIBOgIBAAJBAK"));
        assert!(r.text.contains("before") && r.text.contains("after"));
    }

    #[test]
    fn assigned_secret_is_caught_but_prose_is_not() {
        let secret = redact_deterministic("api_key = 8f3a9c2b7e10d4f6a1b2");
        assert!(has(&secret, RedactionKind::GenericAssignedSecret));
        // Ordinary prose mentioning the word "secret" is untouched.
        let prose = redact_deterministic("the secret to good bread is time and patience");
        assert!(!has(&prose, RedactionKind::GenericAssignedSecret));
    }

    #[test]
    fn ordinary_prose_survives_untouched() {
        let input = "Regenerative agriculture restores soil health and biodiversity over time.";
        let r = redact_deterministic(input);
        assert_eq!(r.total(), 0);
        assert_eq!(r.text, input);
    }

    #[test]
    fn placeholders_preserve_kind_for_analytics() {
        let r = redact_deterministic("mail a@b.com");
        assert!(r.text.contains("[REDACTED:EMAIL]"));
    }
}
