#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod browserhost;
#[cfg(target_os = "macos")]
mod analyze_once;
mod diagnose;
#[cfg(target_os = "macos")]
mod keychain;
mod loginitem;
#[cfg(target_os = "macos")]
mod nativehost;
#[cfg(target_os = "macos")]
mod tray_glyph;
#[cfg(target_os = "macos")]
mod updater;

#[cfg(target_os = "macos")]
fn main() {
    let args: Vec<String> = std::env::args().collect();

    let is_native_host = args
        .iter()
        .any(|a| a.starts_with("chrome-extension://") || a == "--native-host");
    if is_native_host {
        nativehost::run();
        return;
    }
    if args.iter().any(|a| a == "--install-browser-host") {
        browserhost::install();
        return;
    }
    if args.iter().any(|a| a == "--uninstall-browser-host") {
        browserhost::uninstall();
        return;
    }
    if args.iter().any(|a| a == "--disable-login-item") {
        loginitem::unregister();
        return;
    }

    if args.iter().any(|a| a == "--set-analytics-key") {
        let mut key = String::new();
        if std::io::stdin().read_line(&mut key).is_err() || key.trim().is_empty() {
            eprintln!("read the key from stdin: printf %s \"$KEY\" | houdini --set-analytics-key");
            std::process::exit(1);
        }
        match keychain::set_analytics_key(&key) {
            Ok(()) => println!("analytics key stored in the login keychain"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }

    if args.iter().any(|a| a == "--analyze-once") {
        analyze_once::run();
        return;
    }

    if args.iter().any(|a| a == "--diagnose") {
        diagnose::run();
        return;
    }
    if args.iter().any(|a| a == "--export-once") {
        export_once();
        return;
    }
    if args.iter().any(|a| a == "--upload-once") {
        upload_once();
        return;
    }
    if args.iter().any(|a| a == "--check-update") {
        match updater::check() {
            Some(u) => println!(
                "update available: {} (current {})",
                u.version,
                updater::current_version()
            ),
            None => println!("up to date ({})", updater::current_version()),
        }
        return;
    }

    app::run();
}

/// Rewrites the analytics export from the current store without labeling. The
/// session-span minutes are recomputed under the newest engaged-time rules, so
/// this is how a device refreshes last week's snapshot for the weekly wrapped
/// and how the uploader gets a current file to send.
#[cfg(target_os = "macos")]
fn export_once() {
    let paths = houdini::config::Paths::resolve().unwrap_or_else(|_| {
        eprintln!("cannot resolve the data directory");
        std::process::exit(1);
    });
    let cfg = houdini::config::load_or_init(&paths.config_file).unwrap_or_else(|_| {
        eprintln!("cannot read config.json");
        std::process::exit(1);
    });
    let secrets = keychain::load().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    let store = houdini::store::Store::open(&paths.db_file, &secrets.db_key).unwrap_or_else(|e| {
        eprintln!("cannot open the encrypted store: {e}");
        std::process::exit(1);
    });
    let identity = houdini::export::ExportIdentity {
        install_id: &cfg.install_id,
        person: &cfg.person,
        device_name: &cfg.device_name,
    };
    match houdini::export::export_analytics(&store, &identity, &paths.export_dir) {
        Ok(path) => println!("exported {}", path.display()),
        Err(e) => {
            eprintln!("export failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Refreshes the export and pushes it to the collector once, synchronously.
/// This is the manual/testing counterpart to the app's automatic upload; it
/// resolves the same endpoint (HOUDINI_UPLOAD_URL / HOUDINI_UPLOAD_TOKEN or
/// config) so a device or CI can force a send.
#[cfg(target_os = "macos")]
fn upload_once() {
    let paths = houdini::config::Paths::resolve().unwrap_or_else(|_| {
        eprintln!("cannot resolve the data directory");
        std::process::exit(1);
    });
    let cfg = houdini::config::load_or_init(&paths.config_file).unwrap_or_else(|_| {
        eprintln!("cannot read config.json");
        std::process::exit(1);
    });
    let Some(upload) = houdini::upload::from_config(&cfg) else {
        eprintln!("upload not configured: set HOUDINI_UPLOAD_TOKEN plus HOUDINI_UPLOAD_URL or HOUDINI_UPLOAD_DISCOVERY");
        std::process::exit(1);
    };
    let secrets = keychain::load().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    let store = houdini::store::Store::open(&paths.db_file, &secrets.db_key).unwrap_or_else(|e| {
        eprintln!("cannot open the encrypted store: {e}");
        std::process::exit(1);
    });
    let identity = houdini::export::ExportIdentity {
        install_id: &cfg.install_id,
        person: &cfg.person,
        device_name: &cfg.device_name,
    };
    let path = houdini::export::export_analytics(&store, &identity, &paths.export_dir)
        .unwrap_or_else(|e| {
            eprintln!("export failed: {e}");
            std::process::exit(1);
        });
    let body = std::fs::read_to_string(&path).unwrap_or_default();
    match houdini::upload::push(&upload, &body) {
        Ok(()) => println!("pushed {}", path.display()),
        Err(e) => {
            eprintln!("upload failed: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("houdini is macOS-only");
}
