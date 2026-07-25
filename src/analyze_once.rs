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
    let secrets = match crate::keychain::load() {
        Ok(secrets) => secrets,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let Some(api_key) = secrets.analytics_key else {
        eprintln!("no analytics key: pipe it to --set-analytics-key first");
        std::process::exit(1);
    };
    let store = match Store::open(&paths.db_file, &secrets.db_key) {
        Ok(store) => store,
        Err(e) => {
            eprintln!("cannot open the encrypted store: {e}");
            std::process::exit(1);
        }
    };

    if let Ok(removed) = store.drop_superseded_labels(
        houdini::taxonomy::TAXONOMY_VERSION,
        houdini::analytics::PROMPT_VERSION,
    ) {
        if removed > 0 {
            println!("cleared {removed} label(s) from a superseded taxonomy");
        }
    }

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
            for cell in store.label_cells(houdini::taxonomy::TAXONOMY_VERSION).unwrap_or_default() {
                println!(
                    "  {:>5}  {:<12} {:<10} {} / {}  depth={} delegation={}",
                    cell.turns,
                    houdini::attribution::display_tool(&cell.tool),
                    cell.day,
                    cell.intent,
                    cell.domain,
                    cell.depth,
                    cell.delegation
                );
            }
        }
        Err(e) => {
            eprintln!("analytics failed: {e}");
            std::process::exit(1);
        }
    }
}
