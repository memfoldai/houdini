use std::fs::{self, OpenOptions};
use std::path::Path;

const MAX_LOG_BYTES: u64 = 1_000_000;

pub fn init(log_file: &Path) {
    rotate_if_large(log_file);

    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    builder.format_timestamp_millis();
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(log_file) {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }

    let _ = builder.try_init();
}

fn rotate_if_large(log_file: &Path) {
    let Ok(meta) = fs::metadata(log_file) else {
        return;
    };
    if meta.len() <= MAX_LOG_BYTES {
        return;
    }

    let mut backup = log_file.as_os_str().to_owned();
    backup.push(".1");
    let _ = fs::rename(log_file, backup);
}
