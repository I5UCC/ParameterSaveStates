use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use egui::{Event, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect, ViewportId};
use psstates_service::Service;
use psstates_ui::{MainUi, VrControl};

use crate::ffi::{self, OpenVrLib, OverlayHandle, Session};
use crate::raster::SoftwareRenderer;
use crate::upload::Uploader;
use crate::{lock, Command, Shared, VrNotification, VrOptions, APP_KEY, OVERLAY_KEY, OVERLAY_NAME};

/// Render target size in pixels (16:10).
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1200;
/// UI scale: 1920x1200 px = 960x600 points.
const PIXELS_PER_POINT: f32 = 2.0;
/// Physical width of the dashboard overlay. 1920 px over 2.6 m is the same
/// pixel density as the previous 1600 px over 2.2 m, so text keeps its
/// physical size and the extra canvas is extra room for content.
const WIDTH_IN_METERS: f32 = 2.6;
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);
const FRAME_INTERVAL: Duration = Duration::from_millis(33);
const IDLE_INTERVAL: Duration = Duration::from_millis(100);

const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon.png");

type Notify = Box<dyn Fn(VrNotification) + Send + Sync>;

enum SessionEnd {
    Stopped,
    Quit,
}

pub(crate) fn run(shared: Arc<Shared>, service: Service, options: VrOptions, notify: Notify) {
    let lib = match OpenVrLib::load() {
        Ok(lib) => lib,
        Err(e) => {
            tracing::info!("SteamVR overlay disabled: {e:#}");
            lock(&shared.state).error = Some(format!(
                "SteamVR support unavailable: {} was not found next to the application",
                ffi::LIB_NAME
            ));
            shared.attempted.store(true, Ordering::SeqCst);
            return;
        }
    };
    tracing::info!("Loaded {}", lib.path().display());

    let control: Arc<dyn VrControl> = shared.clone();
    let mut frontend = Frontend::new(MainUi::new(service).overlay_mode().with_vr(control));
    let mut last_error: Option<String> = None;

    while !shared.stop.load(Ordering::SeqCst) {
        match lib.connect() {
            Ok(session) => {
                last_error = None;
                tracing::info!("Connected to SteamVR");
                let end = run_session(&session, &shared, &mut frontend, &options, &notify);
                drop(session);
                {
                    let mut st = lock(&shared.state);
                    st.connected = false;
                    st.auto_launch = None;
                }
                notify(VrNotification::Disconnected);
                match end {
                    Ok(SessionEnd::Stopped) => break,
                    Ok(SessionEnd::Quit) => {
                        tracing::info!("SteamVR is shutting down");
                        notify(VrNotification::Quit);
                    }
                    Err(e) => {
                        tracing::error!("SteamVR overlay error: {e:#}");
                        lock(&shared.state).error = Some(format!("{e:#}"));
                    }
                }
            }
            Err(e) => {
                let msg = format!("{e:#}");
                if last_error.as_deref() != Some(msg.as_str()) {
                    tracing::debug!("SteamVR not available: {msg}");
                    last_error = Some(msg);
                }
            }
        }
        shared.attempted.store(true, Ordering::SeqCst);
        sleep_unless_stopped(&shared, RECONNECT_INTERVAL);
    }
}

fn sleep_unless_stopped(shared: &Shared, total: Duration) {
    let deadline = Instant::now() + total;
    while Instant::now() < deadline && !shared.stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct OverlayGuard<'a> {
    session: &'a Session<'a>,
    handles: [OverlayHandle; 2],
}

impl Drop for OverlayGuard<'_> {
    fn drop(&mut self) {
        self.session.hide_keyboard();
        for h in self.handles {
            self.session.destroy_overlay(h);
        }
    }
}

fn warn_on_err(r: Result<()>) {
    if let Err(e) = r {
        tracing::warn!("{e:#}");
    }
}

fn thumbnail_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png).ok()?;
    let img = img
        .resize(256, 256, image::imageops::FilterType::Triangle)
        .into_rgba8();
    let (w, h) = img.dimensions();
    Some((img.into_raw(), w, h))
}

