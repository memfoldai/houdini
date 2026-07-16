//! Session recorder — orchestrates redact → store for one detected AI session.
//!
//! The detector (world model) decides *when* an AI session is happening; this
//! type owns *what gets persisted* while it is. Every turn is redacted BEFORE
//! it is written — raw text never reaches the store. Turns are de-duplicated by
//! their redacted content so a scrolling capture that re-sees the same turn
//! across frames does not store it twice.

use crate::redact::{self, RedactionReport};
use crate::store::{Role, SourceKind, Store};

/// A turn candidate handed in by the capture layer, with a structurally-inferred
/// role (never inferred from content meaning). `text` is RAW; the recorder
/// redacts it.
#[derive(Debug, Clone)]
pub struct TurnCandidate {
    pub role: Role,
    pub text: String,
    pub ts_ms: i64,
}

/// Records one AI session into the store, redacting and de-duplicating turns.
/// It does NOT hold the store (so it can live inside a longer-lived owner like
/// the run-loop's `Monitor` without a self-referential borrow); `&Store` is
/// threaded through each method instead.
pub struct SessionRecorder {
    session_id: i64,
    next_seq: i64,
    /// Redacted texts already written, to skip re-seen turns.
    seen: Vec<String>,
    /// Running audit tally across the session (kinds + counts), for a
    /// share-safe summary. Never holds raw values.
    audit: Vec<(redact::RedactionKind, usize)>,
}

impl SessionRecorder {
    /// Open a recorder, creating the session row.
    pub fn begin(
        store: &Store,
        started_at_ms: i64,
        source: SourceKind,
        app_hash: &str,
    ) -> rusqlite::Result<Self> {
        let session_id = store.begin_session(started_at_ms, source, app_hash)?;
        Ok(Self { session_id, next_seq: 0, seen: Vec::new(), audit: Vec::new() })
    }

    pub fn session_id(&self) -> i64 {
        self.session_id
    }

    /// Redact and record a turn candidate. Returns the redaction report (for
    /// the caller's logging/indicator). A turn whose REDACTED text was already
    /// recorded is skipped (returns the report with `text` set but nothing
    /// stored) so re-seen scroll frames don't duplicate rows.
    pub fn record(&mut self, store: &Store, cand: &TurnCandidate) -> rusqlite::Result<RedactionReport> {
        let report = redact::redact_deterministic(&cand.text);
        // Merge audit counts.
        for (k, n) in &report.counts {
            if let Some(entry) = self.audit.iter_mut().find(|(kk, _)| kk == k) {
                entry.1 += *n;
            } else {
                self.audit.push((*k, *n));
            }
        }
        let normalized = report.text.trim().to_string();
        if normalized.is_empty() || self.seen.iter().any(|s| s == &normalized) {
            return Ok(report);
        }
        store.add_turn(self.session_id, self.next_seq, cand.role, &report.text, cand.ts_ms)?;
        self.seen.push(normalized);
        self.next_seq += 1;
        Ok(report)
    }

    /// Close the session.
    pub fn finish(
        self,
        store: &Store,
        ended_at_ms: i64,
    ) -> rusqlite::Result<Vec<(redact::RedactionKind, usize)>> {
        store.end_session(self.session_id, ended_at_ms)?;
        Ok(self.audit)
    }

    /// How many turns have been stored so far.
    pub fn stored_turns(&self) -> i64 {
        self.next_seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_before_store_and_dedups_reseen_turns() {
        let store = Store::open_in_memory().unwrap();
        let mut rec = SessionRecorder::begin(&store, 1000, SourceKind::Ocr, "hash").unwrap();

        // Turn with a secret + email: stored redacted, raw never present.
        let r = rec
            .record(&store, &TurnCandidate {
                role: Role::User,
                text: "email me at a@b.com key AKIAIOSFODNN7EXAMPLE".into(),
                ts_ms: 1000,
            })
            .unwrap();
        assert!(r.total() >= 2);

        // A scrolling capture re-sees the SAME turn: must not duplicate.
        rec.record(&store, &TurnCandidate {
            role: Role::User,
            text: "email me at a@b.com key AKIAIOSFODNN7EXAMPLE".into(),
            ts_ms: 1200,
        })
        .unwrap();

        // A genuinely new assistant turn.
        rec.record(&store, &TurnCandidate {
            role: Role::Assistant,
            text: "Sure, here is the comparison you asked for.".into(),
            ts_ms: 1500,
        })
        .unwrap();

        assert_eq!(rec.stored_turns(), 2, "re-seen turn must be de-duplicated");
        let sid = rec.session_id();
        let audit = rec.finish(&store, 3000).unwrap();
        assert!(audit.iter().any(|(_, n)| *n > 0));

        let turns = store.session_turns(sid).unwrap();
        assert_eq!(turns.len(), 2);
        for turn in &turns {
            assert!(!turn.redacted_text.contains("a@b.com"), "raw email must never be stored");
            assert!(!turn.redacted_text.contains("AKIAIOSFODNN7EXAMPLE"), "raw secret must never be stored");
        }
    }

    #[test]
    fn empty_after_redaction_is_not_stored() {
        let store = Store::open_in_memory().unwrap();
        let mut rec = SessionRecorder::begin(&store, 0, SourceKind::Ax, "h").unwrap();
        // Whitespace-only candidate stores nothing.
        rec.record(&store, &TurnCandidate { role: Role::Unknown, text: "   ".into(), ts_ms: 0 }).unwrap();
        assert_eq!(rec.stored_turns(), 0);
    }
}
