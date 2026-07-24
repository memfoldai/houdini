use std::cell::{Cell, RefCell};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate};
use objc2_foundation::{
    MainThreadMarker, NSActivityOptions, NSNotification, NSProcessInfo, NSString, NSTimer,
};

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{TrayIcon, TrayIconBuilder};

use houdini::config::{self, AppConfig, Paths};
use houdini::export;
use houdini::ingest::Ingestor;
use houdini::ingest_actions::ActionIngestor;
use houdini::store::{ActivityStats, Store, PAUSE_UNTIL_KEY};
use houdini::webingest;

use houdini::analytics::{Label, LabelRequest, Labeler, ProxyLabeler};
use houdini::analytics_job;

use crate::tray_glyph::{self, Glyph};
use crate::updater;

const BASE_TICK_S: f64 = 0.5;

const RECENT_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

const ACTIVE_WINDOW_MS: i64 = 6_000;

const HEARTBEAT_MS: i64 = 30_000;

const UPDATE_CHECK_MS: i64 = 6 * 60 * 60 * 1000;

const ANALYTICS_RETRY_MS: i64 = 15 * 60 * 1000;

/// A full batch means the LIMIT was hit and work remains, so the next batch
/// follows promptly instead of an hour later. Without this a backlog drains at
/// batch-per-hour: a week of history took five working days of uptime.
const ANALYTICS_DRAIN_MS: i64 = 60 * 1000;

const PAUSE_15M_MS: i64 = 15 * 60 * 1000;
const PAUSE_1H_MS: i64 = 60 * 60 * 1000;

#[derive(Clone, Copy)]
struct Clock {
    mono_ms: i64,
    wall_ms: i64,
}

struct Runtime {
    store: Rc<Store>,
    ingestor: RefCell<Ingestor>,
    action_ingestor: RefCell<ActionIngestor>,
    install_id: String,
    person: String,
    device_name: String,
    export_dir: PathBuf,

    transcript_poll_ms: i64,

    last_transcript_ms: Cell<i64>,
    heartbeat_at: Cell<i64>,
    transcripts_changed: Arc<AtomicBool>,
    #[allow(dead_code)]
    watcher: RefCell<Option<RecommendedWatcher>>,

    paused_until: Cell<Option<i64>>,
    start: Instant,

    web_rx: Receiver<Vec<u8>>,

    last_update_check: Cell<i64>,
    update_rx: RefCell<Option<Receiver<(bool, UpdateOutcome)>>>,
    update_notice_until: Cell<i64>,

    last_analytics: Cell<i64>,
    analytics_rx: RefCell<Option<Receiver<Vec<Result<Label, String>>>>>,
    analytics_batch: RefCell<Vec<LabelRequest>>,
    analytics_interval_ms: i64,
    analytics_batch_limit: i64,
    analytics_labeler: RefCell<Option<Arc<ProxyLabeler>>>,

    tray: RefCell<Option<TrayIcon>>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    _app_nap: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    painted: Cell<Option<Glyph>>,
    status_item: MenuItem,
    detail_item: MenuItem,
    analytics_item: MenuItem,
    resume_item: MenuItem,
    update_item: MenuItem,
    ids: MenuIds,
}

struct MenuIds {
    pause_15m: MenuId,
    pause_1h: MenuId,
    pause_indef: MenuId,
    resume: MenuId,
    show_data: MenuId,
    update: MenuId,
    quit: MenuId,
}

impl Runtime {
    fn clock(&self) -> Clock {
        Clock {
            mono_ms: self.start.elapsed().as_millis() as i64,
            wall_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        }
    }

    fn is_paused(&self) -> bool {
        self.paused_until.get().is_some()
    }
}

struct Ivars {
    rt: Rc<Runtime>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notif: &NSNotification) {
            let rt = self.ivars().rt.clone();
            crate::browserhost::ensure_installed();
            install_tray(&rt);
            install_timer(&rt);
            log::info!("houdini started (transcript ingest)");
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, rt: Rc<Runtime>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { rt });
        unsafe { msg_send![super(this), init] }
    }
}

pub fn run() {
    let mtm = MainThreadMarker::new().expect("must run on the main thread");

    let paths = Paths::resolve().expect("resolve paths");
    houdini::logging::init(&paths.log_file);
    let cfg = config::load_or_init(&paths.config_file).expect("load config");
    let rt = build_runtime(&paths, &cfg);

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let delegate = Delegate::new(mtm, rt);
    let proto = ProtocolObject::from_ref(&*delegate);
    app.setDelegate(Some(proto));

    app.run();
}

