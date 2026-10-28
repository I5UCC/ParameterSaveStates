//! Minimal, dynamically loaded OpenVR bindings.
//!
//! Only the handful of calls needed for a dashboard overlay are bound. The
//! interfaces are accessed through their C function tables (`FnTable:`
//! prefix), with indices matching the interface versions below. The
//! `openvr_api` library is loaded at runtime so the app works (without VR)
//! when it is missing.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use libloading::Library;

pub const IVR_SYSTEM: &str = "IVRSystem_022";
pub const IVR_OVERLAY: &str = "IVROverlay_027";
pub const IVR_APPLICATIONS: &str = "IVRApplications_007";

/// Environment variable with an explicit path to the openvr_api library.
pub const LIB_PATH_ENV: &str = "PSSTATES_OPENVR_LIB";

#[cfg(windows)]
pub const LIB_NAME: &str = "openvr_api.dll";
#[cfg(target_os = "macos")]
pub const LIB_NAME: &str = "libopenvr_api.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
pub const LIB_NAME: &str = "libopenvr_api.so";

pub type OverlayHandle = u64;

// EVRApplicationType
pub const APP_TYPE_BACKGROUND: i32 = 3;
// EVRInitError
pub const INIT_ERROR_NONE: i32 = 0;
// VROverlayInputMethod
pub const INPUT_METHOD_MOUSE: i32 = 1;
// VROverlayFlags
pub const FLAG_SEND_VR_SMOOTH_SCROLL_EVENTS: i32 = 1 << 17;
// EGamepadTextInputMode / EGamepadTextInputLineMode
pub const KEYBOARD_INPUT_NORMAL: i32 = 0;
pub const KEYBOARD_LINE_SINGLE: i32 = 0;

// EVREventType
pub const EVENT_MOUSE_MOVE: u32 = 300;
pub const EVENT_MOUSE_BUTTON_DOWN: u32 = 301;
pub const EVENT_MOUSE_BUTTON_UP: u32 = 302;
pub const EVENT_FOCUS_LEAVE: u32 = 304;
pub const EVENT_SCROLL_SMOOTH: u32 = 309;
pub const EVENT_OVERLAY_SHOWN: u32 = 500;
pub const EVENT_QUIT: u32 = 700;
pub const EVENT_KEYBOARD_CLOSED: u32 = 1200;
pub const EVENT_KEYBOARD_DONE: u32 = 1202;

// ETextureType / EColorSpace
#[cfg(windows)]
pub const TEXTURE_TYPE_DIRECTX: i32 = 0;
#[cfg(windows)]
pub const COLOR_SPACE_GAMMA: i32 = 1;

/// `Texture_t`
#[cfg(windows)]
#[repr(C)]
pub struct VrTexture {
    pub handle: *mut c_void,
    pub texture_type: i32,
    pub color_space: i32,
}

// EVRMouseButton
pub const MOUSE_LEFT: u32 = 1;
pub const MOUSE_RIGHT: u32 = 2;
pub const MOUSE_MIDDLE: u32 = 4;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HmdVector2 {
    pub x: f32,
    pub y: f32,
}

/// `VREvent_Data_t` (a 48 byte union). The OpenVR headers use 8 byte
/// alignment on Windows and `#pragma pack(4)` on Linux.
#[cfg_attr(windows, repr(C, align(8)))]
#[cfg_attr(not(windows), repr(C))]
#[derive(Clone, Copy)]
pub struct EventData(pub [u8; 48]);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VrEvent {
    pub event_type: u32,
    pub tracked_device_index: u32,
    pub event_age_seconds: f32,
    pub data: EventData,
}

const _: () = assert!(std::mem::size_of::<VrEvent>() == if cfg!(windows) { 64 } else { 60 });

impl VrEvent {
    pub fn zeroed() -> Self {
        Self {
            event_type: 0,
            tracked_device_index: 0,
            event_age_seconds: 0.0,
            data: EventData([0; 48]),
        }
    }

    fn f32_at(&self, offset: usize) -> f32 {
        f32::from_ne_bytes(self.data.0[offset..offset + 4].try_into().unwrap())
    }

    fn u32_at(&self, offset: usize) -> u32 {
        u32::from_ne_bytes(self.data.0[offset..offset + 4].try_into().unwrap())
    }

    /// `VREvent_Mouse_t`: x, y (in mouse scale units, origin bottom-left) and button.
    pub fn mouse(&self) -> (f32, f32, u32) {
        (self.f32_at(0), self.f32_at(4), self.u32_at(8))
    }

