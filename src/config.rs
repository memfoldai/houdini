//! Configuration + per-install paths and salt.
//!
//! The salt anonymizes the source-app hash: it is generated once per install,
//! stored locally, and never leaves the machine, so app hashes are stable for
//! grouping within one install but not comparable across installs and never
//! reveal the app id (see `store::app_hash`).

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::detector::DetectorConfig;

const QUALIFIER: &str = "ai";
const ORG: &str = "memfold";
const APP: &str = "ai-usage-monitor";

/// Persisted, human-editable config (thresholds + operational knobs). The
/// detector thresholds live here so the tuning loop can adjust them without a
/// rebuild.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Per-install anonymization salt (hex). Generated on first run. Secret to
    /// this machine — never exported.
    pub salt: String,
    /// Per-install identifier for multi-device aggregation (UUID v4, per the
    /// OpenTelemetry `service.instance.id` recommendation). Carried in every
    /// extract so lines from different machines are distinguishable. Random —
    /// derived from nothing, reveals nothing.
    pub install_id: String,
    /// Sampling cadence while armed, in milliseconds.
    pub sample_interval_ms: u64,
    /// Every Nth tick is a FULL sweep over all windows (all displays, Spaces,
    /// background); other ticks sample only the frontmost app. Bounds the OCR
    /// cost of watching everything.
    pub full_sweep_every_ticks: u32,
    /// Windows smaller than this (points²) are skipped — a surface too small to
    /// host a conversation (status items, palettes). Tunable, not hardcoded.
    pub min_surface_area: f64,
    /// At most this many OCR captures per full sweep; the rest are logged as
    /// skipped (no silent truncation) and picked up next sweep.
    pub max_ocr_per_sweep: usize,
    /// How long (ms) of no growth ends an active AI session.
    pub session_idle_gap_ms: u64,
    /// Detector thresholds (see `DetectorConfig`).
    pub detector: DetectorConfigSerde,
    /// Optional directory holding a provisioned token-mode GLiNER-PII model
    /// (tokenizer.json + model.onnx). When set AND the binary is built with the
    /// `ner` feature, the export sweep adds the NER redaction layer over the
    /// already-deterministically-redacted text. Absent by default; the
    /// deterministic layer always runs regardless. See docs/NER.md.
    #[serde(default)]
    pub ner_model_dir: Option<PathBuf>,
}

/// Serde mirror of `DetectorConfig` (kept separate so the detector module has
/// no serde dependency).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectorConfigSerde {
    pub min_growth_steps: usize,
    pub min_step_growth_chars: usize,
    pub max_step_growth_chars: usize,
    pub exclude_typing_steps: bool,
    pub min_prose_score: f32,
    pub window: usize,
}

impl From<&DetectorConfigSerde> for DetectorConfig {
    fn from(s: &DetectorConfigSerde) -> Self {
        DetectorConfig {
            min_growth_steps: s.min_growth_steps,
            min_step_growth_chars: s.min_step_growth_chars,
            max_step_growth_chars: s.max_step_growth_chars,
            exclude_typing_steps: s.exclude_typing_steps,
            min_prose_score: s.min_prose_score,
            window: s.window,
        }
    }
}

impl Default for DetectorConfigSerde {
    fn default() -> Self {
        let d = DetectorConfig::default();
        Self {
            min_growth_steps: d.min_growth_steps,
            min_step_growth_chars: d.min_step_growth_chars,
            max_step_growth_chars: d.max_step_growth_chars,
            exclude_typing_steps: d.exclude_typing_steps,
            min_prose_score: d.min_prose_score,
            window: d.window,
        }
    }
}

impl AppConfig {
    fn fresh(salt: String, install_id: String) -> Self {
        Self {
            salt,
            install_id,
            sample_interval_ms: 350, // ~3 Hz on the frontmost app
            full_sweep_every_ticks: 6, // full multi-window sweep ~every 2.1 s
            min_surface_area: 40_000.0, // ~250×160 pt — smallest plausible chat window
            max_ocr_per_sweep: 6,
            session_idle_gap_ms: 4_000,
            detector: DetectorConfigSerde::default(),
            ner_model_dir: None,
        }
    }
}

/// Resolved on-disk locations for this install.
pub struct Paths {
    pub config_file: PathBuf,
    pub db_file: PathBuf,
    pub export_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> std::io::Result<Self> {
        let pd = ProjectDirs::from(QUALIFIER, ORG, APP).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory for project dirs")
        })?;
        let data_dir = pd.data_dir().to_path_buf();
        fs::create_dir_all(&data_dir)?;
        let export_dir = data_dir.join("exports");
        fs::create_dir_all(&export_dir)?;
        Ok(Self {
            config_file: data_dir.join("config.json"),
            db_file: data_dir.join("sessions.sqlite"),
            export_dir,
            data_dir,
        })
    }
}

/// Load config, creating it (with a fresh random salt) on first run.
pub fn load_or_init(config_file: &Path) -> std::io::Result<AppConfig> {
    if config_file.exists() {
        let bytes = fs::read(config_file)?;
        let cfg: AppConfig = serde_json::from_slice(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        return Ok(cfg);
    }
    let cfg = AppConfig::fresh(generate_salt(), generate_uuid_v4());
    let bytes = serde_json::to_vec_pretty(&cfg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(config_file, bytes)?;
    Ok(cfg)
}

/// 32 random bytes as hex.
fn generate_salt() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// RFC 4122 UUID v4 (random): 16 random bytes with the version nibble set to 4
/// and the variant bits set to 10, formatted 8-4-4-4-12.
fn generate_uuid_v4() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}{}{}{}-{}{}-{}{}-{}{}-{}{}{}{}{}{}",
        h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_or_init_is_stable_across_reloads() {
        let dir = std::env::temp_dir().join(format!("aum-cfg-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let cf = dir.join("config.json");
        let _ = fs::remove_file(&cf);
        let a = load_or_init(&cf).unwrap();
        let b = load_or_init(&cf).unwrap();
        assert_eq!(a.salt, b.salt, "salt must persist across runs");
        assert_eq!(a.salt.len(), 64);
        assert_eq!(a.install_id, b.install_id, "install id must persist across runs");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn install_id_is_rfc4122_v4_shaped() {
        let id = generate_uuid_v4();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
        assert!(parts[2].starts_with('4'), "version nibble must be 4");
        assert!(matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'), "variant bits must be 10");
    }
}