fn resolve_labeler(cfg: &AppConfig) -> Option<Arc<ProxyLabeler>> {
    if !cfg.analytics_enabled {
        log::info!("analytics: disabled by config");
        return None;
    }
    let Some(key) = crate::keychain::analytics_key() else {
        log::info!("analytics: no api key provisioned; labeling stays off");
        return None;
    };
    log::info!(
        "analytics: labeling with {} via {}",
        cfg.analytics_model,
        cfg.analytics_base_url
    );
    Some(Arc::new(ProxyLabeler::new(
        cfg.analytics_base_url.clone(),
        cfg.analytics_model.clone(),
        key,
    )))
}

fn build_runtime(paths: &Paths, cfg: &AppConfig) -> Rc<Runtime> {
    let key = crate::keychain::db_key().unwrap_or_else(|e| {
        log::error!("{e}");
        std::process::exit(1);
    });
    let store = Rc::new(Store::open(&paths.db_file, &key).unwrap_or_else(|e| {
        log::error!("cannot open the encrypted store: {e}");
        std::process::exit(1);
    }));

    let app_nap = {
        let reason = NSString::from_str("Recording AI usage");
        NSProcessInfo::processInfo().beginActivityWithOptions_reason(
            NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
            &reason,
        )
    };

    crate::loginitem::ensure_registered(&bundle_path());

    let labeler = resolve_labeler(cfg);
    match store.drop_superseded_labels(
        houdini::taxonomy::TAXONOMY_VERSION,
        houdini::analytics::PROMPT_VERSION,
    ) {
        Ok(0) => {}
        Ok(removed) => log::info!(
            "analytics: cleared {removed} label(s) from a superseded taxonomy; they re-analyze under the current one"
        ),
        Err(e) => log::warn!("analytics: could not clear superseded labels: {e}"),
    }

    let (web_tx, web_rx) = mpsc::channel();
    start_web_listener(paths.sock_file.clone(), web_tx);

    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let ingestor = Ingestor::new(home.clone(), now_ms);
    let action_ingestor = ActionIngestor::new(home.clone(), now_ms);
    let transcripts_changed = Arc::new(AtomicBool::new(false));
    let watcher = start_watcher(
        &home,
        &action_ingestor.watch_dirs(),
        transcripts_changed.clone(),
    );

    let ids = MenuIds {
        pause_15m: MenuId::new("pause_15m"),
        pause_1h: MenuId::new("pause_1h"),
        pause_indef: MenuId::new("pause_indef"),
        resume: MenuId::new("resume"),
        show_data: MenuId::new("show_data"),
        update: MenuId::new("update"),
        quit: MenuId::new("quit"),
    };

    let status_item = MenuItem::new("Starting…", false, None);
    let detail_item = MenuItem::new("", false, None);
    let analytics_item = MenuItem::new("", false, None);
    let resume_item = MenuItem::with_id(ids.resume.clone(), "Resume now", false, None);
    let update_item = MenuItem::with_id(ids.update.clone(), "Check for updates…", true, None);

    Rc::new(Runtime {
        store,
        ingestor: RefCell::new(ingestor),
        action_ingestor: RefCell::new(action_ingestor),
        install_id: cfg.install_id.clone(),
        person: cfg.person.clone(),
        device_name: cfg.device_name.clone(),
        export_dir: paths.export_dir.clone(),
        transcript_poll_ms: cfg.transcript_poll_ms as i64,
        last_transcript_ms: Cell::new(i64::MIN),
        heartbeat_at: Cell::new(0),
        transcripts_changed,
        watcher: RefCell::new(watcher),
        paused_until: Cell::new(None),
        start: Instant::now(),
        web_rx,
        last_update_check: Cell::new(i64::MIN),
        update_rx: RefCell::new(None),
        update_notice_until: Cell::new(0),
        last_analytics: Cell::new(i64::MIN),
        analytics_rx: RefCell::new(None),
        analytics_batch: RefCell::new(Vec::new()),
        analytics_interval_ms: cfg.analytics_interval_ms as i64,
        analytics_batch_limit: cfg.analytics_batch_limit,
        analytics_labeler: RefCell::new(labeler),
        tray: RefCell::new(None),
        timer: RefCell::new(None),
        _app_nap: app_nap,
        painted: Cell::new(None),
        status_item,
        detail_item,
        analytics_item,
        resume_item,
        update_item,
        ids,
    })
}

