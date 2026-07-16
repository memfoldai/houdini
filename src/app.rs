//! The macOS app shell: `NSApplication` (Accessory) + status-bar menu + a
//! main-thread sampling timer that drives the `Monitor`.
//!
//! Ordering matters on macOS: a status-bar item must be created only AFTER the
//! run loop is running (tray-icon's documented requirement, to avoid fullscreen
//! glitches), so the menu and timer are created inside the app delegate's
//! `applicationDidFinishLaunching:`, not before `run()`. All of this lives on
//! the main thread; the shared runtime is a single-threaded `Rc` with interior
//! `RefCell`s — the timer fires serially, so there is never re-entrancy.
//!
//! Pause is GLOBAL, not per-window: the user's intent ("don't record me right
//! now, I'm typing something sensitive") is about their attention, which can be
//! in any window, so a single switch is both safer and simpler than per-app
//! bookkeeping the user would have to track. While paused, no capture runs at
//! all (the sweep is skipped), so nothing is stored and CPU drops to idle.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::process::Command;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate};
use objc2_foundation::{MainThreadMarker, NSNotification, NSTimer};

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{TrayIcon, TrayIconBuilder};

use ai_usage_monitor::config::{self, AppConfig, Paths};
use ai_usage_monitor::detector::DetectorConfig;
use ai_usage_monitor::export;
use ai_usage_monitor::monitor::{Monitor, MonitorState, TickClock};
use ai_usage_monitor::store::Store;

use crate::capture::{CaptureEngine, SweepLimits, SweepScope};
use crate::permissions;
use crate::tray_glyph::{self, Glyph};

/// Refresh the menu text on state change and on this slow cadence, so an open
/// menu reads fresh without churning at the full sample rate.
const MENU_REFRESH_EVERY_TICKS: u64 = 3;
/// Window for the "captured recently" status count.
const RECENT_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

/// Timed-pause durations (monotonic ms). Indefinite uses an i64::MAX sentinel.
const PAUSE_15M_MS: i64 = 15 * 60 * 1000;
const PAUSE_1H_MS: i64 = 60 * 60 * 1000;

/// Shared, main-thread-only runtime state. Every field the timer or a menu
/// action touches lives here behind a `RefCell`/`Cell`; nothing crosses threads.
struct Runtime {
    monitor: RefCell<Monitor>,
    capture: RefCell<CaptureEngine>,
    /// Second handle to the same connection as the monitor's, for export reads.
    store: Rc<Store>,
    install_id: String,
    export_dir: PathBuf,
    log_file: PathBuf,
    sample_interval_s: f64,
    /// Every Nth tick sweeps ALL windows; other ticks sample only the frontmost.
    full_sweep_every_ticks: u32,
    sweep_limits: SweepLimits,
    tick_count: Cell<u64>,
    /// Last INFO heartbeat (monotonic ms) — a periodic, content-free "is it
    /// working" line in the activity log so the user can tell capture apart from
    /// detection without turning on debug logging.
    heartbeat_at: Cell<i64>,
    /// `None` = active; `Some(t)` = paused until monotonic ms `t` (i64::MAX =
    /// until the user resumes).
    paused_until: Cell<Option<i64>>,
    /// Monotonic time base — detection timing must not jump with wall-clock
    /// changes; stored timestamps use the wall clock (see TickClock).
    start: Instant,
    /// Kept alive here (dropping the tray removes the icon; the timer, its slot).
    tray: RefCell<Option<TrayIcon>>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    /// Last painted glyph, so the icon/title are only rebuilt on a transition.
    painted: Cell<Option<Glyph>>,
    /// Menu rows updated as state changes (held so the tick loop can retext them).
    status_item: MenuItem,
    detail_item: MenuItem,
    resume_item: MenuItem,
    ids: MenuIds,
    /// Optional NER redactor for the export sweep (feature `ner`).
    #[cfg(feature = "ner")]
    ner: Option<ai_usage_monitor::ner::NerRedactor>,
}

/// Command menu-item ids, matched in the event drain.
struct MenuIds {
    pause_15m: MenuId,
    pause_1h: MenuId,
    pause_indef: MenuId,
    resume: MenuId,
    export: MenuId,
    open_log: MenuId,
    quit: MenuId,
}

impl Runtime {
    fn clock(&self) -> TickClock {
        TickClock {
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
        // Fires once the run loop is up — the only safe point to create the
        // status item and start the sampler.
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notif: &NSNotification) {
            let rt = self.ivars().rt.clone();
            request_permissions();
            install_tray(&rt);
            install_timer(&rt);
            log::info!("ai-usage-monitor started (menu-bar, accessory)");
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, rt: Rc<Runtime>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { rt });
        unsafe { msg_send![super(this), init] }
    }
}

