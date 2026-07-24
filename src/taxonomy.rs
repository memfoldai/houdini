pub const TAXONOMY_VERSION: i64 = 1;

pub const OTHER: &str = "other";

pub const INTENTS: &[&str] = &[
    "write_new_code",
    "debug_or_fix",
    "refactor_or_cleanup",
    "review_or_critique",
    "explain_or_learn",
    "research_facts",
    "compare_options",
    "decide_or_recommend",
    "summarize_or_extract",
    "draft_prose",
    "edit_or_rewrite",
    "translate_or_localize",
    "plan_or_design",
    "analyze_data",
    "automate_or_script",
    "configure_or_setup",
    "troubleshoot_environment",
    "search_or_locate",
    "brainstorm_ideas",
    OTHER,
];

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
        for label in INTENTS.iter().chain(DOMAINS.iter()).chain(DELEGATIONS.iter()) {
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
        for set in [INTENTS, DOMAINS, DELEGATIONS] {
            let mut sorted = set.to_vec();
            sorted.sort_unstable();
            let before = sorted.len();
            sorted.dedup();
            assert_eq!(before, sorted.len());
        }
    }

    #[test]
    fn unknown_values_are_rejected() {
        assert!(!is_intent("vibe_coding"));
        assert!(!is_domain("astrology"));
        assert!(!is_delegation("subprocess"));
        assert!(!is_depth(0));
        assert!(!is_depth(5));
    }
}