fn register_app(session: &Session<'_>, options: &VrOptions) -> Option<bool> {
    if let Some(path) = options.manifest_path.as_ref().filter(|p| p.is_file()) {
        match session.add_application_manifest(path, false) {
            Ok(()) => tracing::info!("Registered SteamVR manifest {}", path.display()),
            Err(e) => tracing::warn!("Could not register {}: {e:#}", path.display()),
        }
    }
    if !session.is_application_installed(APP_KEY) {
        return None;
    }
    if let Err(e) = session.identify_application(std::process::id(), APP_KEY) {
        // Expected when SteamVR launched us itself.
        tracing::debug!("{e:#}");
    }
    Some(session.get_application_auto_launch(APP_KEY))
}

fn run_session(
    session: &Session<'_>,
    shared: &Shared,
    frontend: &mut Frontend,
    options: &VrOptions,
    notify: &Notify,
) -> Result<SessionEnd> {
    let (main, thumb) = session.create_dashboard_overlay(OVERLAY_KEY, OVERLAY_NAME)?;
    let _guard = OverlayGuard {
        session,
        handles: [main, thumb],
    };
    warn_on_err(session.set_overlay_width_in_meters(main, WIDTH_IN_METERS));
    warn_on_err(session.set_overlay_input_method(main, ffi::INPUT_METHOD_MOUSE));
    warn_on_err(session.set_overlay_mouse_scale(main, WIDTH as f32, HEIGHT as f32));
    warn_on_err(session.set_overlay_flag(main, ffi::FLAG_SEND_VR_SMOOTH_SCROLL_EVENTS, true));
    // A new overlay starts empty: whatever was uploaded before is gone.
    frontend.forget_upload();
    let mut uploader = Uploader::new(session, WIDTH, HEIGHT);
    // Prefer the icon file shipped next to the manifest (SteamVR loads it
    // reliably); fall back to the embedded copy.
    let icon_file = options
        .manifest_path
        .as_ref()
        .and_then(|p| p.parent())
        .map(|d| d.join("icon.png"))
        .filter(|p| p.is_file());
    let from_file = icon_file
        .as_ref()
        .map(|p| session.set_overlay_from_file(thumb, p));
    if !matches!(from_file, Some(Ok(()))) {
        if let Some(Err(e)) = from_file {
            tracing::warn!("{e:#}");
        }
        if let Some((mut rgba, w, h)) = thumbnail_rgba() {
            warn_on_err(session.set_overlay_raw(thumb, &mut rgba, w, h));
        }
    }

    let auto_launch = register_app(session, options);
    {
        let mut st = lock(&shared.state);
        st.connected = true;
        st.error = if auto_launch.is_none() {
            Some(format!(
                "{} not registered with SteamVR; automatic start is unavailable",
                crate::MANIFEST_FILE
            ))
        } else {
            None
        };
        st.auto_launch = auto_launch;
        shared.attempted.store(true, Ordering::SeqCst);
    }
    notify(VrNotification::Connected);

    let mut was_visible = false;
    let mut upload_error_logged = false;
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return Ok(SessionEnd::Stopped);
        }
        while let Some(ev) = session.poll_next_event() {
            if ev.event_type == ffi::EVENT_QUIT {
                session.acknowledge_quit_exiting();
                return Ok(SessionEnd::Quit);
            }
        }
        while let Some(ev) = session.poll_next_overlay_event(main) {
            frontend.handle_event(session, &ev);
        }
        for cmd in std::mem::take(&mut *lock(&shared.commands)) {
            match cmd {
                Command::SetAutoLaunch(enabled) => {
                    warn_on_err(session.set_application_auto_launch(APP_KEY, enabled));
                    lock(&shared.state).auto_launch =
                        Some(session.get_application_auto_launch(APP_KEY));
                    shared.dirty.store(true, Ordering::Relaxed);
                }
            }
        }

        let visible = session.is_overlay_visible(main);
        if visible {
            if !was_visible {
                // Shown again: make sure SteamVR gets a fresh image.
                frontend.forget_upload();
            }
            let dirty = shared.dirty.swap(false, Ordering::Relaxed);
            if !was_visible || dirty || frontend.needs_frame() {
                frontend.frame(session, main);
                // Laser pointers send mouse moves constantly and egui repaints for
                // each, but most frames are pixel-identical. Every upload makes
                // SteamVR rebuild the overlay texture, which shows as flickering,
                // so only upload when the picture actually changed.
                if let Some(pixels) = frontend.changed_pixels() {
                    if let Err(e) = uploader.upload(session, main, pixels, WIDTH, HEIGHT) {
                        if !upload_error_logged {
                            tracing::error!("Overlay upload failed: {e:#}");
                            upload_error_logged = true;
                        }
                    }
                }
            }
        } else if was_visible {
            frontend.hidden(session);
        }
        was_visible = visible;
        std::thread::sleep(if visible {
            FRAME_INTERVAL
        } else {
            IDLE_INTERVAL
        });
    }
}

