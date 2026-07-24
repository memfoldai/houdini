pub const TAXONOMY_VERSION: i64 = 3;

pub const OTHER: &str = "other";

/// Activity categories taken from the usage-log studies in the research-usage
/// study (NBER w34255, Pew 2026, Anthropic Economic Index, Stack Overflow 2025,
/// WildChat), not invented. The study's central split is research-shaped use
/// (finding out or understanding) against artifact-shaped use (producing
/// something); both appear here because Houdini sees all of it.
pub const INTENTS: &[&str] = &[
    "facts_or_current_events",
    "how_to_guidance",
    "learning_a_topic",
    "health_and_wellbeing",
    "product_or_purchase_research",
    "decision_support",
    "document_grounded_research",
    "multi_source_synthesis",
    "news_monitoring",
    "search_for_answers",
    "debugging_research",
    "codebase_understanding",
    "library_or_docs_research",
    "technology_evaluation",
    "write_code",
    "modify_code",
    "review_or_critique",
    "write_or_edit_prose",
    "automate_or_script",
    "analyze_data",
    "plan_or_design",
    "configure_or_setup",
    OTHER,
];

/// The study's framing: research-shaped use is "trying to find out or
/// understand something, rather than produce an artifact". Derived from the
/// intent rather than asked of the model, so it costs nothing and can never
/// disagree with the label it summarises.
pub const RESEARCH_INTENTS: &[&str] = &[
    "facts_or_current_events",
    "how_to_guidance",
    "learning_a_topic",
    "health_and_wellbeing",
    "product_or_purchase_research",
    "decision_support",
    "document_grounded_research",
    "multi_source_synthesis",
    "news_monitoring",
    "search_for_answers",
    "debugging_research",
    "codebase_understanding",
    "library_or_docs_research",
    "technology_evaluation",
];

pub fn shape_of(intent: &str) -> &'static str {
    if RESEARCH_INTENTS.contains(&intent) {
        "research"
    } else if intent == OTHER {
        OTHER
    } else {
        "artifact"
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
                ["research", "artifact", OTHER].contains(&shape),
                "{intent} resolved to {shape}"
            );
        }
        assert_eq!(shape_of("multi_source_synthesis"), "research");
        assert_eq!(shape_of("write_code"), "artifact");
    }

    #[test]
    fn research_intents_are_a_subset_of_the_intent_set() {
        for intent in RESEARCH_INTENTS {
            assert!(is_intent(intent), "{intent} is not a declared intent");
        }
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
