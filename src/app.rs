//! The macOS app shell: `NSApplication` (Accessory) + status-bar menu + a
//! main-thread timer that drives the two detectors.
//!
//! There is no screen capture and no TCC permission here anymore. The timer, on
//! the main run loop (serially, so the shared `Rc<Runtime>` never re-enters),
//! does three cheap things on their own cadences: scan tool transcripts for new
//! interactions (Layer A), poll the process table for AI network connections
//! (Layer B), and flush finished records to day files. Everything is read-only
//! observation of files and sockets the user already owns.
//!
//! Pause is GLOBAL: while paused, neither detector runs, so nothing new is
//! recorded — the switch protects whatever the user is doing, in any app.

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
use ai_usage_monitor::export;
use ai_usage_monitor::ingest::Ingestor;
use ai_usage_monitor::store::Store;

use crate::netpresence::NetPresence;
use crate::tray_glyph::{self, Glyph};

/// Base run-loop tick; each detector runs on its own multiple of this.
const BASE_TICK_S: f64 = 1.0;
/// Refresh menu text at least this often (ticks) even without a state change.
const MENU_REFRESH_EVERY_TICKS: u64 = 3;
/// Window for the "recorded today" status count.
const RECENT_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
/// After an interaction is ingested, hold the "catching a chat" glyph this long.
const FRESH_INGEST_MS: i64 = 8_000;
/// Content-free heartbeat cadence.
const HEARTBEAT_MS: i64 = 30_000;

/// Timed-pause durations (monotonic ms). Indefinite uses an i64::MAX sentinel.
const PAUSE_15M_MS: i64 = 15 * 60 * 1000;
const PAUSE_1H_MS: i64 = 60 * 60 * 1000;

/// Monotonic + wall clock for one tick. Detection cadence uses the monotonic
/// value (immune to wall-clock jumps); stored timestamps use the wall clock.
#[derive(Clone, Copy)]
struct Clock {
    mono_ms: i64,
    wall_ms: i64,
}

/// Shared, main-thread-only runtime. Every field a tick or menu action touches
/// lives here behind a `Cell`/`RefCell`; nothing crosses threads.
struct Runtime {
    store: Rc<Store>,
    ingestor: RefCell<Ingestor>,
    net: RefCell<NetPresence>,
    install_id: String,
    export_dir: PathBuf,

    transcript_poll_ms: i64,
    network_poll_ms: i64,
    flush_ms: i64,

    tick_count: Cell<u64>,
    last_transcript_ms: Cell<i64>,
    last_network_ms: Cell<i64>,
    last_flush_ms: Cell<i64>,
    heartbeat_at: Cell<i64>,
    /// Monotonic ms of the last newly-ingested turn (drives the "catching" glyph).
    last_ingest_ms: Cell<i64>,
    /// Distinct AI providers seen active on the network at the last poll.
    net_active: Cell<usize>,

    /// `None` = active; `Some(t)` = paused until monotonic ms `t` (i64::MAX =
    /// until the user resumes).
    paused_until: Cell<Option<i64>>,
    start: Instant,

    tray: RefCell<Option<TrayIcon>>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    painted: Cell<Option<Glyph>>,
    status_item: MenuItem,
    detail_item: MenuItem,
    resume_item: MenuItem,
    ids: MenuIds,
}

/// Command menu-item ids, matched in the event drain.
struct MenuIds {
    pause_15m: MenuId,
    pause_1h: MenuId,
    pause_indef: MenuId,
    resume: MenuId,
    show_data: MenuId,
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
            install_tray(&rt);
            install_timer(&rt);
            log::info!("ai-usage-monitor started (transcript ingest + network presence)");
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
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()));
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    // Ingest activity from launch onward, not the whole archive.
    let ingestor = Ingestor::new(home, now_ms);
    let net = NetPresence::new(cfg.presence_gap_ms as i64);

    let ids = MenuIds {
        pause_15m: MenuId::new("pause_15m"),
        pause_1h: MenuId::new("pause_1h"),
        pause_indef: MenuId::new("pause_indef"),
        resume: MenuId::new("resume"),
        show_data: MenuId::new("show_data"),
        quit: MenuId::new("quit"),
    };

    let status_item = MenuItem::new("Starting…", false, None);
    let detail_item = MenuItem::new("", false, None);
    let resume_item = MenuItem::with_id(ids.resume.clone(), "Resume now", false, None);

    Rc::new(Runtime {
        store,
        ingestor: RefCell::new(ingestor),
        net: RefCell::new(net),
        install_id: cfg.install_id.clone(),
        export_dir: paths.export_dir.clone(),
        transcript_poll_ms: cfg.transcript_poll_ms as i64,
        network_poll_ms: cfg.network_poll_ms as i64,
        flush_ms: cfg.flush_ms as i64,
        tick_count: Cell::new(0),
        last_transcript_ms: Cell::new(i64::MIN),
        last_network_ms: Cell::new(i64::MIN),
        last_flush_ms: Cell::new(0),
        heartbeat_at: Cell::new(0),
        last_ingest_ms: Cell::new(i64::MIN),
        net_active: Cell::new(0),
        paused_until: Cell::new(None),
        start: Instant::now(),
        tray: RefCell::new(None),
        timer: RefCell::new(None),
        painted: Cell::new(None),
        status_item,
        detail_item,
        resume_item,
        ids,
    })
}

