//! Optional SteamVR dashboard overlay.
//!
//! A background thread waits for SteamVR, creates a dashboard overlay and
//! renders the regular [`psstates_ui::MainUi`] into it (CPU rasterised and
//! uploaded with `SetOverlayRaw`). Laser pointer input arrives as overlay
//! mouse events and text entry uses the SteamVR keyboard.
//!
//! The `openvr_api` library is loaded dynamically, so the app runs fine
//! without SteamVR or without the library present.

mod ffi;
mod overlay;
mod raster;
mod upload;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use psstates_service::Service;
use psstates_ui::{VrControl, VrState};

pub use raster::SoftwareRenderer;

/// SteamVR application key (must match `app.vrmanifest`).
pub const APP_KEY: &str = "i5ucc.parametersavestates";
/// Overlay key (same as the Unity version).
pub const OVERLAY_KEY: &str = "parametersavestates_menu";
pub const OVERLAY_NAME: &str = "ParameterSaveStates";

/// File name of the SteamVR app manifest next to the executable.
pub const MANIFEST_FILE: &str = "app.vrmanifest";

/// Lifecycle notifications for the host application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VrNotification {
    /// Connected to SteamVR; the dashboard overlay is available.
    Connected,
    /// Lost the connection to SteamVR (or it was never established).
    Disconnected,
    /// SteamVR is shutting down.
    Quit,
}

pub struct VrOptions {
    /// SteamVR app manifest to register (defaults to `app.vrmanifest` next to the executable).
    pub manifest_path: Option<PathBuf>,
}

impl Default for VrOptions {
    fn default() -> Self {
        let manifest_path = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join(MANIFEST_FILE)));
        Self { manifest_path }
    }
}

enum Command {
    SetAutoLaunch(bool),
}

struct Shared {
    state: Mutex<VrState>,
    commands: Mutex<Vec<Command>>,
    stop: AtomicBool,
    /// Set when the service reports changes; triggers a redraw.
    dirty: AtomicBool,
    /// Set once the first connection attempt has finished (either way).
    attempted: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl VrControl for Shared {
    fn state(&self) -> VrState {
        lock(&self.state).clone()
    }

    fn set_auto_launch(&self, enabled: bool) {
        lock(&self.commands).push(Command::SetAutoLaunch(enabled));
    }
}

/// Handle to the overlay thread.
#[derive(Clone)]
pub struct VrHandle {
    shared: Arc<Shared>,
    thread: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl VrHandle {
    pub fn state(&self) -> VrState {
        self.shared.state()
    }

    /// Settings access for the desktop UI.
    pub fn control(&self) -> Arc<dyn VrControl> {
        self.shared.clone()
    }

    /// Waits (up to `timeout`) for the first connection attempt and returns
    /// whether SteamVR is connected.
    pub fn wait_connected(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self.state().connected {
                return true;
            }
            if self.shared.attempted.load(Ordering::SeqCst) || std::time::Instant::now() >= deadline
            {
                return self.state().connected;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Stops the overlay thread (destroys the overlay, shuts OpenVR down).
    pub fn shutdown(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(t) = lock(&self.thread).take() {
            let _ = t.join();
        }
    }
}

/// Starts the overlay thread.
pub fn spawn(
    service: Service,
    options: VrOptions,
    notify: impl Fn(VrNotification) + Send + Sync + 'static,
) -> VrHandle {
    let shared = Arc::new(Shared {
        state: Mutex::new(VrState::default()),
        attempted: AtomicBool::new(false),
        commands: Mutex::new(Vec::new()),
        stop: AtomicBool::new(false),
        dirty: AtomicBool::new(true),
    });
    {
        let shared = shared.clone();
        service.add_on_change(move || shared.dirty.store(true, Ordering::Relaxed));
    }
    let thread_shared = shared.clone();
    let thread = std::thread::Builder::new()
        .name("vr-overlay".into())
        .spawn(move || overlay::run(thread_shared, service, options, Box::new(notify)))
        .expect("failed to spawn VR thread");
    VrHandle {
        shared,
        thread: Arc::new(Mutex::new(Some(thread))),
    }
}
