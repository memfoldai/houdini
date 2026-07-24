use crate::analytics::{Label, LabelRequest, Labeler, PROMPT_VERSION};
use crate::store::{LabelCandidate, Store, TurnLabelRecord};
use crate::taxonomy::TAXONOMY_VERSION;

pub const DEFAULT_BATCH_LIMIT: i64 = 25;

/// Earlier turns handed to the labeler so a follow-up like "now the other one"
/// is readable. The published method uses up to ten preceding messages.
const CONTEXT_TURNS: i64 = 6;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobReport {
    pub considered: usize,
    pub labeled: usize,
    pub failed: usize,
    pub candidates: usize,
}

impl JobReport {
    pub fn is_idle(&self) -> bool {
        self.considered == 0
    }
}

pub fn collect(store: &Store, limit: i64) -> rusqlite::Result<Vec<LabelRequest>> {
    let pending = store.unlabeled_turns(TAXONOMY_VERSION, PROMPT_VERSION, limit)?;
    let mut requests = Vec::with_capacity(pending.len());
    for turn in pending {
        if turn.redacted_text.trim().is_empty() {
            continue;
        }
        let context = store
            .preceding_turns(turn.session_id, turn.seq, CONTEXT_TURNS)?
            .into_iter()
            .filter(|t| !t.redacted_text.trim().is_empty())
            .map(|t| format!("{}: {}", t.role, t.redacted_text))
            .collect();
        requests.push(LabelRequest {
            session_id: turn.session_id,
            seq: turn.seq,
            text: turn.redacted_text,
            context,
        });
    }
    Ok(requests)
}

pub fn label_batch(labeler: &dyn Labeler, requests: &[LabelRequest]) -> Vec<Result<Label, String>> {
    requests.iter().map(|r| labeler.label(r)).collect()
}

pub fn persist(
    store: &Store,
    model: &str,
    results: &[Result<Label, String>],
    now_ms: i64,
) -> rusqlite::Result<JobReport> {
    let mut report = JobReport {
        considered: results.len(),
        ..JobReport::default()
    };
    for result in results {
        let label = match result {
            Ok(label) => label,
            Err(_) => {
                report.failed += 1;
                continue;
            }
        };
        let record = TurnLabelRecord {
            session_id: label.session_id,
            seq: label.seq,
            taxonomy_version: TAXONOMY_VERSION,
            prompt_version: PROMPT_VERSION,
            model,
            intent: &label.intent,
            domain: &label.domain,
            depth: label.depth,
            delegation: &label.delegation,
            delegate_tool: &label.delegate_tool,
            confidence: label.confidence,
            analyzed_at_ms: now_ms,
        };
        match store.insert_turn_label(&record) {
            Ok(_) => report.labeled += 1,
            Err(e) => {
                log::warn!("analytics: refused label for turn {}: {e}", label.seq);
                report.failed += 1;
                continue;
            }
        }

        for (facet, proposed) in [
            ("intent", label.proposed_intent.as_deref()),
            ("domain", label.proposed_domain.as_deref()),
        ] {
            let Some(proposed) = proposed else { continue };
            store.record_label_candidate(&LabelCandidate {
                taxonomy_version: TAXONOMY_VERSION,
                prompt_version: PROMPT_VERSION,
                model,
                facet,
                proposed,
                rationale: label.proposal_rationale.as_deref().unwrap_or(""),
                seen_at_ms: now_ms,
            })?;
            report.candidates += 1;
        }
    }
    Ok(report)
}