fn install_tray(rt: &Rc<Runtime>) {
    let menu = Menu::new();

    let pause = Submenu::new("Take a break", true);
    pause
        .append(&MenuItem::with_id(rt.ids.pause_15m.clone(), "For 15 minutes", true, None))
        .and(pause.append(&MenuItem::with_id(rt.ids.pause_1h.clone(), "For an hour", true, None)))
        .and(pause.append(&MenuItem::with_id(rt.ids.pause_indef.clone(), "Until I'm back", true, None)))
        .expect("build pause submenu");

    let show_data = MenuItem::with_id(rt.ids.show_data.clone(), "Show my data", true, None);
    let quit = MenuItem::with_id(rt.ids.quit.clone(), "Quit", true, None);

    menu.append(&rt.status_item).expect("status");
    menu.append(&rt.detail_item).expect("detail");
    menu.append(&PredefinedMenuItem::separator()).expect("sep1");
    menu.append(&pause).expect("pause");
    menu.append(&rt.resume_item).expect("resume");
    menu.append(&PredefinedMenuItem::separator()).expect("sep2");
    menu.append(&show_data).expect("show_data");
    menu.append(&quit).expect("quit");

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(tray_glyph::icon(Glyph::Monitoring))
        .with_icon_as_template(true)
        .build()
        .expect("build tray icon");
    *rt.tray.borrow_mut() = Some(tray);
    let clock = rt.clock();
    paint(rt, Glyph::Monitoring);
    refresh_menu(rt, Glyph::Monitoring, clock.wall_ms);
}

fn install_timer(rt: &Rc<Runtime>) {
    let rt_for_block = rt.clone();
    let block = RcBlock::new(move |_t: NonNull<NSTimer>| tick(&rt_for_block));
    let timer =
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(BASE_TICK_S, true, &block) };
    *rt.timer.borrow_mut() = Some(timer);
}

/// One tick: honor pause, run each detector on its cadence, repaint on change,
/// flush, and drain menu clicks.
fn tick(rt: &Rc<Runtime>) {
    let clock = rt.clock();

    if let Some(until) = rt.paused_until.get() {
        if clock.mono_ms >= until {
            rt.paused_until.set(None);
            log::info!("auto-resumed after timed pause");
        }
    }

    if rt.is_paused() {
        paint(rt, Glyph::Paused);
        refresh_menu(rt, Glyph::Paused, clock.wall_ms);
        drain_menu_events(rt);
        return;
    }

    let n = rt.tick_count.get().wrapping_add(1);
    rt.tick_count.set(n);

    // Layer A — transcript ingestion. saturating_sub, never `-`: the timers init
    // to i64::MIN ("due immediately"), and `mono - i64::MIN` OVERFLOWS in release
    // and wraps negative, which would make every poll read as "not due" and stop
    // detection entirely. Saturating gives i64::MAX (== due) on the first tick.
    if clock.mono_ms.saturating_sub(rt.last_transcript_ms.get()) >= rt.transcript_poll_ms {
        rt.last_transcript_ms.set(clock.mono_ms);
        let stats = rt.ingestor.borrow_mut().poll(&rt.store);
        if stats.new_turns > 0 {
            rt.last_ingest_ms.set(clock.mono_ms);
            log::info!(
                "ingested {} new message(s) across {} session(s)",
                stats.new_turns,
                stats.sessions
            );
        }
    }

    // Layer B — network presence.
    if clock.mono_ms.saturating_sub(rt.last_network_ms.get()) >= rt.network_poll_ms {
        rt.last_network_ms.set(clock.mono_ms);
        let active = rt.net.borrow_mut().poll(&rt.store, clock.wall_ms);
        rt.net_active.set(active);
    }

    if clock.mono_ms.saturating_sub(rt.heartbeat_at.get()) > HEARTBEAT_MS {
        rt.heartbeat_at.set(clock.mono_ms);
        log::info!(
            "heartbeat: {} AI provider(s) active on the network; watching transcripts for new messages",
            rt.net_active.get()
        );
    }

    let glyph = glyph_for(rt, clock.mono_ms);
    let changed = rt.painted.get() != Some(glyph);
    paint(rt, glyph);
    if changed || n % MENU_REFRESH_EVERY_TICKS == 0 {
        refresh_menu(rt, glyph, clock.wall_ms);
    }

    if clock.mono_ms.saturating_sub(rt.last_flush_ms.get()) > rt.flush_ms {
        rt.last_flush_ms.set(clock.mono_ms);
        match export::flush_pending(&rt.store, &rt.install_id, &rt.export_dir, clock.wall_ms) {
            Ok(n) if n > 0 => log::info!("wrote {n} record(s) to the day file"),
            Ok(_) => {}
            Err(e) => log::error!("day-file flush error: {e}"),
        }
    }

    drain_menu_events(rt);
}