/// Build the runtime, wire up the delegate, and run the app. Blocks until quit.
pub fn run() {
    let mtm = MainThreadMarker::new().expect("must run on the main thread");

    let paths = Paths::resolve().expect("resolve paths");
    ai_usage_monitor::logging::init(&paths.log_file);
    let cfg = config::load_or_init(&paths.config_file).expect("load config");
    let rt = build_runtime(&paths, &cfg);

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let delegate = Delegate::new(mtm, rt);
    let proto = ProtocolObject::from_ref(&*delegate);
    app.setDelegate(Some(proto));

    app.run();
}

fn build_runtime(paths: &Paths, cfg: &AppConfig) -> Rc<Runtime> {
    let store = Rc::new(Store::open(&paths.db_file).expect("open store"));
    let det = DetectorConfig::from(&cfg.detector);
    // Background surfaces are only observed on full sweeps, so a surface is
    // "gone" only after several sweeps have missed it — plus the idle gap so a
    // still-streaming surface is never dropped mid-session by a slow sweep.
    let sweep_period_ms = cfg.sample_interval_ms * cfg.full_sweep_every_ticks as u64;
    let retention_ms = (3 * sweep_period_ms + cfg.session_idle_gap_ms) as i64;
    let monitor =
        Monitor::new(store.clone(), cfg.salt.clone(), det, cfg.session_idle_gap_ms as i64, retention_ms);

    let ids = MenuIds {
        pause_15m: MenuId::new("pause_15m"),
        pause_1h: MenuId::new("pause_1h"),
        pause_indef: MenuId::new("pause_indef"),
        resume: MenuId::new("resume"),
        export: MenuId::new("export"),
        open_log: MenuId::new("open_log"),
        quit: MenuId::new("quit"),
    };

    // Disabled info rows + the resume toggle (created on the main thread, before
    // the run loop). Text is filled on the first tick.
    let status_item = MenuItem::new("Starting…", false, None);
    let detail_item = MenuItem::new("", false, None);
    let resume_item = MenuItem::with_id(ids.resume.clone(), "Resume now", false, None);

    Rc::new(Runtime {
        monitor: RefCell::new(monitor),
        capture: RefCell::new(CaptureEngine::new()),
        store,
        install_id: cfg.install_id.clone(),
        export_dir: paths.export_dir.clone(),
        log_file: paths.log_file.clone(),
        sample_interval_s: cfg.sample_interval_ms as f64 / 1000.0,
        full_sweep_every_ticks: cfg.full_sweep_every_ticks.max(1),
        sweep_limits: SweepLimits {
            min_surface_area: cfg.min_surface_area,
            max_ocr: cfg.max_ocr_per_sweep,
            ocr_min_interval_ms: cfg.ocr_min_interval_ms as i64,
        },
        tick_count: Cell::new(0),
        heartbeat_at: Cell::new(0),
        paused_until: Cell::new(None),
        start: Instant::now(),
        tray: RefCell::new(None),
        timer: RefCell::new(None),
        painted: Cell::new(None),
        status_item,
        detail_item,
        resume_item,
        ids,
        #[cfg(feature = "ner")]
        ner: load_ner(cfg),
    })
}

/// Load the NER redactor if a model dir is configured. A configured-but-broken
/// model logs and yields `None` (deterministic redaction still guards the
/// extract); a healthy model passes its seeded self-test inside `load`.
#[cfg(feature = "ner")]
fn load_ner(cfg: &AppConfig) -> Option<ai_usage_monitor::ner::NerRedactor> {
    let dir = cfg.ner_model_dir.as_ref()?;
    match ai_usage_monitor::ner::NerRedactor::load(dir) {
        Ok(r) => {
            log::info!("NER redaction layer loaded from {}", dir.display());
            Some(r)
        }
        Err(e) => {
            log::error!("NER model configured but not loaded ({e}); export uses the deterministic layer only");
            None
        }
    }
}

/// Prompt (once) for whichever grant is missing. The app keeps running either
/// way; capture yields nothing until the grants land. Screen Recording only
/// takes effect after a relaunch (Apple's behavior), noted in VERIFICATION.
fn request_permissions() {
    let ax = permissions::accessibility_trusted();
    let sr = permissions::screen_recording_granted();
    log::info!("permissions at launch: accessibility={ax} screen_recording={sr}");
    if !ax {
        permissions::accessibility_prompt();
    }
    if !sr {
        permissions::screen_recording_request();
    }
}

