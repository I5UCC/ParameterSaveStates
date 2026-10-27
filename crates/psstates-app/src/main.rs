#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod instance;
mod tray;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use psstates_core::{legacy, paths, Store};
use psstates_service::{Service, VERSION};
use psstates_ui::MainUi;

pub const APP_TITLE: &str = "Parameter Save States";
const LOG_FILE: &str = "ParameterSaveStates.log";
const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon.png");

/// Commands sent to the main thread (from the tray, other instances, VR).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCommand {
    /// Show (or focus) the window.
    Show,
    /// Exit the application.
    Exit,
    /// SteamVR is shutting down.
    VrQuit,
}

type Sender = Arc<dyn Fn(AppCommand) + Send + Sync>;
type ContextSlot = Arc<Mutex<Option<egui::Context>>>;

fn main() {
    init_logging();
    if let Err(e) = run() {
        tracing::error!("{e:#}");
        show_error(&format!("{e:#}"));
        std::process::exit(1);
    }
}

/// Logs to stdout and to `ParameterSaveStates.log` in the data directory
/// (release builds on Windows have no console).
fn init_logging() {
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    use tracing_subscriber::EnvFilter;
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,mdns_sd=warn"));
    let file = paths::default_data_dir().and_then(|dir| {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::File::create(dir.join(LOG_FILE)).ok()
    });
    match file {
        Some(file) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(std::io::stdout.and(Mutex::new(file)))
            .init(),
        None => tracing_subscriber::fmt().with_env_filter(filter).init(),
    }
}

fn decode_icon() -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .ok()?
        .resize(64, 64, image::imageops::FilterType::Triangle)
        .into_rgba8();
    let (w, h) = img.dimensions();
    Some((img.into_raw(), w, h))
}

fn show_error(text: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(APP_TITLE)
        .set_description(text)
        .show();
}

fn usage() -> String {
    format!(
        "ParameterSaveStates {VERSION}\n\n\
         USAGE:\n  ParameterSaveStates                 start the app\n  \
         ParameterSaveStates --minimized     start in the tray (used by SteamVR)\n  \
         ParameterSaveStates migrate [DIR]   import data of the old Unity version\n  \
         ParameterSaveStates --data-dir      print the data directory\n  \
         ParameterSaveStates --version"
    )
}

fn open_store() -> Result<Store> {
    let dir = paths::default_data_dir().context("Could not determine the data directory")?;
    Store::open(&dir).with_context(|| format!("Could not open data directory {}", dir.display()))
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => run_gui(false),
        Some("--minimized") => run_gui(true),
        Some("--version" | "-V") => {
            println!("{VERSION}");
            Ok(())
        }
        Some("--help" | "-h") => {
            println!("{}", usage());
            Ok(())
        }
        Some("--data-dir") => {
            println!("{}", open_store()?.root().display());
            Ok(())
        }
        Some("migrate") => {
            let dir = match args.get(1) {
                Some(d) => PathBuf::from(d),
                None => paths::legacy_profiles_dir()
                    .ok_or_else(|| anyhow!("No legacy data found; pass the 'Profiles' folder"))?,
            };
            let store = open_store()?;
            let report = legacy::migrate_legacy_dir(&store, &dir)?;
            println!("{}", report.summary());
            for w in &report.warnings {
                println!("warning: {w}");
            }
            Ok(())
        }
        Some(other) => Err(anyhow!("Unknown argument '{other}'\n\n{}", usage())),
    }
}

fn run_gui(minimized: bool) -> Result<()> {
    let listener = match instance::acquire() {
        instance::Instance::Primary(listener) => listener,
        instance::Instance::Signalled => {
            tracing::info!("Already running; asked the other instance to show its window");
            return Ok(());
        }
        instance::Instance::Blocked => {
            show_error(&format!(
                "Port {} is in use. Is the old version of Parameter Save States still running?",
                instance::PORT
            ));
            return Ok(());
        }
    };

    let store = open_store()?;
    tracing::info!("Data directory: {}", store.root().display());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("psstates-rt")
        .build()
        .context("Failed to start async runtime")?;
    let service = Service::start(rt.handle().clone(), store)?;

    // Commands from other threads; wakes the window (if open) on every send.
    let ctx_slot: ContextSlot = Arc::default();
    let (tx, rx) = mpsc::channel::<AppCommand>();
    let send: Sender = {
        let slot = ctx_slot.clone();
        Arc::new(move |cmd| {
            let _ = tx.send(cmd);
            request_repaint(&slot);
        })
    };
    {
        let slot = ctx_slot.clone();
        service.add_on_change(move || request_repaint(&slot));
    }
    instance::serve(listener, send.clone());

    let icon = decode_icon();
    let tray = match icon.clone() {
        Some(icon) => match tray::Tray::spawn(icon, send.clone()) {
            Ok(tray) => Some(tray),
            Err(e) => {
                tracing::warn!("No tray icon: {e:#}");
                None
            }
        },
        None => None,
    };

    let mut ui = MainUi::new(service.clone());
    let vr = VrIntegration::start(&service, &send);
    if let Some(control) = vr.control() {
        ui = ui.with_vr(control);
    }
    // Like the old version: open the window unless we run inside SteamVR.
    let in_vr = !minimized && vr.connected_at_startup();
    let start_hidden = minimized || in_vr;
    // Started for SteamVR: exit together with it (unless the window is open).
    let exit_with_vr = start_hidden;
    tracing::info!(
        "Starting {} (tray: {}, SteamVR: {})",
        if start_hidden {
            "in the background"
        } else {
            "with window"
        },
        tray.is_some(),
        vr.connected()
    );

    let ui = Rc::new(RefCell::new(ui));
    let rx = Rc::new(rx);
    let window_icon = icon.map(|(rgba, width, height)| {
        Arc::new(egui::IconData {
            rgba,
            width,
            height,
        })
    });

    let mut show = !start_hidden;
    loop {
        if show {
            show = false;
            tracing::info!("Opening window");
            let outcome = run_window(&ui, &rx, &ctx_slot, window_icon.clone())?;
            if outcome == WindowOutcome::Exit {
                break;
            }
            if tray.is_none() && !vr.connected() {
                break;
            }
            tracing::info!("Window closed; still running in the background");
            finish_window_teardown();
        }
        match rx.recv() {
            Ok(AppCommand::Show) => show = true,
            Ok(AppCommand::Exit) | Err(_) => break,
            Ok(AppCommand::VrQuit) if exit_with_vr => break,
            Ok(AppCommand::VrQuit) => {}
        }
    }

    tracing::info!("Exiting");
    vr.shutdown();
    drop(tray);
    service.shutdown();
    rt.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}

