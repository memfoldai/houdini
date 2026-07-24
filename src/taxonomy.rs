pub const TAXONOMY_VERSION: i64 = 4;

pub const OTHER: &str = "other";

/// What the person asked for. Balanced across everything people bring to an AI,
/// not weighted toward any one use: coding, writing, learning, admin, personal
/// and creative work all sit in the same list. Categories follow the published
/// usage-log studies (NBER w34255's ChatGPT topic taxonomy, the Anthropic
/// Economic Index, Stack Overflow 2025, WildChat) rather than invented labels.
pub const INTENTS: &[&str] = &[
    "facts_or_lookup",
    "how_to_guidance",
    "learning_or_explanation",
    "news_or_current_events",
    "product_or_purchase_research",
    "health_or_wellbeing",
    "decision_support",
    "multi_source_synthesis",
    "troubleshooting_or_diagnosis",
    "codebase_or_system_understanding",
    "library_or_docs_lookup",
    "write_code",
    "modify_or_debug_code",
    "review_or_critique",
    "write_prose",
    "edit_or_rewrite",
    "translate_or_localize",
    "summarize_or_extract",
    "analyze_data",
    "automate_or_script",
    "configure_or_setup",
    "create_media",
    "plan_or_organize",
    "draft_communication",
    "brainstorm_or_ideate",
    "personal_or_reflective",
    "casual_conversation",
    OTHER,
];

/// NBER w34255 splits real ChatGPT traffic three ways: Asking (seeking
/// information or guidance, about half of all messages), Doing (asking the
/// model to produce or perform something) and Expressing. Derived from the
/// intent rather than asked of the model, so it costs no tokens and can never
/// contradict the label it summarises.
const ASKING: &[&str] = &[
    "facts_or_lookup",
    "how_to_guidance",
    "learning_or_explanation",
    "news_or_current_events",
    "product_or_purchase_research",
    "health_or_wellbeing",
    "decision_support",
    "multi_source_synthesis",
    "troubleshooting_or_diagnosis",
    "codebase_or_system_understanding",
    "library_or_docs_lookup",
];

const EXPRESSING: &[&str] = &["personal_or_reflective", "casual_conversation"];

pub fn shape_of(intent: &str) -> &'static str {
    if ASKING.contains(&intent) {
        "asking"
    } else if EXPRESSING.contains(&intent) {
        "expressing"
    } else if intent == OTHER {
        OTHER
    } else {
        "doing"
    }
}

pub const DOMAINS: &[&str] = &[
    "software_engineering",
    "data_and_analytics",
    "infrastructure_and_devops",
    "security",
    "product_and_design",
    "research_and_science",
    "business_and_finance",
    "marketing_and_sales",
    "legal_and_compliance",
    "people_and_hiring",
    "education_and_learning",
    "health_and_medicine",
    "personal_and_lifestyle",
    "creative_and_media",
    "customer_support",
    "operations_and_admin",
    OTHER,
];

pub const DELEGATIONS: &[&str] = &["none", "tool_call", "agent_run"];

pub const NONE: &str = "none";

/// Who the request hands work to. Read from the request text by the labeler,
/// so a tool driving another tool is recorded as an edge rather than inferred
/// later from two unrelated transcripts.
pub const DELEGATE_TARGETS: &[&str] = &[
    NONE,
    "alma",
    "claude_code",
    "codex",
    "claude",
    "chatgpt",
    "gemini",
    "copilot",
    "cursor",
    "devin",
    OTHER,
];

pub fn is_delegate_target(value: &str) -> bool {
    DELEGATE_TARGETS.contains(&value)
}

pub const MIN_DEPTH: i64 = 1;
pub const MAX_DEPTH: i64 = 4;

pub fn is_intent(value: &str) -> bool {
    INTENTS.contains(&value)
}

pub fn is_domain(value: &str) -> bool {
    DOMAINS.contains(&value)
}

pub fn is_delegation(value: &str) -> bool {
    DELEGATIONS.contains(&value)
}

pub fn is_depth(value: i64) -> bool {
    (MIN_DEPTH..=MAX_DEPTH).contains(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_is_a_stable_snake_case_id() {
        for label in INTENTS
            .iter()
            .chain(DOMAINS.iter())
            .chain(DELEGATIONS.iter())
            .chain(DELEGATE_TARGETS.iter())
        {
            assert!(
                label
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{label} is not a stable id"
            );
        }
    }

    #[test]
    fn both_facets_expose_an_open_set_escape() {
        assert!(is_intent(OTHER));
        assert!(is_domain(OTHER));
    }

    #[test]
    fn label_sets_have_no_duplicates() {
        for set in [INTENTS, DOMAINS, DELEGATIONS, DELEGATE_TARGETS] {
            let mut sorted = set.to_vec();
            sorted.sort_unstable();
            let before = sorted.len();
            sorted.dedup();
            assert_eq!(before, sorted.len());
        }
    }

    #[test]
    fn every_intent_resolves_to_a_shape() {
        for intent in INTENTS {
            let shape = shape_of(intent);
            assert!(
                ["asking", "doing", "expressing", OTHER].contains(&shape),
                "{intent} resolved to {shape}"
            );
        }
        assert_eq!(shape_of("multi_source_synthesis"), "asking");
        assert_eq!(shape_of("write_code"), "doing");
        assert_eq!(shape_of("casual_conversation"), "expressing");
    }

    #[test]
    fn shape_members_are_declared_intents() {
        for intent in ASKING.iter().chain(EXPRESSING.iter()) {
            assert!(is_intent(intent), "{intent} is not a declared intent");
        }
    }

    #[test]
    fn the_taxonomy_is_not_skewed_to_one_kind_of_use() {
        let doing = INTENTS
            .iter()
            .filter(|i| shape_of(i) == "doing")
            .count();
        assert!(
            doing >= ASKING.len(),
            "producing work must be covered at least as well as asking about it"
        );
    }

    #[test]
    fn a_delegated_run_can_name_the_tool_it_drove() {
        assert!(is_delegate_target("claude_code"));
        assert!(is_delegate_target("codex"));
        assert!(is_delegate_target(NONE));
        assert!(is_delegate_target(OTHER));
        assert!(!is_delegate_target("some_agent"));
    }

    #[test]
    fn unknown_values_are_rejected() {
        assert!(!is_intent("vibe_coding"));
        assert!(!is_intent("write_new_code"));
        assert!(!is_domain("astrology"));
        assert!(!is_delegation("subprocess"));
        assert!(!is_depth(0));
        assert!(!is_depth(5));
    }
}
