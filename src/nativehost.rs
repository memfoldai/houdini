use std::os::unix::net::UnixStream;

use ai_usage_monitor::config::Paths;
use ai_usage_monitor::webingest::{read_frame, write_frame};

pub fn run() {
    let paths = match Paths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("native-host: cannot resolve paths: {e}");
            return;
        }
    };
    ai_usage_monitor::logging::init(&paths.log_file);

    let mut app = match UnixStream::connect(&paths.sock_file) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("native-host: monitor app not running ({e}); web chat not captured");
            return;
        }
    };
    log::info!("native-host: connected to monitor, forwarding web chats");

    let mut stdin = std::io::stdin().lock();
    while let Some(bytes) = read_frame(&mut stdin) {
        if write_frame(&mut app, &bytes).is_err() {
            break;
        }
    }
    log::info!("native-host: stdin closed, exiting");
}
