//! Configuration + per-install paths.
//!
//! The identity the study records — provider, tool, surface — is stored in the
//! clear, so there is no anonymization salt anymore. The only per-install secret
//! that remains is `install_id`: a random device id so pooled day files stay
//! attributable per machine. Only interaction CONTENT is redacted (see
//! `redact`), and that happens before storage regardless of config.

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const QUALIFIER: &str = "ai";
const ORG: &str = "memfold";
const APP: &str = "ai-usage-monitor";

/// Persisted, human-editable config. Every field carries a `serde(default)` so a
/// file written by an older version still loads (missing fields default) and is
/// rewritten to gain them; a genuinely corrupt file is backed up and reset
/// rather than crashing the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Per-install device id (UUID v4) for multi-device aggregation. Random,
    /// reveals nothing, stable across runs.
    #[serde(default = "generate_uuid_v4")]
    pub install_id: String,
    /// How often to scan transcripts for new interactions (ms).
    #[serde(default = "d_transcript_poll_ms")]
    pub transcript_poll_ms: u64,
    /// How often to poll the process table for AI network connections (ms).
    #[serde(default = "d_network_poll_ms")]
    pub network_poll_ms: u64,
    /// A provider unseen on the network for this long closes its presence
    /// interval (ms). Bridges brief connection churn into one "was active" span.
    #[serde(default = "d_presence_gap_ms")]
    pub presence_gap_ms: u64,
    /// How often to flush new/closed records to day files (ms).
    #[serde(default = "d_flush_ms")]
    pub flush_ms: u64,
    /// Optional GLiNER-PII model dir; enables the NER redaction layer when the
    /// binary is built with the `ner` feature (see docs/NER.md). The
    /// deterministic redaction layer always runs regardless.
    #[serde(default)]
    pub ner_model_dir: Option<PathBuf>,
}

fn d_transcript_poll_ms() -> u64 {
    5_000
}
fn d_network_poll_ms() -> u64 {
    5_000
}
fn d_presence_gap_ms() -> u64 {
    60_000
}
fn d_flush_ms() -> u64 {
    15_000
}

impl AppConfig {
    fn fresh(install_id: String) -> Self {
        Self {
            install_id,
            transcript_poll_ms: d_transcript_poll_ms(),
            network_poll_ms: d_network_poll_ms(),
            presence_gap_ms: d_presence_gap_ms(),
            flush_ms: d_flush_ms(),
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
    /// Diagnostics log (a named product artifact). Capped + rotated by `logging`.
    pub log_file: PathBuf,
}

impl Paths {
    pub fn resolve() -> std::io::Result<Self> {
        let pd = ProjectDirs::from(QUALIFIER, ORG, APP).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory for project dirs")
        })?;
        let data_dir = pd.data_dir().to_path_buf();
        fs::create_dir_all(&data_dir)?;
        let export_dir = data_dir.join("data");
        fs::create_dir_all(&export_dir)?;
        Ok(Self {
            config_file: data_dir.join("config.json"),
            db_file: data_dir.join("sessions.sqlite"),
            export_dir,
            log_file: data_dir.join("ai-usage-monitor.log"),
            data_dir,
        })
    }
}

/// Load config, or create it on first run. Upgrade-safe (missing fields default
/// and are rewritten); an unparseable file is backed up and replaced.
pub fn load_or_init(config_file: &Path) -> std::io::Result<AppConfig> {
    if config_file.exists() {
        let bytes = fs::read(config_file)?;
        match serde_json::from_slice::<AppConfig>(&bytes) {
            Ok(cfg) => {
                write_config(config_file, &cfg)?;
                return Ok(cfg);
            }
            Err(e) => {
                let bad = config_file.with_extension("json.bad");
                let _ = fs::rename(config_file, &bad);
                log::error!("config unparseable ({e}); backed up to {} and starting fresh", bad.display());
            }
        }
    }
    let cfg = AppConfig::fresh(generate_uuid_v4());
    write_config(config_file, &cfg)?;
    Ok(cfg)
}

fn write_config(config_file: &Path, cfg: &AppConfig) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(cfg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(config_file, bytes)
}

/// RFC 4122 UUID v4 (random).
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
        assert_eq!(a.install_id, b.install_id, "install id must persist across runs");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn old_config_missing_new_fields_still_loads_and_upgrades() {
        let dir = std::env::temp_dir().join(format!("aum-oldcfg-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let cf = dir.join("config.json");
        // A minimal older file: just an install id.
        fs::write(&cf, r#"{"install_id":"keep-me"}"#).unwrap();
        let cfg = load_or_init(&cf).expect("old config must load, not crash");
        assert_eq!(cfg.install_id, "keep-me", "existing id preserved");
        assert_eq!(cfg.transcript_poll_ms, 5_000, "missing field takes default");
        let reread = fs::read_to_string(&cf).unwrap();
        assert!(reread.contains("network_poll_ms"), "file upgraded on load");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn install_id_is_rfc4122_v4_shaped() {
        let id = generate_uuid_v4();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
        assert!(parts[2].starts_with('4'), "version nibble must be 4");
    }
}
