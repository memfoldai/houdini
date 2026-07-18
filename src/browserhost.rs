use std::fs;
use std::path::PathBuf;

const HOST_NAME: &str = "ai.memfold.usage_monitor";

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

pub fn install() {
    let Some(base) = app_support() else {
        eprintln!("install-browser-host: no HOME");
        return;
    };
    let exe = std::env::current_exe().expect("current exe path");
    let manifest = manifest_json(&exe.to_string_lossy());

    println!(
        "Installing native-messaging host '{HOST_NAME}' → {}",
        exe.display()
    );
    let mut installed = 0;
    for (label, subdir) in BROWSERS {
        let browser_dir = base.join(subdir);

        if !browser_dir.exists() {
            continue;
        }
        let hosts_dir = browser_dir.join("NativeMessagingHosts");
        if let Err(e) = fs::create_dir_all(&hosts_dir) {
            eprintln!("  {label}: cannot create {}: {e}", hosts_dir.display());
            continue;
        }
        let path = hosts_dir.join(format!("{HOST_NAME}.json"));
        match fs::write(&path, &manifest) {
            Ok(()) => {
                println!("  {label}: {}", path.display());
                installed += 1;
            }
            Err(e) => eprintln!("  {label}: write failed: {e}"),
        }
    }
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
        "{{\n  \"name\": \"{HOST_NAME}\",\n  \"description\": \"AI Usage Monitor — internal study web-chat capture (local only)\",\n  \"path\": \"{escaped}\",\n  \"type\": \"stdio\",\n  \"allowed_origins\": [\"chrome-extension://{EXTENSION_ID}/\"]\n}}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_required_fields() {
        let m = manifest_json("/Applications/AI Usage Monitor.app/Contents/MacOS/ai-usage-monitor");
        let v: serde_json::Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["name"], HOST_NAME);
        assert_eq!(v["type"], "stdio");
        assert_eq!(
            v["allowed_origins"][0],
            format!("chrome-extension://{EXTENSION_ID}/")
        );
        assert!(v["path"].as_str().unwrap().ends_with("ai-usage-monitor"));
    }
}