/// egui cannot be asked for a field's text directly, so the focused field is
/// select-all + copied and the copied text is read from the frame output.
enum Probe {
    Idle,
    /// Select-all + copy events are queued for the next frame.
    Requested,
    /// The field's current text.
    Ready(String),
}

/// egui context + renderer + input translation for the overlay.
struct Frontend {
    ui: MainUi,
    ctx: egui::Context,
    renderer: SoftwareRenderer,
    start: Instant,
    events: Vec<Event>,
    pointer: Pos2,
    next_repaint: Option<Instant>,
    keyboard_open: bool,
    keyboard_target: Option<egui::Id>,
    pending_text: Option<String>,
    /// Reading the focused field's text so the keyboard can start with it.
    probe: Probe,
    /// The image SteamVR currently shows (empty = unknown, upload next frame).
    last_upload: Vec<u8>,
    /// Fixed clock for tests; `None` uses real time.
    time_override: Option<f64>,
}

impl Frontend {
    fn new(ui: MainUi) -> Self {
        Self {
            ui,
            ctx: egui::Context::default(),
            renderer: SoftwareRenderer::new(WIDTH as usize, HEIGHT as usize),
            start: Instant::now(),
            events: Vec::new(),
            pointer: Pos2::ZERO,
            next_repaint: None,
            keyboard_open: false,
            keyboard_target: None,
            pending_text: None,
            probe: Probe::Idle,
            last_upload: Vec::new(),
            time_override: None,
        }
    }

    /// The next frame must be uploaded even if it equals the previous one.
    fn forget_upload(&mut self) {
        self.last_upload.clear();
    }

    /// The rendered image, if it differs from what SteamVR currently shows.
    fn changed_pixels(&mut self) -> Option<&mut [u8]> {
        let pixels = self.renderer.pixels_mut();
        if self.last_upload == *pixels {
            return None;
        }
        self.last_upload.clear();
        self.last_upload.extend_from_slice(pixels);
        Some(pixels)
    }

    fn needs_frame(&self) -> bool {
        !self.events.is_empty()
            || self.pending_text.is_some()
            || self.next_repaint.is_some_and(|t| Instant::now() >= t)
    }

    /// Overlay mouse coordinates (pixels, origin bottom-left) to egui points.
    fn to_points(x: f32, y: f32) -> Pos2 {
        Pos2::new(x / PIXELS_PER_POINT, (HEIGHT as f32 - y) / PIXELS_PER_POINT)
    }

    fn handle_event(&mut self, session: &Session<'_>, ev: &ffi::VrEvent) {
        match ev.event_type {
            ffi::EVENT_MOUSE_MOVE => {
                let (x, y, _) = ev.mouse();
                self.pointer = Self::to_points(x, y);
                self.events.push(Event::PointerMoved(self.pointer));
            }
            ffi::EVENT_MOUSE_BUTTON_DOWN | ffi::EVENT_MOUSE_BUTTON_UP => {
                let (x, y, button) = ev.mouse();
                self.pointer = Self::to_points(x, y);
                let button = match button {
                    ffi::MOUSE_RIGHT => PointerButton::Secondary,
                    ffi::MOUSE_MIDDLE => PointerButton::Middle,
                    ffi::MOUSE_LEFT => PointerButton::Primary,
                    _ => PointerButton::Primary,
                };
                self.events.push(Event::PointerMoved(self.pointer));
                self.events.push(Event::PointerButton {
                    pos: self.pointer,
                    button,
                    pressed: ev.event_type == ffi::EVENT_MOUSE_BUTTON_DOWN,
                    modifiers: Modifiers::default(),
                });
            }
            ffi::EVENT_SCROLL_SMOOTH => {
                let (dx, dy) = ev.scroll();
                self.events.push(Event::MouseWheel {
                    unit: MouseWheelUnit::Line,
                    delta: egui::vec2(dx, dy),
                    modifiers: Modifiers::default(),
                });
            }
            ffi::EVENT_FOCUS_LEAVE => self.events.push(Event::PointerGone),
            ffi::EVENT_OVERLAY_SHOWN => self.next_repaint = Some(Instant::now()),
            ffi::EVENT_KEYBOARD_DONE => {
                self.keyboard_open = false;
                self.pending_text = Some(session.get_keyboard_text());
            }
            ffi::EVENT_KEYBOARD_CLOSED => {
                self.keyboard_open = false;
                if self.pending_text.is_none() {
                    self.release_focus();
                }
            }
            _ => {}
        }
    }

