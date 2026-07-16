//! Session state machine — ties the detector, store, and session recorder into
//! the observe → detect → capture → close lifecycle, for MANY windows at once.
//!
//! Every observed window ("surface") gets its own independent streaming
//! detector and session state, keyed by a stable [`SurfaceId`]. That is what
//! makes concurrent AI use work: two chat windows streaming at the same time —
//! on different displays, Spaces, or in the background — are two surfaces, two
//! detectors, two sessions. Nothing here knows how surfaces are produced; the
//! native capture layer (or a test) hands in per-surface samples.
//!
//! Clocks: detection timing uses a MONOTONIC clock (growth intervals must not
//! jump with wall-clock changes); stored session/turn timestamps use the WALL
//! clock (analytics need real time). [`TickClock`] carries both.
//!
//! What it persists: the redacted, on-screen conversation text of each detected
//! AI session, captured when a streaming run completes. Role stays `Unknown`
//! (structure, not content, decides roles — and a window snapshot has no
//! reliable structural role signal). De-duplication by redacted text keeps
//! re-captures from piling up.

use std::collections::HashMap;
use std::rc::Rc;

use crate::detector::{DetectorConfig, Sample, StreamingDetector, Verdict};
use crate::session::{SessionRecorder, TurnCandidate};
use crate::store::{Role, SourceKind, Store, app_hash};

/// Stable identity of one observed window. The capture layer guarantees the
/// same window keeps the same id across ticks (AX element identity or
/// CGWindowID); tests use any string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SurfaceId(pub String);

/// One surface's sample for this tick.
#[derive(Debug, Clone)]
pub struct SurfaceSample {
    pub surface: SurfaceId,
    /// Owning app id (unhashed; hashed before storage).
    pub app_id: String,
    /// The window's candidate output-region text.
    pub output_text: String,
    /// The user's caret is in an editable input of THIS app — growth is
    /// attributed to the user, not a model.
    pub user_typing: bool,
    /// AX vs OCR provenance.
    pub via_ocr: bool,
}

/// The two clocks for one tick (see module docs).
#[derive(Debug, Clone, Copy)]
pub struct TickClock {
    pub mono_ms: i64,
    pub wall_ms: i64,
}

/// Per-surface detection + capture state.
struct SurfaceTracker {
    det: StreamingDetector,
    session: Option<SessionRecorder>,
    app_id: String,
    streaming: bool,
    last_growth_mono: i64,
    last_seen_mono: i64,
    latest_output: String,
}

pub struct Monitor {
    store: Rc<Store>,
    salt: String,
    det_cfg: DetectorConfig,
    /// No growth for this long closes an active session.
    idle_gap_ms: i64,
    /// A surface unseen for this long (window closed / app quit) is dropped.
    /// Must exceed the caller's full-sweep period, or background surfaces
    /// would be dropped between sweeps.
    retention_ms: i64,
    surfaces: HashMap<SurfaceId, SurfaceTracker>,
}

impl Monitor {
    pub fn new(
        store: Rc<Store>,
        salt: String,
        det_cfg: DetectorConfig,
        idle_gap_ms: i64,
        retention_ms: i64,
    ) -> Self {
        Self { store, salt, det_cfg, idle_gap_ms, retention_ms, surfaces: HashMap::new() }
    }

    /// How many windows are currently being tracked (for the status line —
    /// a non-zero count is proof the capture path is seeing windows).
    pub fn surface_count(&self) -> usize {
        self.surfaces.len()
    }

    /// Current tray-facing state: capturing if ANY surface has an open session,
    /// armed if any surface is being watched.
    pub fn state(&self) -> MonitorState {
        if self.surfaces.values().any(|t| t.session.is_some()) {
            MonitorState::Capturing
        } else if self.surfaces.is_empty() {
            MonitorState::Idle
        } else {
            MonitorState::Armed
        }
    }

    /// One tick with this round's surface samples (a fast tick passes only the
    /// frontmost app's surfaces; a full sweep passes everything). Store errors
    /// are surfaced; the caller logs and keeps sampling.
    pub fn tick(
        &mut self,
        clock: TickClock,
        samples: Vec<SurfaceSample>,
    ) -> rusqlite::Result<MonitorState> {
        for sample in samples {
            self.observe(clock, sample)?;
        }
        self.close_idle_and_gone(clock)?;
        Ok(self.state())
    }