fn start_web_listener(sock: PathBuf, tx: Sender<Vec<u8>>) {
    let _ = std::fs::remove_file(&sock);
    let listener = match UnixListener::bind(&sock) {
        Ok(l) => l,
        Err(e) => {
            log::error!("web listener: cannot bind {}: {e}", sock.display());
            return;
        }
    };
    let _ = std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600));

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let tx = tx.clone();
            thread::spawn(move || {
                while let Some(bytes) = webingest::read_frame(&mut stream) {
                    if tx.send(bytes).is_err() {
                        return;
                    }
                }
            });
        }
    });
}

fn start_watcher(
    home: &Path,
    extra_dirs: &[PathBuf],
    changed: Arc<AtomicBool>,
) -> Option<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            changed.store(true, Ordering::Relaxed);
        }
    })
    .ok()?;
    let mut paths: Vec<PathBuf> = [
        ".claude/projects",
        ".codex",
        ".openclaw",
        ".openclaw-user",
        ".openclaw-dev",
    ]
    .iter()
    .map(|d| home.join(d))
    .collect();
    paths.extend(extra_dirs.iter().cloned());
    paths.sort();
    paths.dedup();

    let mut watched = 0;
    for path in paths {
        if path.exists() && watcher.watch(&path, RecursiveMode::Recursive).is_ok() {
            watched += 1;
        }
    }
    (watched > 0).then_some(watcher)
}

fn install_tray(rt: &Rc<Runtime>) {
    let menu = Menu::new();

    let pause = Submenu::new("Take a break", true);
    pause
        .append(&MenuItem::with_id(
            rt.ids.pause_15m.clone(),
            "For 15 minutes",
            true,
            None,
        ))
        .and(pause.append(&MenuItem::with_id(
            rt.ids.pause_1h.clone(),
            "For an hour",
            true,
            None,
        )))
        .and(pause.append(&MenuItem::with_id(
            rt.ids.pause_indef.clone(),
            "Until I'm back",
            true,
            None,
        )))
        .expect("build pause submenu");

    let show_data = MenuItem::with_id(rt.ids.show_data.clone(), "Export my data…", true, None);
    let quit = MenuItem::with_id(rt.ids.quit.clone(), "Quit", true, None);

    let title = MenuItem::new(concat!("Houdini ", env!("CARGO_PKG_VERSION")), false, None);

    menu.append(&title).expect("title");
    menu.append(&PredefinedMenuItem::separator()).expect("sep0");
    menu.append(&rt.status_item).expect("status");
    menu.append(&rt.detail_item).expect("detail");
    menu.append(&rt.analytics_item).expect("analytics");
    menu.append(&PredefinedMenuItem::separator()).expect("sep1");
    menu.append(&pause).expect("pause");
    menu.append(&rt.resume_item).expect("resume");
    menu.append(&PredefinedMenuItem::separator()).expect("sep2");
    menu.append(&show_data).expect("show_data");
    menu.append(&rt.update_item).expect("update");
    menu.append(&quit).expect("quit");

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(tray_glyph::icon(Glyph::Idle))
        .with_icon_as_template(true)
        .build()
        .expect("build tray icon");
    *rt.tray.borrow_mut() = Some(tray);
    let clock = rt.clock();
    let stats = rt
        .store
        .activity_stats(clock.wall_ms - RECENT_WINDOW_MS)
        .unwrap_or_default();
    paint(rt, Glyph::Idle);
    refresh_menu(rt, Glyph::Idle, clock.wall_ms, &stats);
}

fn install_timer(rt: &Rc<Runtime>) {
    let rt_for_block = rt.clone();
    let block = RcBlock::new(move |_t: NonNull<NSTimer>| tick(&rt_for_block));
    let timer =
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(BASE_TICK_S, true, &block) };
    *rt.timer.borrow_mut() = Some(timer);
}