fn install_tray(rt: &Rc<Runtime>) {
    let menu = Menu::new();

    let pause = Submenu::new("Pause watching", true);
    pause
        .append(&MenuItem::with_id(rt.ids.pause_15m.clone(), "For 15 minutes", true, None))
        .and(pause.append(&MenuItem::with_id(rt.ids.pause_1h.clone(), "For 1 hour", true, None)))
        .and(pause.append(&MenuItem::with_id(rt.ids.pause_indef.clone(), "Until I resume", true, None)))
        .expect("build pause submenu");

    let export = MenuItem::with_id(rt.ids.export.clone(), "Export for review…", true, None);
    let open_log = MenuItem::with_id(rt.ids.open_log.clone(), "Open activity log", true, None);
    let quit = MenuItem::with_id(rt.ids.quit.clone(), "Quit", true, None);

    menu.append(&rt.status_item).expect("status");
    menu.append(&rt.detail_item).expect("detail");
    menu.append(&PredefinedMenuItem::separator()).expect("sep1");
    menu.append(&pause).expect("pause");
    menu.append(&rt.resume_item).expect("resume");
    menu.append(&PredefinedMenuItem::separator()).expect("sep2");
    menu.append(&export).expect("export");
    menu.append(&open_log).expect("open_log");
    menu.append(&quit).expect("quit");

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(tray_glyph::icon(Glyph::Idle))
        .with_icon_as_template(true) // monochrome template — macOS tints it
        .build()
        .expect("build tray icon");
    *rt.tray.borrow_mut() = Some(tray);
    // First paint + text.
    let clock = rt.clock();
    paint(rt, Glyph::Idle);
    refresh_menu(rt, Glyph::Idle, clock.wall_ms);
}

fn install_timer(rt: &Rc<Runtime>) {
    let rt_for_block = rt.clone();
    // The block runs on the main run loop, serially, so borrowing the runtime's
    // RefCells inside it can never overlap another tick.
    let block = RcBlock::new(move |_t: NonNull<NSTimer>| tick(&rt_for_block));
    let timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(rt.sample_interval_s, true, &block)
    };
    *rt.timer.borrow_mut() = Some(timer);
}

/// One sampling tick: honor pause, sweep surfaces, advance the monitor, repaint
/// on change, and drain menu clicks.
fn tick(rt: &Rc<Runtime>) {
    let clock = rt.clock();

    // Auto-resume a timed pause the moment it lapses.
    if let Some(until) = rt.paused_until.get() {
        if clock.mono_ms >= until {
            rt.paused_until.set(None);
            log::info!("auto-resumed after timed pause");
        }
    }

    // Paused: capture nothing (no sweep, no CPU), just keep the UI live.
    if rt.is_paused() {
        paint(rt, Glyph::Paused);
        refresh_menu(rt, Glyph::Paused, clock.wall_ms);
        drain_menu_events(rt);
        return;
    }

    let n = rt.tick_count.get().wrapping_add(1);
    rt.tick_count.set(n);
    let scope = if n % rt.full_sweep_every_ticks as u64 == 0 {
        SweepScope::AllWindows
    } else {
        SweepScope::FrontmostApp
    };

    let samples = rt.capture.borrow_mut().sweep(clock.mono_ms, scope, &rt.sweep_limits);

    // Heartbeat (~every 30s, INFO): how many windows are being read and the most
    // text seen this tick. If reads=0, capture/permissions are the problem; if
    // reads>0 but no sessions appear, it's detection — this splits the two
    // without the user needing RUST_LOG=debug.
    if clock.mono_ms - rt.heartbeat_at.get() > 30_000 {
        rt.heartbeat_at.set(clock.mono_ms);
        let peak = samples
            .iter()
            .map(|s| ai_usage_monitor::detector::prose_len(&s.output_text))
            .max()
            .unwrap_or(0);
        log::info!(
            "heartbeat: reading {} window(s) this tick (most prose seen: {} chars); \
             open ChatGPT/Claude and send a message to test",
            samples.len(),
            peak
        );
    }

    let state = match rt.monitor.borrow_mut().tick(clock, samples) {
        Ok(state) => state,
        // A store failure must not kill the loop — log and keep sampling.
        Err(e) => {
            log::error!("tick store error: {e}");
            drain_menu_events(rt);
            return;
        }
    };

    let glyph = glyph_for(state);
    let changed = rt.painted.get() != Some(glyph);
    paint(rt, glyph);
    if changed || n % MENU_REFRESH_EVERY_TICKS == 0 {
        refresh_menu(rt, glyph, clock.wall_ms);
    }
    drain_menu_events(rt);
}

fn glyph_for(state: MonitorState) -> Glyph {
    match state {
        MonitorState::Idle => Glyph::Idle,
        MonitorState::Armed => Glyph::Watching,
        MonitorState::Capturing => Glyph::Capturing,
    }
}