/// Icon state: a real interaction just recorded flashes the disc briefly;
/// otherwise the steady monitoring glyph. Network presence (an app merely open)
/// deliberately does NOT drive the icon — that state never cleared and read as
/// stale; it lives in the data and the dropdown detail instead.
fn glyph_for(rt: &Rc<Runtime>, now_mono_ms: i64) -> Glyph {
    // saturating_sub: last_ingest_ms init is i64::MIN, and `now - i64::MIN`
    // overflows/wraps in release to a small value → the icon would read
    // "Recording" forever. Saturating gives i64::MAX (not fresh) → Monitoring.
    if now_mono_ms.saturating_sub(rt.last_ingest_ms.get()) < FRESH_INGEST_MS {
        Glyph::Recording
    } else {
        Glyph::Monitoring
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

fn refresh_menu(rt: &Rc<Runtime>, glyph: Glyph, now_ms: i64) {
    let status = match glyph {
        Glyph::Paused => "Taking a break ☕".to_string(),
        Glyph::Recording => "Recording an AI chat…".to_string(),
        Glyph::Monitoring => "Watching for AI use 👀".to_string(),
    };
    rt.status_item.set_text(status);

    match rt.store.activity_stats(now_ms - RECENT_WINDOW_MS) {
        Ok(stats) if stats.recent_interactions == 0 && stats.last_activity_ms.is_none() => {
            rt.detail_item.set_text("Nothing recorded yet today");
        }
        Ok(stats) => rt.detail_item.set_text(format!(
            "{} AI session{} today · last {}",
            stats.recent_interactions,
            if stats.recent_interactions == 1 { "" } else { "s" },
            relative_time(stats.last_activity_ms, now_ms)
        )),
        Err(e) => log::error!("status stats error: {e}"),
    }

    rt.resume_item.set_enabled(rt.is_paused());
}

fn drain_menu_events(rt: &Rc<Runtime>) {
    while let Ok(ev) = MenuEvent::receiver().try_recv() {
        let id = &ev.id;
        if id == &rt.ids.show_data {
            do_show_data(rt);
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

/// Apply a pause/resume. On pause, close any open presence interval so a
/// half-observed span is finalized rather than left dangling.
fn set_pause(rt: &Rc<Runtime>, until: Option<i64>, why: &str) {
    if until.is_some() {
        rt.net.borrow_mut().flush_open(&rt.store, rt.clock().wall_ms);
    }
    rt.paused_until.set(until);
    log::info!("{why}");
    let glyph = if until.is_some() { Glyph::Paused } else { Glyph::Monitoring };
    paint(rt, glyph);
    refresh_menu(rt, glyph, rt.clock().wall_ms);
}

/// Reveal the day-partitioned data folder in Finder, flushing pending first.
fn do_show_data(rt: &Rc<Runtime>) {
    rt.net.borrow_mut().flush_open(&rt.store, rt.clock().wall_ms);
    let _ = export::flush_pending(&rt.store, &rt.install_id, &rt.export_dir, rt.clock().wall_ms);
    let dir = export::data_dir_path(&rt.export_dir);
    let _ = Command::new("open").arg(&dir).spawn();
}

fn do_quit(rt: &Rc<Runtime>) {
    rt.net.borrow_mut().flush_open(&rt.store, rt.clock().wall_ms);
    let _ = export::flush_pending(&rt.store, &rt.install_id, &rt.export_dir, rt.clock().wall_ms);
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).terminate(None);
    }
}

fn tooltip_for(glyph: Glyph) -> &'static str {
    match glyph {
        Glyph::Paused => "AI Usage Monitor — taking a break",
        Glyph::Recording => "AI Usage Monitor — recording an AI chat",
        Glyph::Monitoring => "AI Usage Monitor — watching for AI use",
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
