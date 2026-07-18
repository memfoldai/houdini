use std::fs;
use std::path::PathBuf;

const HOST_NAME: &str = "ai.memfold.houdini";

const EXTENSION_ID: &str = "jphmlmjmieilhimgemjanlkgfommlife";

const BROWSERS: &[(&str, &str)] = &[
    ("Google Chrome", "Google/Chrome"),
    ("Chromium", "Chromium"),
    ("Brave", "BraveSoftware/Brave-Browser"),
    ("Microsoft Edge", "Microsoft Edge"),
    ("Arc", "Arc/User Data"),
];

fn app_support() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Application Support"),
    )
}

fn write_manifests() -> usize {
    let Some(base) = app_support() else {
        return 0;
    };
    let Ok(exe) = std::env::current_exe() else {
        return 0;
    };
    let manifest = manifest_json(&exe.to_string_lossy());

    let mut installed = 0;
    for (_label, subdir) in BROWSERS {
        let browser_dir = base.join(subdir);
        if !browser_dir.exists() {
            continue;
        }
        let hosts_dir = browser_dir.join("NativeMessagingHosts");
        if fs::create_dir_all(&hosts_dir).is_err() {
            continue;
        }
        let path = hosts_dir.join(format!("{HOST_NAME}.json"));
        if fs::write(&path, &manifest).is_ok() {
            installed += 1;
        }
    }
    installed
}

pub fn ensure_installed() {
    let n = write_manifests();
    log::info!("native-messaging host registered for {n} browser(s)");
}

pub fn install() {
    let installed = write_manifests();
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    println!("Installing native-messaging host '{HOST_NAME}' → {exe}");
    if installed == 0 {
        println!("No Chromium browsers found. Install one, then re-run.");
    } else {
        println!(
            "Done ({installed}). Load the unpacked extension from ./extension (id {EXTENSION_ID}),\n\
             then use an AI web chat — its prompts/replies flow to the monitor locally."
        );
    }
}

pub fn uninstall() {
    let Some(base) = app_support() else { return };
    println!("Removing native-messaging host '{HOST_NAME}'…");
    for (label, subdir) in BROWSERS {
        let path = base
            .join(subdir)
            .join("NativeMessagingHosts")
            .join(format!("{HOST_NAME}.json"));
        if path.exists() {
            match fs::remove_file(&path) {
                Ok(()) => println!("  {label}: removed"),
                Err(e) => eprintln!("  {label}: {e}"),
            }
        }
    }
}

fn manifest_json(exe_path: &str) -> String {
    let escaped = exe_path.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "{{\n  \"name\": \"{HOST_NAME}\",\n  \"description\": \"Houdini — internal study web-chat capture (local only)\",\n  \"path\": \"{escaped}\",\n  \"type\": \"stdio\",\n  \"allowed_origins\": [\"chrome-extension://{EXTENSION_ID}/\"]\n}}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_required_fields() {
        let m = manifest_json("/Applications/Houdini.app/Contents/MacOS/houdini");
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["name"], HOST_NAME);
        assert_eq!(v["type"], "stdio");
        assert_eq!(
            v["allowed_origins"][0],
            format!("chrome-extension://{EXTENSION_ID}/")
        );
        assert!(v["path"].as_str().unwrap().ends_with("houdini"));
    }
}
