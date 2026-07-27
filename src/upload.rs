use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::AppConfig;

const CONNECT_TIMEOUT_S: u64 = 10;
const REQUEST_TIMEOUT_S: u64 = 60;

// curl retries transient failures itself so a push that races a waking laptop or
// a collector that is mid-restart still lands without waiting for the next tick.
const RETRIES: u64 = 4;
const RETRY_DELAY_S: u64 = 3;

/// How the device finds and authenticates to the collector. The token is always
/// required. The base URL is either baked static (a stable hostname) or found at
/// runtime from a discovery pointer — a public URL holding the collector's
/// current address, which is how a laptop behind a rotating quick tunnel stays
/// reachable without a domain.
#[derive(Clone)]
pub struct Config {
    pub static_url: Option<String>,
    pub discovery_url: Option<String>,
    pub token: String,
}

/// None turns uploads off (a dev build with nothing provisioned). Some requires
/// a token plus at least one way to reach the collector.
pub fn from_config(cfg: &AppConfig) -> Option<Config> {
    let token = upload_token()?;
    let static_url = env_or(cfg.upload_url.clone(), "HOUDINI_UPLOAD_URL");
    let discovery_url = env_or(cfg.upload_discovery_url.clone(), "HOUDINI_UPLOAD_DISCOVERY");
    if static_url.is_none() && discovery_url.is_none() {
        return None;
    }
    Some(Config {
        static_url,
        discovery_url,
        token,
    })
}

fn env_or(configured: Option<String>, var: &str) -> Option<String> {
    configured
        .or_else(|| std::env::var(var).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn upload_token() -> Option<String> {
    std::env::var("HOUDINI_UPLOAD_TOKEN")
        .ok()
        .or_else(|| option_env!("HOUDINI_UPLOAD_TOKEN").map(str::to_string))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Resolves the current collector base URL, preferring a baked static hostname
/// and otherwise fetching the discovery pointer fresh so a rotated tunnel URL is
/// picked up without a restart.
fn base_url(cfg: &Config) -> Result<String, String> {
    if let Some(url) = &cfg.static_url {
        return Ok(url.trim_end_matches('/').to_string());
    }
    let discovery = cfg
        .discovery_url
        .as_deref()
        .ok_or("upload: no collector url or discovery pointer")?;
    let body = get(discovery)?;
    let url = body
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("http"))
        .ok_or("upload: discovery pointer had no url")?;
    Ok(url.trim_end_matches('/').to_string())
}

/// Posts the analytics NDJSON to the collector. The collector upserts by row
/// identity, so re-sending the whole export every cycle is safe and needs no
/// client-side watermark; combined with curl's own retries this is what makes a
/// laptop that just came online catch the collector up with nothing missed and
/// nothing duplicated. Callers run this off the UI thread (curl can block).
pub fn push(cfg: &Config, body: &str) -> Result<(), String> {
    if body.trim().is_empty() {
        return Ok(());
    }
    let url = base_url(cfg)?;
    let tmp = std::env::temp_dir().join(format!("houdini-upload-{}.ndjson", std::process::id()));
    write_private(&tmp, body).map_err(|e| format!("stage upload: {e}"))?;
    let result = post(&url, &cfg.token, &tmp);
    let _ = std::fs::remove_file(&tmp);
    result
}

fn get(url: &str) -> Result<String, String> {
    let out = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--location",
            "--connect-timeout",
            &CONNECT_TIMEOUT_S.to_string(),
            "--max-time",
            &REQUEST_TIMEOUT_S.to_string(),
            url,
        ])
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "curl get failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The token and headers ride in a curl config on stdin so they never appear in
/// the process argument list; the NDJSON body is a 0600 temp file referenced by
/// `@path` so an arbitrarily large payload needs no shell escaping. The body is
/// already-redacted analytics, so the temp file is not sensitive beyond tidiness.
fn post(url: &str, token: &str, body_file: &Path) -> Result<(), String> {
    let config = curl_config(url, token, body_file);
    let mut child = Command::new("curl")
        .arg("--config")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("curl: no stdin")?
        .write_all(config.as_bytes())
        .map_err(|e| format!("curl: {e}"))?;
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "curl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn curl_config(url: &str, token: &str, body_file: &Path) -> String {
    let ingest = format!("{url}/v1/ingest");
    let mut c = String::new();
    c.push_str(&format!("url = \"{}\"\n", esc(&ingest)));
    c.push_str(&format!("header = \"Authorization: Bearer {}\"\n", esc(token)));
    c.push_str("header = \"Content-Type: application/x-ndjson\"\n");
    // Bypass ngrok/tunnel browser-warning interstitials for API clients.
    c.push_str("header = \"ngrok-skip-browser-warning: true\"\n");
    c.push_str(&format!("data-binary = \"@{}\"\n", esc(&body_file.to_string_lossy())));
    c.push_str(&format!("max-time = {REQUEST_TIMEOUT_S}\n"));
    c.push_str(&format!("connect-timeout = {CONNECT_TIMEOUT_S}\n"));
    c.push_str(&format!("retry = {RETRIES}\n"));
    c.push_str(&format!("retry-delay = {RETRY_DELAY_S}\n"));
    c.push_str("retry-connrefused\nretry-all-errors\n");
    c.push_str("fail\nsilent\nshow-error\n");
    c
}

fn esc(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn write_private(path: &Path, body: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(body.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(url: Option<&str>, disco: Option<&str>) -> Config {
        Config {
            static_url: url.map(str::to_string),
            discovery_url: disco.map(str::to_string),
            token: "t".into(),
        }
    }

    #[test]
    fn a_static_url_wins_and_is_trimmed() {
        assert_eq!(base_url(&cfg(Some("https://c.example/"), None)).unwrap(), "https://c.example");
    }

    #[test]
    fn no_url_and_no_pointer_is_an_error() {
        assert!(base_url(&cfg(None, None)).is_err());
    }

    #[test]
    fn curl_config_targets_ingest_hides_token_and_retries() {
        let c = curl_config("https://c.example", "secret", Path::new("/tmp/b.ndjson"));
        assert!(c.contains("url = \"https://c.example/v1/ingest\""));
        assert!(c.contains("Authorization: Bearer secret"));
        assert!(c.contains("data-binary = \"@/tmp/b.ndjson\""));
        assert!(c.contains("retry = 4"));
        assert!(c.contains("retry-connrefused"));
    }

    #[test]
    fn an_empty_body_is_a_noop() {
        assert!(push(&cfg(Some("https://c.example"), None), "  \n ").is_ok());
    }
}