/// Repaint the icon (and the transient menu-bar label) only on a state change.
fn paint(rt: &Rc<Runtime>, glyph: Glyph) {
    if rt.painted.get() == Some(glyph) {
        return;
    }
    if let Some(tray) = rt.tray.borrow().as_ref() {
        // Keep the template flag on every swap, else macOS stops tinting it.
        let _ = tray.set_icon_with_as_template(Some(tray_glyph::icon(glyph)), true);
        let _ = tray.set_tooltip(Some(tooltip_for(glyph)));
        // A short label appears next to the icon ONLY while actively recording —
        // transparent feedback exactly when it matters, no persistent clutter.
        let title = if glyph == Glyph::Capturing { Some(" Recording") } else { None };
        tray.set_title(title);
    }
    rt.painted.set(Some(glyph));
}

/// Update the status/detail rows + the Resume toggle so the menu reads in plain
/// language and confirms it is working.
fn refresh_menu(rt: &Rc<Runtime>, glyph: Glyph, now_ms: i64) {
    let status = if !permissions::screen_recording_granted() {
        "Turn on Screen Recording in System Settings".to_string()
    } else if !permissions::accessibility_trusted() {
        "Turn on Accessibility in System Settings".to_string()
    } else {
        match glyph {
            Glyph::Paused => "Paused — not watching".to_string(),
            Glyph::Idle => "No AI activity right now".to_string(),
            Glyph::Watching => "Watching for AI use".to_string(),
            Glyph::Capturing => "Recording an AI chat".to_string(),
        }
    };
    rt.status_item.set_text(status);

    match rt.store.session_stats(now_ms - RECENT_WINDOW_MS) {
        Ok(stats) if stats.recent == 0 && stats.last_capture_ms.is_none() => {
            rt.detail_item.set_text("Nothing captured yet");
        }
        Ok(stats) => rt.detail_item.set_text(format!(
            "{} captured (24h) · last {}",
            stats.recent,
            relative_time(stats.last_capture_ms, now_ms)
        )),
        Err(e) => log::error!("status stats error: {e}"),
    }

    rt.resume_item.set_enabled(rt.is_paused());
}

fn drain_menu_events(rt: &Rc<Runtime>) {
    while let Ok(ev) = MenuEvent::receiver().try_recv() {
        let id = &ev.id;
        if id == &rt.ids.export {
            do_export(rt);
        } else if id == &rt.ids.open_log {
            open_log(rt);
        } else if id == &rt.ids.quit {
            do_quit(rt);
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

/// Apply a pause/resume. On pause, close any in-flight session cleanly so a
/// half-captured chat is finalized rather than left dangling.
fn set_pause(rt: &Rc<Runtime>, until: Option<i64>, why: &str) {
    if until.is_some() {
        if let Err(e) = rt.monitor.borrow_mut().shutdown(rt.clock()) {
            log::error!("pause shutdown error: {e}");
        }
    }
    rt.paused_until.set(until);
    log::info!("{why}");
    // Reflect immediately without waiting for the next tick.
    let glyph = if until.is_some() { Glyph::Paused } else { Glyph::Idle };
    paint(rt, glyph);
    refresh_menu(rt, glyph, rt.clock().wall_ms);
}

/// Gate 2 of the two-gate export: write the (already-redacted) extract and point
/// the person at it. Nothing is sent anywhere.
fn do_export(rt: &Rc<Runtime>) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string();
    match run_export(rt, &stamp) {
        Ok(path) => {
            log::info!("wrote extract for review: {}", path.display());
            // Reveal it so the person can review before sharing.
            let _ = Command::new("open").arg("-R").arg(&path).spawn();
        }
        Err(e) => log::error!("export failed: {e}"),
    }
}

/// Write the extract, applying the NER sweep when a redactor is loaded.
fn run_export(rt: &Rc<Runtime>, stamp: &str) -> std::io::Result<PathBuf> {
    #[cfg(feature = "ner")]
    if let Some(ner) = rt.ner.as_ref() {
        return export::export_all_ner(&rt.store, &rt.install_id, &rt.export_dir, stamp, ner);
    }
    export::export_all(&rt.store, &rt.install_id, &rt.export_dir, stamp)
}

fn open_log(rt: &Rc<Runtime>) {
    let _ = Command::new("open").arg(&rt.log_file).spawn();
}

fn do_quit(rt: &Rc<Runtime>) {
    // Persist any in-flight session before exiting.
    if let Err(e) = rt.monitor.borrow_mut().shutdown(rt.clock()) {
        log::error!("shutdown store error: {e}");
    }
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).terminate(None);
    }
}

/// Hover tooltip per glyph.
fn tooltip_for(glyph: Glyph) -> &'static str {
    match glyph {
        Glyph::Paused => "AI Usage Monitor — paused",
        Glyph::Idle => "AI Usage Monitor — no AI activity",
        Glyph::Watching => "AI Usage Monitor — watching for AI use",
        Glyph::Capturing => "AI Usage Monitor — recording an AI chat",
    }
}

/// Human "N ago" for a past unix-ms instant, or "never".
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
