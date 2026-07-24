use houdini::analytics::{ProxyLabeler, DEFAULT_BATCH_LIMIT_HINT};
use houdini::analytics_job;
use houdini::config::{self, Paths};
use houdini::store::Store;

pub fn run() {
    let Ok(paths) = Paths::resolve() else {
        eprintln!("cannot resolve the data directory");
        std::process::exit(1);
    };
    let Ok(cfg) = config::load_or_init(&paths.config_file) else {
        eprintln!("cannot read config.json");
        std::process::exit(1);
    };
    let Some(api_key) = crate::keychain::analytics_key() else {
        eprintln!("no analytics key: copy it, then use the menu item, or pipe it to --set-analytics-key");
        std::process::exit(1);
    };
    let key = match crate::keychain::db_key() {
        Ok(key) => key,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let store = match Store::open(&paths.db_file, &key) {
        Ok(store) => store,
        Err(e) => {
            eprintln!("cannot open the encrypted store: {e}");
            std::process::exit(1);
        }
    };

    let labeler = ProxyLabeler::new(cfg.analytics_base_url, cfg.analytics_model, api_key);
    let limit = std::env::args()
        .skip_while(|a| a != "--analyze-once")
        .nth(1)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_BATCH_LIMIT_HINT);

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default();

    println!("labeling up to {limit} turn(s)…");
    match analytics_job::run_once(&store, &labeler, limit, now_ms) {
        Ok(report) => {
            println!(
                "considered {} | labeled {} | failed {} | candidates {}",
                report.considered, report.labeled, report.failed, report.candidates
            );
            for cell in store.label_counts(houdini::taxonomy::TAXONOMY_VERSION).unwrap_or_default() {
                println!(
                    "  {:>5}  {} / {}  depth={} delegation={}",
                    cell.turns, cell.intent, cell.domain, cell.depth, cell.delegation
                );
            }
        }
        Err(e) => {
            eprintln!("analytics failed: {e}");
            std::process::exit(1);
        }
    }
}