    /// Feed one sample into its surface's detector; open a session on the
    /// first streaming verdict.
    fn observe(&mut self, clock: TickClock, sample: SurfaceSample) -> rusqlite::Result<()> {
        let tracker = self.surfaces.entry(sample.surface).or_insert_with(|| SurfaceTracker {
            det: StreamingDetector::new(self.det_cfg.clone()),
            session: None,
            app_id: sample.app_id.clone(),
            streaming: false,
            last_growth_mono: 0,
            last_seen_mono: 0,
            latest_output: String::new(),
        });
        tracker.last_seen_mono = clock.mono_ms;
        tracker.app_id = sample.app_id;

        let verdict = tracker.det.push(Sample {
            ts_ms: clock.mono_ms as u64,
            text: sample.output_text.clone(),
            user_typing: sample.user_typing,
        });
        // Content-free per-surface trace for tuning: text length + verdict, no text.
        if log::log_enabled!(log::Level::Debug) && sample.output_text.chars().count() > 40 {
            log::debug!(
                "detect: app={} len={} prose_len={} typing={} verdict={:?}",
                tracker.app_id,
                sample.output_text.chars().count(),
                crate::detector::prose_len(&sample.output_text),
                sample.user_typing,
                verdict
            );
        }
        if let Verdict::Streaming { .. } = verdict {
            tracker.last_growth_mono = clock.mono_ms;
            tracker.latest_output = sample.output_text;
            tracker.streaming = true;
            if tracker.session.is_none() {
                let hash = app_hash(&self.salt, &tracker.app_id);
                let source = if sample.via_ocr { SourceKind::Ocr } else { SourceKind::Ax };
                log::info!("AI session started (source={source:?}, app_hash={hash})");
                tracker.session =
                    Some(SessionRecorder::begin(&self.store, clock.wall_ms, source, &hash)?);
            }
        }
        Ok(())
    }

    /// Close sessions idle past the gap; drop surfaces gone past retention
    /// (their window closed or app quit — an in-flight session is persisted).
    fn close_idle_and_gone(&mut self, clock: TickClock) -> rusqlite::Result<()> {
        let mut gone: Vec<SurfaceId> = Vec::new();
        for (id, tracker) in self.surfaces.iter_mut() {
            let idle_closed =
                tracker.streaming && clock.mono_ms - tracker.last_growth_mono > self.idle_gap_ms;
            let vanished = clock.mono_ms - tracker.last_seen_mono > self.retention_ms;
            if idle_closed || vanished {
                finalize(&self.store, tracker, clock.wall_ms)?;
            }
            if vanished {
                gone.push(id.clone());
            }
        }
        for id in gone {
            self.surfaces.remove(&id);
        }
        Ok(())
    }

    /// Persist every in-flight session (shutdown path).
    pub fn shutdown(&mut self, clock: TickClock) -> rusqlite::Result<()> {
        for tracker in self.surfaces.values_mut() {
            finalize(&self.store, tracker, clock.wall_ms)?;
        }
        self.surfaces.clear();
        Ok(())
    }
}

/// Record the captured conversation and close the tracker's session, if any.
fn finalize(store: &Store, tracker: &mut SurfaceTracker, wall_ms: i64) -> rusqlite::Result<()> {
    if let Some(mut rec) = tracker.session.take() {
        if !tracker.latest_output.trim().is_empty() {
            rec.record(store, &TurnCandidate {
                role: Role::Unknown,
                text: std::mem::take(&mut tracker.latest_output),
                ts_ms: wall_ms,
            })?;
        }
        let turns = rec.stored_turns();
        rec.finish(store, wall_ms)?;
        log::info!("AI session ended ({turns} turn(s) captured)");
    }
    tracker.streaming = false;
    tracker.latest_output.clear();
    Ok(())
}