fn tick(rt: &Rc<Runtime>) {
    let clock = rt.clock();

    if let Some(until) = rt.paused_until.get() {
        if clock.mono_ms >= until {
            rt.paused_until.set(None);
            let _ = rt.store.set_setting(PAUSE_UNTIL_KEY, "0");
            log::info!("auto-resumed after timed pause");
        }
    }

    if !rt.is_paused() {
        let changed = rt.transcripts_changed.swap(false, Ordering::Relaxed);
        let due_poll =
            clock.mono_ms.saturating_sub(rt.last_transcript_ms.get()) >= rt.transcript_poll_ms;
        if changed || due_poll {
            rt.last_transcript_ms.set(clock.mono_ms);
            let stats = rt.ingestor.borrow_mut().poll(&rt.store);
            if stats.new_turns > 0 {
                log::info!(
                    "ingested {} new message(s) across {} session(s)",
                    stats.new_turns,
                    stats.sessions
                );
            }
            let acted = rt.action_ingestor.borrow_mut().poll(&rt.store);
            if acted > 0 {
                log::info!("attributed {acted} new agent action(s)");
            }
        }
        if due(&rt.heartbeat_at, clock.mono_ms, HEARTBEAT_MS) {
            log::info!("heartbeat: watching for new AI messages");
        }
    }

    drain_web_messages(rt);

    if !rt.is_paused() && due(&rt.last_analytics, clock.mono_ms, rt.analytics_interval_ms) {
        spawn_analytics(rt);
    }
    poll_analytics(rt, clock.mono_ms, clock.wall_ms);

    if due(&rt.last_update_check, clock.mono_ms, UPDATE_CHECK_MS) {
        spawn_update_check(rt, false);
    }
    poll_update_check(rt, clock.mono_ms);

    let stats = rt
        .store
        .activity_stats(clock.wall_ms - RECENT_WINDOW_MS)
        .unwrap_or_default();
    let glyph = glyph_for(rt, clock.wall_ms, &stats);
    paint(rt, glyph);
    refresh_menu(rt, glyph, clock.wall_ms, &stats);
    drain_menu_events(rt);
}

enum UpdateOutcome {
    UpToDate,
    Staged(PathBuf),
    Failed(String),
}

fn drain_web_messages(rt: &Rc<Runtime>) {
    while let Ok(bytes) = rt.web_rx.try_recv() {
        match webingest::ingest(&rt.store, &bytes) {
            Ok(0) => {}
            Ok(n) => log::info!("web: stored {n} chat turn(s)"),
            Err(e) => log::warn!("web: dropped a message: {e}"),
        }
    }
}

/// The .app bundle this process is running from, derived from the executable
/// path rather than assumed, so a relocated or renamed install still resolves.
fn bundle_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
        .unwrap_or(exe)
}

fn analytics_progress_text(rt: &Rc<Runtime>) -> String {
    if rt.analytics_labeler.borrow().is_none() {
        return String::new();
    }
    let Ok((total, labeled)) = rt.store.label_progress(
        houdini::taxonomy::TAXONOMY_VERSION,
        houdini::analytics::PROMPT_VERSION,
    ) else {
        return String::new();
    };
    if total == 0 {
        return String::new();
    }
    if labeled >= total {
        format!("Analytics: all {total} requests analyzed")
    } else {
        format!(
            "Analytics: {}% analyzed ({labeled} of {total})",
            labeled * 100 / total
        )
    }
}

fn spawn_analytics(rt: &Rc<Runtime>) {
    let Some(labeler) = rt.analytics_labeler.borrow().clone() else {
        return;
    };
    if rt.analytics_rx.borrow().is_some() {
        return;
    }
    let requests = match analytics_job::collect(&rt.store, rt.analytics_batch_limit) {
        Ok(requests) => requests,
        Err(e) => {
            log::warn!("analytics: could not read pending turns: {e}");
            return;
        }
    };
    if requests.is_empty() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    let batch = requests.clone();
    thread::spawn(move || {
        let _ = tx.send(analytics_job::label_batch(labeler.as_ref(), &batch));
    });
    *rt.analytics_batch.borrow_mut() = requests;
    *rt.analytics_rx.borrow_mut() = Some(rx);
}

