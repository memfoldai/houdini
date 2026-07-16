//! The macOS app shell: `NSApplication` (Accessory) + status-bar tray + a
//! main-thread sampling timer that drives the `Monitor`.
//!
//! Ordering matters on macOS: a status-bar item must be created only AFTER the
//! run loop is running (tray-icon's documented requirement, to avoid fullscreen
//! glitches), so the tray and timer are created inside the app delegate's
//! `applicationDidFinishLaunching:`, not before `run()`. All of this lives on
//! the main thread; the shared runtime is a single-threaded `Rc` with interior
//! `RefCell`s — the timer fires serially, so there is never re-entrancy.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate};
use objc2_foundation::{MainThreadMarker, NSNotification, NSTimer};

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use ai_usage_monitor::config::{self, AppConfig, Paths};
use ai_usage_monitor::detector::DetectorConfig;
use ai_usage_monitor::export;
use ai_usage_monitor::monitor::{Monitor, MonitorState, TickClock};
use ai_usage_monitor::store::Store;

use crate::capture::{CaptureEngine, SweepLimits, SweepScope};
use crate::permissions;

/// Shared, main-thread-only runtime state. Every field the timer or a menu
/// action touches lives here behind a `RefCell`/`Cell`; nothing crosses threads.
struct Runtime {
    monitor: RefCell<Monitor>,
    capture: RefCell<CaptureEngine>,
    /// Second handle to the same connection as the monitor's, for export reads.
    store: Rc<Store>,
    install_id: String,
    export_dir: PathBuf,
    sample_interval_s: f64,
    /// Every Nth tick sweeps ALL windows (displays/Spaces/background); other
    /// ticks sample only the frontmost app. See config.
    full_sweep_every_ticks: u32,
    sweep_limits: SweepLimits,
    tick_count: Cell<u64>,
    /// Monotonic time base — detection timing must not jump with wall-clock
    /// changes; stored timestamps use the wall clock (see TickClock).
    start: Instant,
    /// The status-bar item, kept alive here (dropping it removes the icon).
    tray: RefCell<Option<TrayIcon>>,
    /// The repeating sampler, kept alive here (dropping it invalidates the timer).
    timer: RefCell<Option<Retained<NSTimer>>>,
    /// Last painted state, so the icon is only rebuilt on a transition.
    painted: Cell<Option<MonitorState>>,
    export_id: MenuId,
    quit_id: MenuId,
    /// Optional NER redactor for the export sweep (feature `ner`). Loaded once
    /// at startup; `None` when unconfigured or the model failed its self-test.
    #[cfg(feature = "ner")]
    ner: Option<ai_usage_monitor::ner::NerRedactor>,
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
}

/// Instance variables for the delegate: just the shared runtime handle.
struct Ivars {
    rt: Rc<Runtime>,
}