    /// `VREvent_Scroll_t`: x and y delta.
    pub fn scroll(&self) -> (f32, f32) {
        (self.f32_at(0), self.f32_at(4))
    }
}

type InitFn = unsafe extern "C" fn(*mut i32, i32, *const c_char) -> u32;
type ShutdownFn = unsafe extern "C" fn();
type GetInterfaceFn = unsafe extern "C" fn(*const c_char, *mut i32) -> *mut c_void;
type IsVersionValidFn = unsafe extern "C" fn(*const c_char) -> bool;
type InitErrorDescFn = unsafe extern "C" fn(i32) -> *const c_char;

/// The loaded `openvr_api` library.
pub struct OpenVrLib {
    init: InitFn,
    shutdown: ShutdownFn,
    get_interface: GetInterfaceFn,
    is_version_valid: IsVersionValidFn,
    init_error_desc: InitErrorDescFn,
    path: PathBuf,
    _lib: Library,
}

fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os(LIB_PATH_ENV).filter(|p| !p.is_empty()) {
        out.push(PathBuf::from(p));
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        out.push(dir.join(LIB_NAME));
    }
    // Let the OS loader search its default paths.
    out.push(PathBuf::from(LIB_NAME));
    out
}

impl OpenVrLib {
    pub fn load() -> Result<Self> {
        let mut errors = Vec::new();
        for path in candidates() {
            // SAFETY: loading openvr_api runs no initialisation code with
            // preconditions we could violate.
            match unsafe { Library::new(&path) } {
                Ok(lib) => return Self::from_library(lib, path),
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }
        bail!("{LIB_NAME} not found ({})", errors.join("; "))
    }

    fn from_library(lib: Library, path: PathBuf) -> Result<Self> {
        // SAFETY: the symbol signatures match openvr_capi.h.
        unsafe {
            let init = *lib.get::<InitFn>(b"VR_InitInternal2\0")?;
            let shutdown = *lib.get::<ShutdownFn>(b"VR_ShutdownInternal\0")?;
            let get_interface = *lib.get::<GetInterfaceFn>(b"VR_GetGenericInterface\0")?;
            let is_version_valid = *lib.get::<IsVersionValidFn>(b"VR_IsInterfaceVersionValid\0")?;
            let init_error_desc =
                *lib.get::<InitErrorDescFn>(b"VR_GetVRInitErrorAsEnglishDescription\0")?;
            Ok(Self {
                init,
                shutdown,
                get_interface,
                is_version_valid,
                init_error_desc,
                path,
                _lib: lib,
            })
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn init_error(&self, code: i32) -> String {
        // SAFETY: returns a static string for any code.
        let ptr = unsafe { (self.init_error_desc)(code) };
        if ptr.is_null() {
            format!("OpenVR init error {code}")
        } else {
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned()
        }
    }

    /// Initialises OpenVR as a background application (does not start SteamVR).
    pub fn connect(&self) -> Result<Session<'_>> {
        let mut err = INIT_ERROR_NONE;
        // SAFETY: valid out pointer and null startup info.
        unsafe { (self.init)(&mut err, APP_TYPE_BACKGROUND, std::ptr::null()) };
        if err != INIT_ERROR_NONE {
            bail!("{}", self.init_error(err));
        }
        // From here on the session shuts OpenVR down on drop.
        let mut session = Session {
            lib: self,
            system: std::ptr::null(),
            overlay: std::ptr::null(),
            apps: std::ptr::null(),
        };
        session.system = self.interface(IVR_SYSTEM)?;
        session.overlay = self.interface(IVR_OVERLAY)?;
        session.apps = self.interface(IVR_APPLICATIONS)?;
        Ok(session)
    }

    fn interface(&self, version: &str) -> Result<FnTable> {
        let c_version = CString::new(version).unwrap();
        // SAFETY: valid C string.
        if !unsafe { (self.is_version_valid)(c_version.as_ptr()) } {
            bail!("SteamVR does not support {version}");
        }
        let name = CString::new(format!("FnTable:{version}")).unwrap();
        let mut err = INIT_ERROR_NONE;
        // SAFETY: valid C string and out pointer.
        let ptr = unsafe { (self.get_interface)(name.as_ptr(), &mut err) };
        if err != INIT_ERROR_NONE || ptr.is_null() {
            bail!("Failed to get {version}: {}", self.init_error(err));
        }
        Ok(ptr as FnTable)
    }
}

type FnTable = *const *const c_void;

/// Reads entry `index` of a function table as function pointer type `F`.
unsafe fn entry<F: Copy>(table: FnTable, index: usize) -> F {
    debug_assert_eq!(
        std::mem::size_of::<F>(),
        std::mem::size_of::<*const c_void>()
    );
    let ptr = *table.add(index);
    std::mem::transmute_copy::<*const c_void, F>(&ptr)
}

/// An initialised OpenVR connection. Shuts OpenVR down when dropped.
pub struct Session<'a> {
    lib: &'a OpenVrLib,
    system: FnTable,
    overlay: FnTable,
    apps: FnTable,
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        // SAFETY: OpenVR was initialised by `connect`.
        unsafe { (self.lib.shutdown)() };
    }
}

fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap()
}

macro_rules! call {
    ($table:expr, $index:expr, fn($($arg_ty:ty),*) -> $ret:ty, $($arg:expr),*) => {{
        // SAFETY: index and signature match the interface version's function table.
        unsafe {
            let f: unsafe extern "system" fn($($arg_ty),*) -> $ret = entry($table, $index);
            f($($arg),*)
        }
    }};
}

/// OpenVR error code (0 = success).
fn check(what: &str, code: i32) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(anyhow!("{what} failed (error {code})"))
    }
}

impl Session<'_> {
    // ------------------------------------------------------------- IVRSystem

    pub fn poll_next_event(&self) -> Option<VrEvent> {
        let mut ev = VrEvent::zeroed();
        let size = std::mem::size_of::<VrEvent>() as u32;
        let got = call!(
            self.system,
            29,
            fn(*mut VrEvent, u32) -> bool,
            &mut ev,
            size
        );
        got.then_some(ev)
    }

    /// LUID of the GPU SteamVR renders on (0 if unknown). Textures handed to
    /// SteamVR must be created on the same adapter.
    #[cfg(windows)]
    pub fn output_device_luid(&self) -> u64 {
        let mut luid = 0u64;
        call!(
            self.system,
            8,
            fn(*mut u64, i32, *mut c_void) -> (),
            &mut luid,
            TEXTURE_TYPE_DIRECTX,
            std::ptr::null_mut()
        );
        luid
    }

    pub fn acknowledge_quit_exiting(&self) {
        call!(self.system, 43, fn() -> (),);
    }

    // ------------------------------------------------------- IVRApplications

    pub fn add_application_manifest(&self, path: &Path, temporary: bool) -> Result<()> {
        let path = cstring(&path.to_string_lossy());
        let code = call!(
            self.apps,
            0,
            fn(*const c_char, bool) -> i32,
            path.as_ptr(),
            temporary
        );
        check("AddApplicationManifest", code)
    }

    pub fn is_application_installed(&self, app_key: &str) -> bool {
        let key = cstring(app_key);
        call!(self.apps, 2, fn(*const c_char) -> bool, key.as_ptr())
    }

    pub fn identify_application(&self, pid: u32, app_key: &str) -> Result<()> {
        let key = cstring(app_key);
        let code = call!(
            self.apps,
            11,
            fn(u32, *const c_char) -> i32,
            pid,
            key.as_ptr()
        );
        check("IdentifyApplication", code)
    }

    pub fn set_application_auto_launch(&self, app_key: &str, enabled: bool) -> Result<()> {
        let key = cstring(app_key);
        let code = call!(
            self.apps,
            17,
            fn(*const c_char, bool) -> i32,
            key.as_ptr(),
            enabled
        );
        check("SetApplicationAutoLaunch", code)
    }

    pub fn get_application_auto_launch(&self, app_key: &str) -> bool {
        let key = cstring(app_key);
        call!(self.apps, 18, fn(*const c_char) -> bool, key.as_ptr())
    }

    // ------------------------------------------------------------ IVROverlay

    pub fn create_dashboard_overlay(
        &self,
        key: &str,
        name: &str,
    ) -> Result<(OverlayHandle, OverlayHandle)> {
        let key = cstring(key);
        let name = cstring(name);
        let mut main = 0u64;
        let mut thumb = 0u64;
        let code = call!(
            self.overlay,
            65,
            fn(*const c_char, *const c_char, *mut u64, *mut u64) -> i32,
            key.as_ptr(),
            name.as_ptr(),
            &mut main,
            &mut thumb
        );
        check("CreateDashboardOverlay", code)?;
        Ok((main, thumb))
    }

    pub fn destroy_overlay(&self, handle: OverlayHandle) {
        call!(self.overlay, 2, fn(u64) -> i32, handle);
    }

    pub fn set_overlay_flag(&self, handle: OverlayHandle, flag: i32, enabled: bool) -> Result<()> {
        let code = call!(
            self.overlay,
            10,
            fn(u64, i32, bool) -> i32,
            handle,
            flag,
            enabled
        );
        check("SetOverlayFlag", code)
    }

    pub fn set_overlay_width_in_meters(&self, handle: OverlayHandle, width: f32) -> Result<()> {
        let code = call!(self.overlay, 21, fn(u64, f32) -> i32, handle, width);
        check("SetOverlayWidthInMeters", code)
    }

    pub fn is_overlay_visible(&self, handle: OverlayHandle) -> bool {
        call!(self.overlay, 43, fn(u64) -> bool, handle)
    }

    pub fn poll_next_overlay_event(&self, handle: OverlayHandle) -> Option<VrEvent> {
        let mut ev = VrEvent::zeroed();
        let size = std::mem::size_of::<VrEvent>() as u32;
        let got = call!(
            self.overlay,
            46,
            fn(u64, *mut VrEvent, u32) -> bool,
            handle,
            &mut ev,
            size
        );
        got.then_some(ev)
    }

    pub fn set_overlay_input_method(&self, handle: OverlayHandle, method: i32) -> Result<()> {
        let code = call!(self.overlay, 48, fn(u64, i32) -> i32, handle, method);
        check("SetOverlayInputMethod", code)
    }

    pub fn set_overlay_mouse_scale(&self, handle: OverlayHandle, x: f32, y: f32) -> Result<()> {
        let scale = HmdVector2 { x, y };
        let code = call!(
            self.overlay,
            50,
            fn(u64, *const HmdVector2) -> i32,
            handle,
            &scale
        );
        check("SetOverlayMouseScale", code)
    }

    /// Uploads RGBA8 pixels (rows top to bottom).
    pub fn set_overlay_from_file(&self, handle: OverlayHandle, path: &Path) -> Result<()> {
        let path = cstring(&path.to_string_lossy());
        let code = call!(
            self.overlay,
            61,
            fn(u64, *const c_char) -> i32,
            handle,
            path.as_ptr()
        );
        check("SetOverlayFromFile", code)
    }

    pub fn set_overlay_raw(
        &self,
        handle: OverlayHandle,
        rgba: &mut [u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        assert_eq!(rgba.len(), (width * height * 4) as usize);
        let code = call!(
            self.overlay,
            60,
            fn(u64, *mut c_void, u32, u32, u32) -> i32,
            handle,
            rgba.as_mut_ptr().cast(),
            width,
            height,
            4
        );
        check("SetOverlayRaw", code)
    }

    /// Shows a GPU texture on the overlay. SteamVR copies it during the call,
    /// so the texture can be reused for the next frame right away.
    ///
    /// # Safety
    /// `texture` must be a valid native texture of the given API type that was
    /// created on SteamVR's adapter.
    #[cfg(windows)]
    pub unsafe fn set_overlay_texture(
        &self,
        handle: OverlayHandle,
        texture: &VrTexture,
    ) -> Result<()> {
        let code = call!(
            self.overlay,
            58,
            fn(u64, *const VrTexture) -> i32,
            handle,
            texture
        );
        check("SetOverlayTexture", code)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show_keyboard_for_overlay(
        &self,
        handle: OverlayHandle,
        description: &str,
        existing: &str,
        max_chars: u32,
    ) -> Result<()> {
        let description = cstring(description);
        let existing = cstring(existing);
        let code = call!(
            self.overlay,
            73,
            fn(u64, i32, i32, u32, *const c_char, u32, *const c_char, u64) -> i32,
            handle,
            KEYBOARD_INPUT_NORMAL,
            KEYBOARD_LINE_SINGLE,
            0,
            description.as_ptr(),
            max_chars,
            existing.as_ptr(),
            0
        );
        check("ShowKeyboardForOverlay", code)
    }

    pub fn get_keyboard_text(&self) -> String {
        let mut buf = vec![0u8; 1024];
        let len = call!(
            self.overlay,
            74,
            fn(*mut c_char, u32) -> u32,
            buf.as_mut_ptr().cast(),
            buf.len() as u32
        );
        let len = (len as usize).min(buf.len());
        let end = buf[..len].iter().position(|b| *b == 0).unwrap_or(len);
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }

    pub fn hide_keyboard(&self) {
        call!(self.overlay, 75, fn() -> (),);
    }
}

/// Live checks against a running SteamVR. Ignored by default:
/// `cargo test -p psstates-vr --release live_ -- --ignored --nocapture`
/// (set `PSSTATES_OPENVR_LIB` if `openvr_api` is not next to the test binary).
#[cfg(test)]
mod live {
    use super::*;
    use std::time::Instant;

    fn with_overlay(f: impl FnOnce(&Session<'_>, OverlayHandle)) {
        let lib = OpenVrLib::load().expect("openvr_api not found");
        let session = lib.connect().expect("SteamVR is not running");
        // A throwaway key so a running copy of the app is not disturbed.
        let (main, thumb) = session
            .create_dashboard_overlay("psstates_live_test", "psstates live test")
            .expect("create overlay");
        f(&session, main);
        session.destroy_overlay(main);
        session.destroy_overlay(thumb);
    }

    /// How long does one SetOverlayRaw take for different canvas sizes?
    #[test]
    #[ignore = "needs a running SteamVR"]
    fn live_raw_upload_timing() {
        with_overlay(|session, overlay| {
            for (w, h) in [(1600u32, 1000u32), (2000, 1250), (2400, 1500)] {
                let mut buf = vec![40u8; (w * h * 4) as usize];
                let mut times = Vec::new();
                for i in 0..20u32 {
                    // Different content every time so nothing can be skipped.
                    let at = (i as usize * 4099) % buf.len();
                    buf[at] ^= 0xff;
                    let start = Instant::now();
                    session.set_overlay_raw(overlay, &mut buf, w, h).unwrap();
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                }
                times.sort_by(|a, b| a.partial_cmp(b).unwrap());
                println!(
                    "SetOverlayRaw {w}x{h}: median {:.1} ms, max {:.1} ms",
                    times[times.len() / 2],
                    times[times.len() - 1]
                );
            }
        });
    }

    /// Does SteamVR accept our D3D11 texture, and what does it cost?
    #[cfg(windows)]
    #[test]
    #[ignore = "needs a running SteamVR"]
    fn live_gpu_texture_upload() {
        use crate::upload::d3d11::GpuTexture;
        with_overlay(|session, overlay| {
            let luid = session.output_device_luid();
            println!("SteamVR output device LUID: {luid:#x}");
            for (w, h) in [(1600u32, 1000u32), (2000, 1250), (2400, 1500)] {
                let texture = GpuTexture::new(luid, w, h).expect("create texture");
                let mut buf = vec![40u8; (w * h * 4) as usize];
                let mut times = Vec::new();
                for i in 0..20u32 {
                    let at = (i as usize * 4099) % buf.len();
                    buf[at] ^= 0xff;
                    let start = Instant::now();
                    texture
                        .upload(session, overlay, &buf)
                        .expect("SteamVR rejected the texture");
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                }
                times.sort_by(|a, b| a.partial_cmp(b).unwrap());
                println!(
                    "SetOverlayTexture {w}x{h}: median {:.1} ms, max {:.1} ms",
                    times[times.len() / 2],
                    times[times.len() - 1]
                );
            }
        });
    }

    /// The uploader must keep the overlay working whatever SteamVR thinks of
    /// the texture: if the GPU path fails it has to fall back to raw pixels.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs a running SteamVR"]
    fn live_uploader_survives_rejected_textures() {
        use crate::upload::{d3d11::GpuTexture, Uploader};
        with_overlay(|session, overlay| {
            let (w, h) = (1280u32, 800u32);
            let mut frame = vec![90u8; (w * h * 4) as usize];

            // A texture on the wrong adapter size class: whatever happens, the
            // next upload has to succeed.
            let wrong_size = GpuTexture::new(session.output_device_luid(), w / 2, h / 2).unwrap();
            let mut uploader = Uploader::Gpu(wrong_size);
            uploader
                .upload(session, overlay, &mut frame, w, h)
                .expect("fallback to raw pixels must succeed");
            assert!(
                matches!(uploader, Uploader::Raw),
                "a mismatched frame must switch to raw uploads"
            );
            uploader
                .upload(session, overlay, &mut frame, w, h)
                .expect("raw upload");

            // The normal path.
            let mut uploader = Uploader::new(session, w, h);
            frame[7] ^= 0xff;
            uploader
                .upload(session, overlay, &mut frame, w, h)
                .expect("normal upload");
        });
    }
}
