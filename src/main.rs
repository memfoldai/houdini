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

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("houdini is macOS-only");
}