    fn release_focus(&mut self) {
        if let Some(id) = self.keyboard_target.take() {
            self.ctx.memory_mut(|m| m.surrender_focus(id));
            self.next_repaint = Some(Instant::now());
        }
    }

    fn hidden(&mut self, session: &Session<'_>) {
        if self.keyboard_open {
            session.hide_keyboard();
            self.keyboard_open = false;
        }
        self.release_focus();
        self.events.push(Event::PointerGone);
    }

    fn frame(&mut self, session: &Session<'_>, overlay: OverlayHandle) {
        if self.render() {
            // Keyboard text was typed into the focused field.
            self.release_focus();
            return;
        }
        self.sync_keyboard(session, overlay);
    }

    /// Runs egui and rasterises into the CPU buffer. Returns whether text from
    /// the SteamVR keyboard was injected.
    fn render(&mut self) -> bool {
        let mut events = std::mem::take(&mut self.events);
        let injected = self.pending_text.take();
        if let Some(text) = &injected {
            // Replace the focused text field's content with the keyboard text.
            for pressed in [true, false] {
                events.push(Event::Key {
                    key: Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::COMMAND,
                });
            }
            events.push(Event::Text(text.clone()));
        }
        let probing = matches!(self.probe, Probe::Requested);
        if probing {
            for pressed in [true, false] {
                events.push(Event::Key {
                    key: Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Modifiers::COMMAND,
                });
            }
            events.push(Event::Copy);
        }

        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(
                    WIDTH as f32 / PIXELS_PER_POINT,
                    HEIGHT as f32 / PIXELS_PER_POINT,
                ),
            )),
            time: Some(
                self.time_override
                    .unwrap_or_else(|| self.start.elapsed().as_secs_f64()),
            ),
            focused: true,
            max_texture_side: Some(4096),
            events,
            ..Default::default()
        };
        input
            .viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(PIXELS_PER_POINT);

        let ui = &mut self.ui;
        let output = self.ctx.run(input, |ctx| ui.show(ctx));
        if probing {
            let copied = output.platform_output.commands.iter().find_map(|c| match c {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            });
            self.probe = Probe::Ready(copied.unwrap_or_default());
        }
        self.renderer.update_textures(&output.textures_delta);
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.renderer.render(&primitives, output.pixels_per_point);

        self.next_repaint = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map(|v| v.repaint_delay)
            .filter(|d| *d < Duration::from_secs(60))
            .map(|d| Instant::now() + d);

        injected.is_some()
    }

    /// The SteamVR keyboard follows egui's text focus.
    fn sync_keyboard(&mut self, session: &Session<'_>, overlay: OverlayHandle) {
        let wants_keyboard = self.ctx.wants_keyboard_input();
        let focused = self.ctx.memory(|m| m.focused());
        if wants_keyboard && !self.keyboard_open && focused.is_some() {
            let existing = match std::mem::replace(&mut self.probe, Probe::Idle) {
                Probe::Ready(text) => text,
                _ => {
                    self.probe = Probe::Requested;
                    self.next_repaint = Some(Instant::now());
                    return;
                }
            };
            match session.show_keyboard_for_overlay(overlay, "Enter text", &existing, 256) {
                Ok(()) => {
                    self.keyboard_open = true;
                    self.keyboard_target = focused;
                }
                Err(e) => {
                    tracing::warn!("{e:#}");
                    // Avoid retrying every frame.
                    self.keyboard_target = focused;
                    self.release_focus();
                }
            }
        } else if !wants_keyboard {
            self.probe = Probe::Idle;
            if self.keyboard_open {
                session.hide_keyboard();
                self.keyboard_open = false;
                self.keyboard_target = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use psstates_core::Store;

    struct Fixture {
        _rt: tokio::runtime::Runtime,
        _dir: tempfile::TempDir,
        frontend: Frontend,
        clock: f64,
    }

    impl Fixture {
        fn new() -> Self {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .unwrap();
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(dir.path()).unwrap();
            let service = Service::start_with(rt.handle().clone(), store, false).unwrap();
            service
                .store()
                .update_avatar("avtr_demo", |a| {
                    for name in ["Casual", "Winter outfit", "Dance floor", "Gym"] {
                        a.add_profile(Some(name), Default::default())?;
                    }
                    Ok(())
                })
                .unwrap();
            service.preview("VRChat-Client-TEST", "avtr_demo", "Demo Avatar");
            Self {
                _rt: rt,
                _dir: dir,
                frontend: Frontend::new(MainUi::new(service).overlay_mode()),
                clock: 0.0,
            }
        }

        /// Renders one frame, advancing the clock like a 30 fps overlay would.
        fn frame(&mut self) -> Vec<u8> {
            self.clock += 0.033;
            self.frontend.time_override = Some(self.clock);
            self.frontend.render();
            self.frontend.renderer.pixels_mut().to_vec()
        }

        fn move_pointer(&mut self, x: f32, y: f32) {
            self.frontend
                .events
                .push(Event::PointerMoved(Pos2::new(x, y)));
        }
    }

    /// Hovering anything must settle in a single frame (no animation), because
    /// each changed frame is a texture upload that SteamVR shows as flicker.
    #[test]
    fn pointer_hover_settles_immediately() {
        let mut f = Fixture::new();
        for _ in 0..8 {
            f.frame();
        }
        let width = WIDTH as f32 / PIXELS_PER_POINT;
        let height = HEIGHT as f32 / PIXELS_PER_POINT;
        let mut positions = 0;
        let mut y = 12.0;
        while y < height {
            let mut x = 12.0;
            while x < width {
                f.move_pointer(x, y);
                let after_move = f.frame();
                let next = f.frame();
                assert!(
                    after_move == next,
                    "frame still changing after moving the pointer to ({x}, {y})"
                );
                positions += 1;
                x += 97.0;
            }
            y += 53.0;
        }
        assert!(positions > 80);
    }

    /// A resting laser pointer must never make a tooltip appear: it is never
    /// perfectly still, so tooltips would pop in and out (each a changed frame).
    #[test]
    fn tooltips_never_appear() {
        let mut f = Fixture::new();
        for _ in 0..8 {
            f.frame();
        }
        let mut checked = 0;
        // Sweep the area where the status bar, buttons and profile names are.
        let mut y = 40.0;
        while y < 360.0 {
            for x in [60.0, 480.0, 800.0] {
                f.move_pointer(x, y);
                f.frame();
                let settled = f.frame();
                // Longer than egui's default 0.5 s tooltip delay.
                for _ in 0..20 {
                    assert!(
                        f.frame() == settled,
                        "something appeared while the pointer rested at ({x}, {y})"
                    );
                }
                checked += 1;
            }
            y += 60.0;
        }
        assert!(checked >= 15);
    }

    /// A pointer that jitters without crossing anything must not cause uploads.
    #[test]
    fn identical_frames_are_not_uploaded() {
        let mut f = Fixture::new();
        for _ in 0..8 {
            f.frame();
        }
        // Park the pointer on empty background in the bottom right corner.
        f.move_pointer(940.0, 590.0);
        f.frame();
        f.frame();
        assert!(f.frontend.changed_pixels().is_some(), "first upload");
        let mut uploads = 0;
        for i in 0..60 {
            f.move_pointer(940.0 + (i % 3) as f32, 590.0);
            f.frame();
            if f.frontend.changed_pixels().is_some() {
                uploads += 1;
            }
        }
        assert_eq!(uploads, 0, "pointer jitter caused re-uploads");

        // A new overlay (or re-shown dashboard) needs the image again.
        f.frontend.forget_upload();
        f.frame();
        assert!(f.frontend.changed_pixels().is_some());
    }
}
