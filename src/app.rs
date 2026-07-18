use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::process::Command;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
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
use ai_usage_monitor::store::{ActivityStats, Store, PAUSE_UNTIL_KEY};

use crate::tray_glyph::{self, Glyph};
use crate::updater::{self, Update};

const BASE_TICK_S: f64 = 1.0;

const RECENT_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

const ACTIVE_WINDOW_MS: i64 = 45_000;

const HEARTBEAT_MS: i64 = 30_000;

const UPDATE_CHECK_MS: i64 = 6 * 60 * 60 * 1000;

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
    install_id: String,
    export_dir: PathBuf,

    transcript_poll_ms: i64,
    flush_ms: i64,

    last_transcript_ms: Cell<i64>,
    last_flush_ms: Cell<i64>,
    heartbeat_at: Cell<i64>,

    paused_until: Cell<Option<i64>>,
    start: Instant,

    last_update_check: Cell<i64>,
    update_rx: RefCell<Option<Receiver<Option<Update>>>>,
    available_update: RefCell<Option<Update>>,

    tray: RefCell<Option<TrayIcon>>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    painted: Cell<Option<Glyph>>,
    status_item: MenuItem,
    detail_item: MenuItem,
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
            install_tray(&rt);
            install_timer(&rt);
            log::info!("ai-usage-monitor started (transcript ingest)");
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
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let ingestor = Ingestor::new(home, now_ms);

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
    let resume_item = MenuItem::with_id(ids.resume.clone(), "Resume now", false, None);
    let update_item = MenuItem::with_id(ids.update.clone(), "Check for updates…", true, None);

    Rc::new(Runtime {
        store,
        ingestor: RefCell::new(ingestor),
        install_id: cfg.install_id.clone(),
        export_dir: paths.export_dir.clone(),
        transcript_poll_ms: cfg.transcript_poll_ms as i64,
        flush_ms: cfg.flush_ms as i64,
        last_transcript_ms: Cell::new(i64::MIN),
        last_flush_ms: Cell::new(0),
        heartbeat_at: Cell::new(0),
        paused_until: Cell::new(None),
        start: Instant::now(),
        last_update_check: Cell::new(i64::MIN),
        update_rx: RefCell::new(None),
        available_update: RefCell::new(None),
        tray: RefCell::new(None),
        timer: RefCell::new(None),
        painted: Cell::new(None),
        status_item,
        detail_item,
        resume_item,
        update_item,
        ids,
    })
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

    let show_data = MenuItem::with_id(rt.ids.show_data.clone(), "Show my data", true, None);
    let quit = MenuItem::with_id(rt.ids.quit.clone(), "Quit", true, None);

    let title = MenuItem::new(
        concat!("AI Usage Monitor ", env!("CARGO_PKG_VERSION")),
        false,
        None,
    );

    menu.append(&title).expect("title");
    menu.append(&PredefinedMenuItem::separator()).expect("sep0");
    menu.append(&rt.status_item).expect("status");
    menu.append(&rt.detail_item).expect("detail");
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
        if due(&rt.last_transcript_ms, clock.mono_ms, rt.transcript_poll_ms) {
            let stats = rt.ingestor.borrow_mut().poll(&rt.store);
            if stats.new_turns > 0 {
                log::info!(
                    "ingested {} new message(s) across {} session(s)",
                    stats.new_turns,
                    stats.sessions
                );
            }
        }
        if due(&rt.last_flush_ms, clock.mono_ms, rt.flush_ms) {
            if let Err(e) =
                export::flush_pending(&rt.store, &rt.install_id, &rt.export_dir, clock.wall_ms)
            {
                log::error!("day-file flush error: {e}");
            }
        }
        if due(&rt.heartbeat_at, clock.mono_ms, HEARTBEAT_MS) {
            log::info!("heartbeat: watching for new AI messages");
        }
    }

    if due(&rt.last_update_check, clock.mono_ms, UPDATE_CHECK_MS) {
        spawn_update_check(rt);
    }
    poll_update_check(rt);

    let stats = rt
        .store
        .activity_stats(clock.wall_ms - RECENT_WINDOW_MS)
        .unwrap_or_default();
    let glyph = glyph_for(rt, clock.wall_ms, &stats);
    paint(rt, glyph);
    refresh_menu(rt, glyph, clock.wall_ms, &stats);
    drain_menu_events(rt);
}

fn spawn_update_check(rt: &Rc<Runtime>) {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(updater::check());
    });
    *rt.update_rx.borrow_mut() = Some(rx);
}

fn poll_update_check(rt: &Rc<Runtime>) {
    let result = rt.update_rx.borrow().as_ref().and_then(|rx| rx.try_recv().ok());
    let Some(result) = result else { return };
    *rt.update_rx.borrow_mut() = None;
    match result {
        Some(update) => {
            rt.update_item.set_text(format!("Install update {}", update.version));
            *rt.available_update.borrow_mut() = Some(update);
        }
        None => {
            rt.update_item.set_text("You're on the latest version");
            *rt.available_update.borrow_mut() = None;
        }
    }
}

fn do_update(rt: &Rc<Runtime>) {
    let update = rt.available_update.borrow().clone();
    match update {
        Some(update) => {
            rt.update_item.set_text("Installing update…");
            match updater::install(&update) {
                Ok(()) => {
                    if let Some(mtm) = MainThreadMarker::new() {
                        NSApplication::sharedApplication(mtm).terminate(None);
                    }
                }
                Err(e) => {
                    log::error!("update install failed: {e}");
                    rt.update_item.set_text("Update failed — see log");
                }
            }
        }
        None => {
            rt.update_item.set_text("Checking for updates…");
            spawn_update_check(rt);
        }
    }
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

    if stats.recent_interactions == 0 && stats.last_activity_ms.is_none() {
        rt.detail_item.set_text("Nothing recorded yet today");
    } else {
        rt.detail_item.set_text(format!(
            "{} AI session{} today · last {}",
            stats.recent_interactions,
            if stats.recent_interactions == 1 {
                ""
            } else {
                "s"
            },
            relative_time(stats.last_activity_ms, now_ms)
        ));
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
    let _ = export::flush_pending(
        &rt.store,
        &rt.install_id,
        &rt.export_dir,
        rt.clock().wall_ms,
    );
    let dir = export::data_dir_path(&rt.export_dir);
    let _ = Command::new("open").arg(&dir).spawn();
}

fn do_quit(rt: &Rc<Runtime>) {
    let _ = export::flush_pending(
        &rt.store,
        &rt.install_id,
        &rt.export_dir,
        rt.clock().wall_ms,
    );
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).terminate(None);
    }
}

fn tooltip_for(glyph: Glyph) -> &'static str {
    match glyph {
        Glyph::Paused => "AI Usage Monitor — taking a break",
        Glyph::Active => "AI Usage Monitor — recording AI activity",
        Glyph::Idle => "AI Usage Monitor — watching for AI use",
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
