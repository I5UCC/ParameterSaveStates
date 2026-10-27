//! System tray icon with "Open" and "Exit".
//!
//! The tray lives on its own thread. On Windows that thread runs a Win32
//! message loop (required by tray-icon); on Linux the StatusNotifierItem
//! backend talks D-Bus on a worker thread, so ours just keeps it alive.

use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::{anyhow, Context, Result};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use crate::{AppCommand, APP_TITLE};

pub struct Tray {
    thread: Option<JoinHandle<()>>,
    stop: Box<dyn Fn() + Send>,
}

impl Tray {
    /// Creates the tray icon. Fails if no tray is available.
    pub fn spawn(
        icon_rgba: (Vec<u8>, u32, u32),
        send: Arc<dyn Fn(AppCommand) + Send + Sync>,
    ) -> Result<Self> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<StopSignal>>();
        let thread = std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || tray_thread(icon_rgba, send, ready_tx))
            .context("Failed to spawn tray thread")?;
        let stop = ready_rx
            .recv()
            .map_err(|_| anyhow!("Tray thread exited unexpectedly"))??;
        Ok(Self {
            thread: Some(thread),
            stop: Box::new(move || stop.signal()),
        })
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        (self.stop)();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Tells the tray thread to exit its loop.
struct StopSignal {
    #[cfg(windows)]
    thread_id: u32,
    #[cfg(not(windows))]
    tx: mpsc::Sender<()>,
}

impl StopSignal {
    fn signal(&self) {
        #[cfg(windows)]
        // SAFETY: posting WM_QUIT to a thread id is always safe.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                self.thread_id,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT,
                0,
                0,
            );
        }
        #[cfg(not(windows))]
        let _ = self.tx.send(());
    }
}

fn tray_thread(
    (rgba, width, height): (Vec<u8>, u32, u32),
    send: Arc<dyn Fn(AppCommand) + Send + Sync>,
    ready: mpsc::Sender<Result<StopSignal>>,
) {
    let open = MenuItem::new("Open", true, None);
    let exit = MenuItem::new("Exit", true, None);
    let menu = Menu::new();
    let built = menu
        .append_items(&[&open, &PredefinedMenuItem::separator(), &exit])
        .map_err(|e| anyhow!("Failed to build tray menu: {e}"))
        .and_then(|()| {
            tray_icon::Icon::from_rgba(rgba, width, height)
                .map_err(|e| anyhow!("Invalid tray icon: {e}"))
        })
        .and_then(|icon| {
            TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip(APP_TITLE)
                .with_icon(icon)
                .with_menu_on_left_click(false)
                .build()
                .map_err(|e| anyhow!("Failed to create tray icon: {e}"))
        });
    let tray = match built {
        Ok(t) => t,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };

    let open_id = open.id().clone();
    let exit_id = exit.id().clone();
    {
        let send = send.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == open_id {
                send(AppCommand::Show);
            } else if event.id == exit_id {
                send(AppCommand::Exit);
            }
        }));
    }
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
        | TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => send(AppCommand::Show),
        _ => {}
    }));

    run_loop(ready);

    MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    drop(tray);
}

#[cfg(windows)]
fn run_loop(ready: mpsc::Sender<Result<StopSignal>>) {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage, MSG,
    };
    // SAFETY: plain Win32 message loop on the current thread.
    unsafe {
        let _ = ready.send(Ok(StopSignal {
            thread_id: GetCurrentThreadId(),
        }));
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(not(windows))]
fn run_loop(ready: mpsc::Sender<Result<StopSignal>>) {
    let (tx, rx) = mpsc::channel();
    let _ = ready.send(Ok(StopSignal { tx }));
    let _ = rx.recv();
}