/// winit destroys its window asynchronously: dropping it only posts a message
/// to the thread's queue. Because the main thread then blocks on the command
/// channel instead of pumping messages, the (already closed) window would stay
/// on screen, frozen. Drain the queue so the window really goes away.
#[cfg(windows)]
fn finish_window_teardown() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };
    for _ in 0..30 {
        // SAFETY: plain Win32 message pump on the thread that owns the window.
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(not(windows))]
fn finish_window_teardown() {}

fn request_repaint(slot: &ContextSlot) {
    if let Some(ctx) = slot.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        ctx.request_repaint();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowOutcome {
    /// The user closed the window.
    Closed,
    /// Exit was requested (e.g. from the tray).
    Exit,
}

/// Opens the window and blocks until it is closed. Can be called repeatedly
/// (eframe reuses its event loop).
fn run_window(
    ui: &Rc<RefCell<MainUi>>,
    rx: &Rc<mpsc::Receiver<AppCommand>>,
    ctx_slot: &ContextSlot,
    icon: Option<Arc<egui::IconData>>,
) -> Result<WindowOutcome> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_TITLE)
        .with_app_id("parametersavestates")
        .with_inner_size([1100.0, 720.0])
        .with_min_inner_size([640.0, 480.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        persist_window: true,
        run_and_return: true,
        ..Default::default()
    };

    let exit = Rc::new(Cell::new(false));
    let app = App {
        ui: ui.clone(),
        rx: rx.clone(),
        exit: exit.clone(),
    };
    let slot = ctx_slot.clone();
    let result = eframe::run_native(
        APP_TITLE,
        options,
        Box::new(move |cc| {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(cc.egui_ctx.clone());
            app.ui.borrow_mut().context_changed();
            Ok(Box::new(app))
        }),
    );
    *ctx_slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
    result.map_err(|e| anyhow!("UI error: {e}"))?;
    Ok(if exit.get() {
        WindowOutcome::Exit
    } else {
        WindowOutcome::Closed
    })
}

struct App {
    ui: Rc<RefCell<MainUi>>,
    rx: Rc<mpsc::Receiver<AppCommand>>,
    exit: Rc<Cell<bool>>,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(cmd) = self.rx.try_recv() {
            match cmd {
                AppCommand::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                AppCommand::Exit => {
                    self.exit.set(true);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                // The window is open, keep running.
                AppCommand::VrQuit => {}
            }
        }
        self.ui.borrow_mut().show(ctx);
    }
}

// ------------------------------------------------------------------ VR glue

#[cfg(feature = "openvr")]
struct VrIntegration(psstates_vr::VrHandle);

#[cfg(feature = "openvr")]
impl VrIntegration {
    fn start(service: &Service, send: &Sender) -> Self {
        let send = send.clone();
        Self(psstates_vr::spawn(
            service.clone(),
            psstates_vr::VrOptions::default(),
            move |n| {
                if n == psstates_vr::VrNotification::Quit {
                    send(AppCommand::VrQuit);
                }
            },
        ))
    }

    fn control(&self) -> Option<Arc<dyn psstates_ui::VrControl>> {
        Some(self.0.control())
    }

    fn connected_at_startup(&self) -> bool {
        self.0.wait_connected(Duration::from_secs(3))
    }

    fn connected(&self) -> bool {
        self.0.state().connected
    }

    fn shutdown(&self) {
        self.0.shutdown();
    }
}

#[cfg(not(feature = "openvr"))]
struct VrIntegration;

#[cfg(not(feature = "openvr"))]
impl VrIntegration {
    fn start(_service: &Service, _send: &Sender) -> Self {
        Self
    }

    fn control(&self) -> Option<Arc<dyn psstates_ui::VrControl>> {
        None
    }

    fn connected_at_startup(&self) -> bool {
        false
    }

    fn connected(&self) -> bool {
        false
    }

    fn shutdown(&self) {}
}