fn poll_analytics(rt: &Rc<Runtime>, now_mono_ms: i64, now_wall_ms: i64) {
    let received = {
        let rx = rt.analytics_rx.borrow();
        rx.as_ref().and_then(|rx| rx.try_recv().ok())
    };
    let Some(results) = received else {
        return;
    };
    *rt.analytics_rx.borrow_mut() = None;
    rt.analytics_batch.borrow_mut().clear();

    let model = rt
        .analytics_labeler
        .borrow()
        .as_ref()
        .map(|l| l.model().to_string())
        .unwrap_or_default();
    match analytics_job::persist(&rt.store, &model, &results, now_wall_ms) {
        Ok(report) => {
            log::info!(
                "analytics: labeled {} of {} turn(s), {} failed, {} candidate(s)",
                report.labeled,
                report.considered,
                report.failed,
                report.candidates
            );
            let backlog_remains = report.considered as i64 >= rt.analytics_batch_limit;
            let next_in_ms = if report.failed > 0 && report.labeled == 0 {
                Some(ANALYTICS_RETRY_MS)
            } else if backlog_remains {
                Some(ANALYTICS_DRAIN_MS)
            } else {
                None
            };
            if let Some(next_in_ms) = next_in_ms {
                rt.last_analytics
                    .set(now_mono_ms.saturating_sub(rt.analytics_interval_ms - next_in_ms));
            }
        }
        Err(e) => log::warn!("analytics: could not store labels: {e}"),
    }
}

fn spawn_update_check(rt: &Rc<Runtime>, manual: bool) {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let outcome = match updater::check() {
            None => UpdateOutcome::UpToDate,
            Some(update) => match updater::download_and_stage(&update) {
                Ok(bundle) => UpdateOutcome::Staged(bundle),
                Err(e) => UpdateOutcome::Failed(e),
            },
        };
        let _ = tx.send((manual, outcome));
    });
    *rt.update_rx.borrow_mut() = Some(rx);
}

const UPDATE_DEFAULT: &str = "Check for updates…";
const UPDATE_NOTICE_MS: i64 = 4_000;

fn poll_update_check(rt: &Rc<Runtime>, now_mono_ms: i64) {
    let received = {
        let rx = rt.update_rx.borrow();
        rx.as_ref().and_then(|rx| rx.try_recv().ok())
    };
    if let Some((manual, outcome)) = received {
        *rt.update_rx.borrow_mut() = None;
        match outcome {
            UpdateOutcome::Staged(bundle) => relaunch_and_quit(&bundle),
            UpdateOutcome::UpToDate if manual => {
                set_update_notice(rt, "You're on the latest version", now_mono_ms)
            }
            UpdateOutcome::Failed(e) => {
                log::warn!("update check/install failed: {e}");
                if manual {
                    set_update_notice(rt, "Update check failed — see the log", now_mono_ms);
                }
            }
            UpdateOutcome::UpToDate => {}
        }
    }

    let until = rt.update_notice_until.get();
    if until != 0 && now_mono_ms >= until {
        rt.update_notice_until.set(0);
        rt.update_item.set_text(UPDATE_DEFAULT);
    }
}

fn relaunch_and_quit(bundle: &Path) {
    let _ = Command::new("open").arg("-n").arg(bundle).spawn();
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).terminate(None);
    }
}

fn set_update_notice(rt: &Rc<Runtime>, text: &str, now_mono_ms: i64) {
    rt.update_item.set_text(text);
    rt.update_notice_until.set(now_mono_ms + UPDATE_NOTICE_MS);
}

fn do_update(rt: &Rc<Runtime>) {
    rt.update_notice_until.set(0);
    rt.update_item.set_text("Checking for updates…");
    spawn_update_check(rt, true);
}

fn due(last: &Cell<i64>, now_mono_ms: i64, interval_ms: i64) -> bool {
    if now_mono_ms.saturating_sub(last.get()) >= interval_ms {
        last.set(now_mono_ms);
        true
    } else {
        false
    }
}

fn glyph_for(rt: &Rc<Runtime>, wall_now_ms: i64, stats: &ActivityStats) -> Glyph {
    if rt.is_paused() {
        return Glyph::Paused;
    }
    match stats.last_activity_ms {
        Some(t) if wall_now_ms.saturating_sub(t) < ACTIVE_WINDOW_MS => Glyph::Active,
        _ => Glyph::Idle,
    }
}

fn paint(rt: &Rc<Runtime>, glyph: Glyph) {
    if rt.painted.get() == Some(glyph) {
        return;
    }
    if let Some(tray) = rt.tray.borrow().as_ref() {
        let _ = tray.set_icon_with_as_template(Some(tray_glyph::icon(glyph)), true);
        let _ = tray.set_tooltip(Some(tooltip_for(glyph)));
    }
    rt.painted.set(Some(glyph));
}

