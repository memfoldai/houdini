use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO: &str = "memfoldai/ai-usage-monitor";

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub tag: String,
    pub version: String,
}

pub fn check() -> Option<Update> {
    let out = Command::new("gh")
        .args(["api", &format!("repos/{REPO}/releases/latest"), "--jq", ".tag_name"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let tag = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if tag.is_empty() {
        return None;
    }
    let version = tag.trim_start_matches('v').to_string();
    is_newer(&version, current_version()).then_some(Update { tag, version })
}

pub fn install(update: &Update) -> Result<(), String> {
    let bundle = installed_app_bundle()
        .ok_or_else(|| "not running from an installed .app in /Applications".to_string())?;

    let work = std::env::temp_dir().join("aum-update");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;

    run("gh", &["release", "download", &update.tag, "--repo", REPO, "--pattern", "*.dmg", "--dir", &work.to_string_lossy()])?;
    let dmg = first_with_ext(&work, "dmg").ok_or_else(|| "release has no .dmg asset".to_string())?;

    let mount = work.join("mnt");
    std::fs::create_dir_all(&mount).map_err(|e| e.to_string())?;
    run("hdiutil", &["attach", &dmg.to_string_lossy(), "-nobrowse", "-mountpoint", &mount.to_string_lossy()])?;

    let result = swap_from_mount(&mount, &bundle);
    let _ = Command::new("hdiutil").args(["detach", &mount.to_string_lossy(), "-force"]).output();
    result?;

    relaunch(&bundle);
    Ok(())
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

fn relaunch(bundle: &Path) {
    let _ = Command::new("open").arg("-n").arg(bundle).spawn();
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
}