define_class!(
    // Accessory app delegate. Main-thread-only (delegates always are).
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
    let monitor = Monitor::new(
        store.clone(),
        cfg.salt.clone(),
        det,
        cfg.session_idle_gap_ms as i64,
        retention_ms,
    );

    let export_id = MenuId::new("export");
    let quit_id = MenuId::new("quit");

    Rc::new(Runtime {
        monitor: RefCell::new(monitor),
        capture: RefCell::new(CaptureEngine::new()),
        store,
        install_id: cfg.install_id.clone(),
        export_dir: paths.export_dir.clone(),
        sample_interval_s: cfg.sample_interval_ms as f64 / 1000.0,
        full_sweep_every_ticks: cfg.full_sweep_every_ticks.max(1),
        sweep_limits: SweepLimits {
            min_surface_area: cfg.min_surface_area,
            max_ocr: cfg.max_ocr_per_sweep,
        },
        tick_count: Cell::new(0),
        start: Instant::now(),
        tray: RefCell::new(None),
        timer: RefCell::new(None),
        painted: Cell::new(None),
        export_id,
        quit_id,
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
/// way; capture simply yields nothing until the grants land. Screen Recording
/// only takes effect after a relaunch (Apple's behavior), noted in VERIFICATION.
fn request_permissions() {
    if !permissions::accessibility_trusted() {
        permissions::accessibility_prompt();
    }
    if !permissions::screen_recording_granted() {
        permissions::screen_recording_request();
    }
}

fn install_tray(rt: &Rc<Runtime>) {
    let menu = Menu::new();
    let export_item = MenuItem::with_id(rt.export_id.clone(), "Export extract for review…", true, None);
    let quit_item = MenuItem::with_id(rt.quit_id.clone(), "Quit", true, None);
    menu.append(&export_item).expect("append export item");
    menu.append(&quit_item).expect("append quit item");

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(tooltip_for(MonitorState::Idle))
        .with_icon(icon_for(MonitorState::Idle))
        .build()
        .expect("build tray icon");

    *rt.tray.borrow_mut() = Some(tray);
    rt.painted.set(Some(MonitorState::Idle));
}

fn install_timer(rt: &Rc<Runtime>) {
    let rt_for_block = rt.clone();
    // The block runs on the main run loop, serially, so borrowing the runtime's
    // RefCells inside it can never overlap another tick.
    let block = RcBlock::new(move |_t: NonNull<NSTimer>| {
        tick(&rt_for_block);
    });
    let timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(rt.sample_interval_s, true, &block)
    };
    *rt.timer.borrow_mut() = Some(timer);
}

/// One sampling tick: sweep surfaces (frontmost app every tick; ALL windows on
/// every Nth tick), advance the monitor, repaint the icon on a state change,
/// and drain any pending menu clicks.
fn tick(rt: &Rc<Runtime>) {
    let clock = rt.clock();
    let n = rt.tick_count.get().wrapping_add(1);
    rt.tick_count.set(n);
    let scope = if n % rt.full_sweep_every_ticks as u64 == 0 {
        SweepScope::AllWindows
    } else {
        SweepScope::FrontmostApp
    };

    let samples = rt.capture.borrow_mut().sweep(scope, &rt.sweep_limits);
    match rt.monitor.borrow_mut().tick(clock, samples) {
        Ok(state) => repaint(rt, state),
        // A store failure must not kill the loop — log and keep sampling.
        Err(e) => log::error!("tick store error: {e}"),
    }

    drain_menu_events(rt);
}

/// Repaint the status item only when the state actually changed.
fn repaint(rt: &Rc<Runtime>, state: MonitorState) {
    if rt.painted.get() == Some(state) {
        return;
    }
    if let Some(tray) = rt.tray.borrow().as_ref() {
        let _ = tray.set_icon(Some(icon_for(state)));
        let _ = tray.set_tooltip(Some(tooltip_for(state)));
    }
    rt.painted.set(Some(state));
}

fn drain_menu_events(rt: &Rc<Runtime>) {
    while let Ok(ev) = MenuEvent::receiver().try_recv() {
        if ev.id == rt.export_id {
            do_export(rt);
        } else if ev.id == rt.quit_id {
            do_quit(rt);
        }
    }
}

/// Gate 2 of the two-gate export: write the (already-redacted) extract to a
/// timestamped file and point the person at it. Nothing is sent anywhere.
fn do_export(rt: &Rc<Runtime>) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string();
    match run_export(rt, &stamp) {
        Ok(path) => {
            log::info!("wrote extract for review: {}", path.display());
            if let Some(tray) = rt.tray.borrow().as_ref() {
                let msg = format!("Extract written — review before sharing:\n{}", path.display());
                let _ = tray.set_tooltip(Some(&msg));
            }
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

fn do_quit(rt: &Rc<Runtime>) {
    // Persist any in-flight session before exiting.
    if let Err(e) = rt.monitor.borrow_mut().shutdown(rt.clock()) {
        log::error!("shutdown store error: {e}");
    }
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        app.terminate(None);
    }
}

/// Tooltip text per state.
fn tooltip_for(state: MonitorState) -> &'static str {
    match state {
        MonitorState::Idle => "AI usage monitor — idle",
        MonitorState::Armed => "AI usage monitor — watching open windows",
        MonitorState::Capturing => "AI usage monitor — capturing an AI session",
    }
}

/// State color for the status-bar dot.
fn icon_for(state: MonitorState) -> Icon {
    let (r, g, b) = match state {
        MonitorState::Idle => (150, 150, 150),   // gray: nothing tracked
        MonitorState::Armed => (60, 120, 255),   // blue: watching, no AI yet
        MonitorState::Capturing => (40, 200, 90), // green: capturing now
    };
    render_dot(r, g, b)
}

/// A filled circle on a transparent background, as an RGBA `Icon`. Small and
/// dependency-free — the status bar scales it to fit.
fn render_dot(r: u8, g: u8, b: u8) -> Icon {
    const N: i32 = 18;
    let c = (N - 1) as f32 / 2.0;
    let radius = c; // touch the edges
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            let inside = dx * dx + dy * dy <= radius * radius;
            if inside {
                rgba.extend_from_slice(&[r, g, b, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Icon::from_rgba(rgba, N as u32, N as u32).expect("valid rgba icon")
}
