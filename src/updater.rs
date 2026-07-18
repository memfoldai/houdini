use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

pub const REPO: &str = "memfoldai/ai-usage-monitor";

const UPDATE_TOKEN: Option<&str> = option_env!("AUM_UPDATE_TOKEN");

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub tag: String,
    pub version: String,
    pub asset_id: u64,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    id: u64,
}

pub fn check() -> Option<Update> {
    let token = UPDATE_TOKEN?;
    let body = run_curl(&config(token, &latest_release_url(), "application/vnd.github+json", None)).ok()?;
    let release: Release = serde_json::from_slice(&body).ok()?;

    let version = release.tag_name.trim_start_matches('v').to_string();
    if !is_newer(&version, current_version()) {
        return None;
    }
    let asset = release.assets.into_iter().find(|a| a.name.ends_with(".dmg"))?;
    Some(Update { tag: release.tag_name, version, asset_id: asset.id })
}

pub fn download_and_stage(update: &Update) -> Result<PathBuf, String> {
    let token = UPDATE_TOKEN.ok_or("no update token embedded")?;
    let bundle = installed_app_bundle()
        .ok_or_else(|| "not running from an installed .app in /Applications".to_string())?;

    let work = std::env::temp_dir().join("aum-update");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;

    let dmg = work.join("update.dmg");
    run_curl(&config(token, &asset_url(update.asset_id), "application/octet-stream", Some(&dmg)))?;

    let mount = work.join("mnt");
    std::fs::create_dir_all(&mount).map_err(|e| e.to_string())?;
    run("hdiutil", &["attach", &dmg.to_string_lossy(), "-nobrowse", "-mountpoint", &mount.to_string_lossy()])?;

    let result = swap_from_mount(&mount, &bundle);
    let _ = Command::new("hdiutil").args(["detach", &mount.to_string_lossy(), "-force"]).output();
    result?;

    Ok(bundle)
}

fn latest_release_url() -> String {
    format!("https://api.github.com/repos/{REPO}/releases/latest")
}

fn asset_url(asset_id: u64) -> String {
    format!("https://api.github.com/repos/{REPO}/releases/assets/{asset_id}")
}

fn swap_from_mount(mount: &Path, bundle: &Path) -> Result<(), String> {
    let new_app = first_with_ext(mount, "app").ok_or_else(|| "no .app inside the .dmg".to_string())?;
    run("codesign", &["--verify", "--deep", &new_app.to_string_lossy()])?;

    let staged = bundle.with_extension("app.new");
    let _ = std::fs::remove_dir_all(&staged);
    run("ditto", &[&new_app.to_string_lossy(), &staged.to_string_lossy()])?;

    let backup = bundle.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::rename(bundle, &backup).map_err(|e| format!("replace failed: {e}"))?;
    if let Err(e) = std::fs::rename(&staged, bundle) {
        let _ = std::fs::rename(&backup, bundle);
        return Err(format!("install failed, restored previous: {e}"));
    }
    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

fn installed_app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bundle = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"))?;
    bundle.starts_with("/Applications").then(|| bundle.to_path_buf())
}

fn first_with_ext(dir: &Path, ext: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == ext))
}

fn config(token: &str, url: &str, accept: &str, output: Option<&Path>) -> String {
    let mut c = String::new();
    c.push_str(&format!("url = \"{url}\"\n"));
    c.push_str(&format!("header = \"Authorization: Bearer {token}\"\n"));
    c.push_str(&format!("header = \"Accept: {accept}\"\n"));
    c.push_str("header = \"X-GitHub-Api-Version: 2022-11-28\"\n");
    c.push_str("header = \"User-Agent: ai-usage-monitor\"\n");
    c.push_str("fail\nlocation\nsilent\nshow-error\n");
    if let Some(path) = output {
        c.push_str(&format!("output = \"{}\"\n", path.to_string_lossy()));
    }
    c
}

fn run_curl(config: &str) -> Result<Vec<u8>, String> {
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
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("curl failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(cmd).args(args).output().map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{cmd} failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn is_newer(candidate: &str, current: &str) -> bool {
    parse(candidate) > parse(current)
}

fn parse(v: &str) -> (u64, u64, u64) {
    let core = v.trim_start_matches('v').split(['-', '+']).next().unwrap_or(v);
    let mut it = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert!(is_newer("0.4.5", "0.4.4"));
        assert!(is_newer("0.5.0", "0.4.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.4.4", "0.4.4"));
        assert!(!is_newer("0.4.3", "0.4.4"));
    }

    #[test]
    fn parses_tags_and_prereleases() {
        assert_eq!(parse("v0.4.4"), (0, 4, 4));
        assert_eq!(parse("0.4.4-beta.1"), (0, 4, 4));
        assert!(is_newer("v0.4.5", "0.4.4"));
    }

    #[test]
    fn parses_latest_release_json() {
        let json = br#"{"tag_name":"v0.5.0","assets":[
            {"name":"AI-Usage-Monitor-0.5.0.dmg","id":42},
            {"name":"notes.txt","id":7}]}"#;
        let release: Release = serde_json::from_slice(json).unwrap();
        assert_eq!(release.tag_name, "v0.5.0");
        let dmg = release.assets.into_iter().find(|a| a.name.ends_with(".dmg")).unwrap();
        assert_eq!(dmg.id, 42);
    }

    #[test]
    fn config_carries_auth_and_url() {
        let c = config("secret-token", &asset_url(99), "application/octet-stream", None);
        assert!(c.contains("Authorization: Bearer secret-token"));
        assert!(c.contains("releases/assets/99"));
        assert!(c.contains("application/octet-stream"));
    }
}