/// Tray-facing state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorState {
    /// No surfaces observed.
    Idle,
    /// Watching surfaces, no AI session open.
    Armed,
    /// At least one AI session is being captured right now.
    Capturing,
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALL_BASE: i64 = 1_700_000_000_000;

    fn clock(mono_ms: i64) -> TickClock {
        TickClock { mono_ms, wall_ms: WALL_BASE + mono_ms }
    }

    fn monitor(store: &Rc<Store>) -> Monitor {
        Monitor::new(store.clone(), "salt".into(), DetectorConfig::default(), 4000, 8000)
    }

    fn sample(surface: &str, app: &str, text: &str) -> SurfaceSample {
        SurfaceSample {
            surface: SurfaceId(surface.into()),
            app_id: app.into(),
            output_text: text.into(),
            user_typing: false,
            via_ocr: true,
        }
    }

    /// Grow `parts` cumulatively onto `prefix`, one string per tick.
    fn stream_texts(prefix: &str, parts: &[&str]) -> Vec<String> {
        let mut acc = String::from(prefix);
        let mut out = vec![acc.clone()];
        for p in parts {
            acc.push_str(p);
            out.push(acc.clone());
        }
        out
    }

    #[test]
    fn two_surfaces_streaming_concurrently_become_two_sessions() {
        let store = Rc::new(Store::open_in_memory().unwrap());
        let mut mon = monitor(&store);

        // A ChatGPT browser window and a native AI app window streaming AT THE
        // SAME TIME (e.g. on two displays, or one in the background).
        let a = stream_texts("Answer one: regenerative agriculture ", &[
            "is a set of ",
            "farming practices that ",
            "restore soil health and ",
            "increase biodiversity over time.",
        ]);
        let b = stream_texts("Answer two: the capital of France ", &[
            "is Paris, which ",
            "is also the country's ",
            "largest city and ",
            "its cultural and economic center.",
        ]);
        let mut t = 0i64;
        for (ta, tb) in a.iter().zip(&b) {
            let samples = vec![
                sample("win:1", "com.google.Chrome", ta),
                sample("win:2", "com.example.aiapp", tb),
            ];
            mon.tick(clock(t), samples).unwrap();
            t += 350;
        }
        assert_eq!(mon.state(), MonitorState::Capturing);

        // Both go idle → both sessions close, independently persisted.
        mon.tick(clock(t + 5000), vec![]).unwrap();
        assert_eq!(store.session_count().unwrap(), 2);

        // Stored timestamps are wall-clock, not monotonic-from-zero.
        let sessions = store.all_sessions().unwrap();
        assert!(sessions.iter().all(|s| s.started_at_ms >= WALL_BASE));
    }

    #[test]
    fn user_typing_never_creates_a_session() {
        let store = Rc::new(Store::open_in_memory().unwrap());
        let mut mon = monitor(&store);
        let mut t = 0i64;
        for text in stream_texts("best domain ", &["extensions ", "in 2026 for ", "a startup blog"]) {
            let mut s = sample("win:9", "com.apple.Safari", &text);
            s.user_typing = true;
            mon.tick(clock(t), vec![s]).unwrap();
            t += 350;
        }
        assert_eq!(store.session_count().unwrap(), 0, "user's own typing must never be captured");
    }

    #[test]
    fn vanished_surface_closes_and_persists_its_session() {
        let store = Rc::new(Store::open_in_memory().unwrap());
        let mut mon = monitor(&store);
        let mut t = 0i64;
        for text in stream_texts("Answer: the capital of France ", &[
            "is Paris, a major European ",
            "city that sits on the river Seine ",
            "and serves as the country's ",
            "political and cultural center.",
        ]) {
            mon.tick(clock(t), vec![sample("win:5", "com.openai.chat", &text)]).unwrap();
            t += 350;
        }
        assert_eq!(mon.state(), MonitorState::Capturing);

        // The window closes: no more samples for it, past retention.
        mon.tick(clock(t + 9000), vec![]).unwrap();
        assert_eq!(store.session_count().unwrap(), 1);
        assert_eq!(mon.state(), MonitorState::Idle, "vanished surface is dropped");
    }

    #[test]
    fn captured_conversation_is_redacted_in_store() {
        let store = Rc::new(Store::open_in_memory().unwrap());
        let mut mon = monitor(&store);
        let mut t = 0i64;
        for text in stream_texts(
            "Sure. Email the vendor at ops@acme.com and note the key AKIAIOSFODNN7EXAMPLE ",
            &["for the ", "integration once ", "billing is confirmed and ", "the account is live."],
        ) {
            mon.tick(clock(t), vec![sample("win:3", "com.openai.chat", &text)]).unwrap();
            t += 350;
        }
        mon.shutdown(clock(t)).unwrap();
        let turns = store.session_turns(1).unwrap();
        assert_eq!(turns.len(), 1);
        assert!(!turns[0].redacted_text.contains("ops@acme.com"));
        assert!(!turns[0].redacted_text.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(turns[0].redacted_text.contains("[REDACTED:"));
    }
}