pub fn run_once(
    store: &Store,
    labeler: &dyn Labeler,
    limit: i64,
    now_ms: i64,
) -> rusqlite::Result<JobReport> {
    let requests = collect(store, limit)?;
    if requests.is_empty() {
        return Ok(JobReport::default());
    }
    let results = label_batch(labeler, &requests);
    persist(store, labeler.model(), &results, now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Role, SessionUpsert};

    struct FakeLabeler {
        label: Option<Label>,
    }

    impl Labeler for FakeLabeler {
        fn model(&self) -> &str {
            "fake-model"
        }
        fn label(&self, request: &LabelRequest) -> Result<Label, String> {
            match &self.label {
                Some(label) => Ok(Label {
                    session_id: request.session_id,
                    seq: request.seq,
                    ..label.clone()
                }),
                None => Err("proxy unreachable".to_string()),
            }
        }
    }

    fn label(intent: &str, proposed: Option<&str>) -> Label {
        Label {
            session_id: 0,
            seq: 0,
            intent: intent.to_string(),
            domain: "software_engineering".to_string(),
            depth: 2,
            delegation: "none".to_string(),
            delegate_tool: "none".to_string(),
            confidence: 0.9,
            proposed_intent: proposed.map(str::to_string),
            proposed_domain: None,
            proposal_rationale: proposed.map(|_| "nothing listed covered it".to_string()),
        }
    }

    fn store_with_turns(count: i64) -> Store {
        let store = Store::open_in_memory().unwrap();
        let (id, _) = store
            .upsert_session(&SessionUpsert {
                tool: "claude-code",
                external_id: "s1",
                provider: "anthropic",
                surface: "cli",
                model: None,
                started_at_ms: 0,
                ended_at_ms: 0,
                message_count: count,
            })
            .unwrap();
        for seq in 0..count {
            store
                .add_turn(id, seq, Role::User, &format!("request {seq}"), seq)
                .unwrap();
            store
                .add_turn(id, seq + 1_000, Role::Assistant, "reply", seq)
                .unwrap();
        }
        store
    }

    #[test]
    fn only_user_turns_are_queued_for_labeling() {
        let store = store_with_turns(3);
        let requests = collect(&store, 100).unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|r| r.text.starts_with("request")));
    }

    #[test]
    fn the_queue_respects_its_limit() {
        let store = store_with_turns(10);
        assert_eq!(collect(&store, 4).unwrap().len(), 4);
    }

    #[test]
    fn labeled_turns_leave_the_queue_and_re_running_is_idempotent() {
        let store = store_with_turns(2);
        let labeler = FakeLabeler {
            label: Some(label("debugging_research", None)),
        };
        let first = run_once(&store, &labeler, 100, 1_000).unwrap();
        assert_eq!(first.labeled, 2);
        assert!(collect(&store, 100).unwrap().is_empty());

        let second = run_once(&store, &labeler, 100, 2_000).unwrap();
        assert!(second.is_idle());
        assert_eq!(store.label_cells(TAXONOMY_VERSION).unwrap()[0].turns, 2);
    }

    #[test]
    fn a_failed_call_stores_nothing_and_leaves_the_turn_queued() {
        let store = store_with_turns(2);
        let labeler = FakeLabeler { label: None };
        let report = run_once(&store, &labeler, 100, 1_000).unwrap();
        assert_eq!(report.failed, 2);
        assert_eq!(report.labeled, 0);
        assert_eq!(collect(&store, 100).unwrap().len(), 2);
        assert!(store.label_cells(TAXONOMY_VERSION).unwrap().is_empty());
    }

    #[test]
    fn an_other_label_records_a_deduplicated_candidate_with_its_observation_count() {
        let store = store_with_turns(3);
        let labeler = FakeLabeler {
            label: Some(label("other", Some("pair_programming"))),
        };
        let report = run_once(&store, &labeler, 100, 1_000).unwrap();
        assert_eq!(report.candidates, 3);

        let candidates = store.all_label_candidates().unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].proposed, "pair_programming");
        assert_eq!(candidates[0].observations, 3);
    }

    #[test]
    fn an_empty_store_makes_no_calls() {
        let store = Store::open_in_memory().unwrap();
        let labeler = FakeLabeler { label: None };
        assert!(run_once(&store, &labeler, 100, 1_000).unwrap().is_idle());
    }
}
