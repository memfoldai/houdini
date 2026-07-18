use regex::Regex;
use std::sync::OnceLock;

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

#[derive(Debug, Clone, Default)]
pub struct RedactionReport {
    pub text: String,

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

    validate: Option<fn(&str) -> bool>,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![

            Rule {
                kind: RedactionKind::AwsKey,
                re: Regex::new(r"\b(?:AKIA|ASIA|AGPA|AIDA|AROA|AIPA|ANPA|ANVA)[A-Z0-9]{16}\b").unwrap(),
                validate: None,
            },

            Rule {
                kind: RedactionKind::GithubToken,
                re: Regex::new(r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b|\bgithub_pat_[A-Za-z0-9_]{22,}\b").unwrap(),
                validate: None,
            },

            Rule {
                kind: RedactionKind::OpenAiKey,
                re: Regex::new(r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{20,}\b").unwrap(),
                validate: None,
            },

            Rule {
                kind: RedactionKind::SlackToken,
                re: Regex::new(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b").unwrap(),
                validate: None,
            },

            Rule {
                kind: RedactionKind::StripeKey,
                re: Regex::new(r"\b(?:sk|rk|pk)_live_[A-Za-z0-9]{16,}\b").unwrap(),
                validate: None,
            },

            Rule {
                kind: RedactionKind::GoogleApiKey,
                re: Regex::new(r"\bAIza[0-9A-Za-z_\-]{35}\b").unwrap(),
                validate: None,
            },

            Rule {
                kind: RedactionKind::PrivateKeyBlock,
                re: Regex::new(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----").unwrap(),
                validate: None,
            },

            Rule {
                kind: RedactionKind::GenericAssignedSecret,
                re: Regex::new(r#"(?i)\b(?:api[_-]?key|secret|token|password|passwd|access[_-]?key|client[_-]?secret)\b\s*[:=]\s*['"]?([A-Za-z0-9_\-\.]{12,})['"]?"#).unwrap(),
                validate: None,
            },

            Rule {
                kind: RedactionKind::Email,
                re: Regex::new(r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b").unwrap(),
                validate: None,
            },

            Rule {
                kind: RedactionKind::CreditCard,
                re: Regex::new(r"\b(?:\d[ -]?){13,19}\b").unwrap(),
                validate: Some(luhn_ok),
            },

            Rule {
                kind: RedactionKind::UsSsn,
                re: Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(),
                validate: Some(ssn_plausible),
            },

            Rule {
                kind: RedactionKind::Phone,
                re: Regex::new(r"\b(?:\+?\d{1,3}[ .\-]?)?(?:\(\d{2,4}\)[ .\-]?)?\d{3,4}[ .\-]\d{3,4}(?:[ .\-]\d{2,4})?\b").unwrap(),
                validate: Some(phone_plausible),
            },
        ]
    })
}

pub fn redact_deterministic(input: &str) -> RedactionReport {
    let mut report = RedactionReport {
        text: input.to_string(),
        counts: Vec::new(),
    };
    for rule in rules() {
        let placeholder = rule.kind.placeholder();
        let mut n = 0usize;
        let out = rule.re.replace_all(&report.text, |caps: &regex::Captures| {
            let whole = caps.get(0).unwrap().as_str();
            if let Some(v) = rule.validate {
                if !v(whole) {
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
        let valid = redact_deterministic("card 4242 4242 4242 4242 expires soon");
        assert!(has(&valid, RedactionKind::CreditCard));

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