fn refresh_menu(rt: &Rc<Runtime>, glyph: Glyph, now_ms: i64, stats: &ActivityStats) {
    let status = match glyph {
        Glyph::Paused => "Paused",
        Glyph::Active => "Recording AI activity",
        Glyph::Idle => "Watching for AI use",
    };
    rt.status_item.set_text(status);

    if stats.recent_interactions == 0 && stats.recent_actions == 0 {
        rt.detail_item.set_text("Nothing recorded yet today");
    } else {
        rt.detail_item.set_text(format!(
            "{} AI session{} · {} action{} today · last {}",
            stats.recent_interactions,
            if stats.recent_interactions == 1 {
                ""
            } else {
                "s"
            },
            stats.recent_actions,
            if stats.recent_actions == 1 { "" } else { "s" },
            relative_time(stats.last_activity_ms, now_ms)
        ));
    }
    rt.analytics_item
        .set_text(analytics_progress_text(rt));

    if let Some(summary) = houdini::summary::format_action_summary(
        &rt.store
            .action_stats(now_ms - RECENT_WINDOW_MS)
            .unwrap_or_default(),
    ) {
        rt.detail_item.set_text(summary);
    }

    rt.resume_item.set_enabled(rt.is_paused());
}

fn drain_menu_events(rt: &Rc<Runtime>) {
    while let Ok(ev) = MenuEvent::receiver().try_recv() {
        let id = &ev.id;
        if id == &rt.ids.show_data {
            do_show_data(rt);

        } else if id == &rt.ids.update {
            do_update(rt);
        } else if id == &rt.ids.quit {
            do_quit();
        } else if id == &rt.ids.resume {
            set_pause(rt, None, "resumed by user");
        } else if id == &rt.ids.pause_15m {
            set_pause(rt, Some(rt.clock().mono_ms + PAUSE_15M_MS), "paused 15m");
        } else if id == &rt.ids.pause_1h {
            set_pause(rt, Some(rt.clock().mono_ms + PAUSE_1H_MS), "paused 1h");
        } else if id == &rt.ids.pause_indef {
            set_pause(rt, Some(i64::MAX), "paused until resume");
        }
    }
}

fn set_pause(rt: &Rc<Runtime>, until: Option<i64>, why: &str) {
    let clock = rt.clock();
    rt.paused_until.set(until);

    let deadline_wall = match until {
        None => 0,
        Some(i64::MAX) => i64::MAX,
        Some(mono) => clock.wall_ms + (mono - clock.mono_ms).max(0),
    };
    let _ = rt
        .store
        .set_setting(PAUSE_UNTIL_KEY, &deadline_wall.to_string());
    log::info!("{why}");

    let glyph = if until.is_some() {
        Glyph::Paused
    } else {
        Glyph::Idle
    };
    let stats = rt
        .store
        .activity_stats(clock.wall_ms - RECENT_WINDOW_MS)
        .unwrap_or_default();
    paint(rt, glyph);
    refresh_menu(rt, glyph, clock.wall_ms, &stats);
}

fn do_show_data(rt: &Rc<Runtime>) {
    let identity = export::ExportIdentity {
        install_id: &rt.install_id,
        person: &rt.person,
        device_name: &rt.device_name,
    };
    match export::export_snapshot(&rt.store, &identity, &rt.export_dir) {
        Ok(path) => {
            let _ = Command::new("open").arg("-R").arg(&path).spawn();
        }
        Err(e) => {
            log::error!("export failed: {e}");
            let _ = Command::new("open")
                .arg(export::data_dir_path(&rt.export_dir))
                .spawn();
        }
    }
}

fn do_quit() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).terminate(None);
    }
}

fn tooltip_for(glyph: Glyph) -> &'static str {
    match glyph {
        Glyph::Paused => "Houdini — taking a break",
        Glyph::Active => "Houdini — recording AI activity",
        Glyph::Idle => "Houdini — watching for AI use",
    }
}

fn relative_time(then_ms: Option<i64>, now_ms: i64) -> String {
    let Some(then) = then_ms else {
        return "never".to_string();
    };
    let secs = (now_ms - then).max(0) / 1000;
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}
