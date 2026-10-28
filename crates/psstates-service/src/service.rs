//! The public facade: a cloneable handle that starts networking and exposes
//! the app's operations as fire-and-forget calls.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context, Result};
use psstates_core::Store;
use psstates_osc::discovery::Discovery;
use psstates_osc::osc::{self, OscSender};
use psstates_osc::{server, SERVICE_NAME};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::notices::{Notice, NoticeCursor, NoticeLevel, NoticeLog};
use crate::state::{lock, read, write, AvatarRef, Inner, ParamCache, Status};
use crate::vrchat_connection::event_loop;

/// Cloneable handle to the running service.
#[derive(Clone)]
pub struct Service {
    pub(crate) inner: Arc<Inner>,
}

impl Service {
    /// Starts networking (OSC receiver, OSCQuery server, mDNS) on `rt`.
    /// Must not be called from within the runtime.
    pub fn start(rt: Handle, store: Store) -> Result<Self> {
        Self::start_with(rt, store, true)
    }

    /// Like [`Service::start`], optionally without mDNS (for tests, which must
    /// not pick up a VRChat running on the same machine).
    pub fn start_with(rt: Handle, store: Store, mdns: bool) -> Result<Self> {
        let (socket, osc_port) = rt
            .block_on(osc::bind_receiver())
            .context("Failed to bind OSC receive socket")?;
        let oscquery_port = rt
            .block_on(server::start_oscquery_server(SERVICE_NAME, osc_port))
            .context("Failed to start OSCQuery server")?;
        let sender = OscSender::new().context("Failed to create OSC send socket")?;

        let (osc_tx, osc_rx) = mpsc::unbounded_channel();
        let (disc_tx, disc_rx) = mpsc::unbounded_channel();

        let mut status = Status {
            osc_port,
            oscquery_port,
            osc_target: Some(sender.target()),
            ..Default::default()
        };
        let discovery = if mdns {
            let _guard = rt.enter();
            match Discovery::start(SERVICE_NAME, osc_port, oscquery_port, disc_tx) {
                Ok(d) => Some(d),
                Err(e) => {
                    tracing::error!("mDNS unavailable: {e:#}");
                    status.discovery_error = Some(format!("{e:#}"));
                    None
                }
            }
        } else {
            None
        };
        tracing::info!("OSC receive port {osc_port}, OSCQuery port {oscquery_port}");

        let inner = Arc::new(Inner {
            store,
            rt: rt.clone(),
            sender,
            status: RwLock::new(status),
            connection: RwLock::new(None),
            cache: Mutex::new(ParamCache::default()),
            notices: Mutex::new(NoticeLog::default()),
            on_change: RwLock::new(Vec::new()),
            discovery: Mutex::new(discovery),
            known: Mutex::new(HashMap::new()),
            connecting: AtomicBool::new(false),
        });
        rt.spawn(osc::run_receiver(socket, osc_tx));
        rt.spawn(event_loop(inner.clone(), osc_rx, disc_rx));
        Ok(Self { inner })
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn runtime(&self) -> &Handle {
        &self.inner.rt
    }

    pub fn status(&self) -> Status {
        read(&self.inner.status).clone()
    }

    pub fn current_avatar_id(&self) -> Option<String> {
        self.inner.current_avatar_id()
    }

    /// Registers a callback invoked whenever status or notices change.
    /// Callbacks must be cheap and must not block.
    pub fn add_on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        write(&self.inner.on_change).push(Arc::new(f));
    }

    /// A cursor positioned at the end of the notice log (ignores older notices).
    pub fn notice_cursor(&self) -> NoticeCursor {
        NoticeCursor(lock(&self.inner.notices).next_seq)
    }

    /// Notices published after `cursor`; advances the cursor.
    pub fn notices_since(&self, cursor: &mut NoticeCursor) -> Vec<Notice> {
        lock(&self.inner.notices).since(cursor)
    }

    /// Adds a notice (also used by UIs for their own synchronous operations).
    pub fn notice(&self, level: NoticeLevel, text: impl Into<String>) {
        self.inner.push_notice(level, text.into());
    }

    /// Pretends VRChat is connected with the given avatar loaded. Only meant for
    /// rendering UI previews and tests without a running VRChat.
    #[doc(hidden)]
    pub fn preview(&self, instance: &str, avatar_id: &str, avatar_name: &str) {
        {
            let mut st = write(&self.inner.status);
            st.connected = true;
            st.vrchat_instance = Some(instance.to_string());
            st.current_avatar = Some(AvatarRef {
                id: avatar_id.to_string(),
                name: Some(avatar_name.to_string()),
            });
            st.revision += 1;
        }
        self.inner.notify();
    }

    /// Call after modifying the store directly so that all views reload.
    pub fn notify_data_changed(&self) {
        self.inner.bump_revision();
    }

    /// Saves the current avatar state as a new profile.
    pub fn save_profile(&self, name: Option<String>) {
        let inner = self.inner.clone();
        self.spawn(async move { inner.save_profile(name, None).await });
    }

    /// Replaces an existing profile with the current avatar state.
    pub fn override_profile(&self, name: String) {
        let inner = self.inner.clone();
        self.spawn(async move { inner.save_profile(None, Some(name)).await });
    }

    /// Sends a profile (plus fixed parameters) to VRChat.
    pub fn apply_profile(&self, name: String) {
        let inner = self.inner.clone();
        self.spawn(async move { inner.apply_profile(&name) });
    }

    /// Copies all profiles of the previous avatar to the current one.
    pub fn copy_from_previous(&self) {
        let inner = self.inner.clone();
        self.spawn(async move { inner.copy_from_previous() });
    }

    /// Copies all profiles of `source_avatar` to the current avatar.
    pub fn copy_from_avatar(&self, source_avatar: String) {
        let inner = self.inner.clone();
        self.spawn(async move { inner.copy_from_avatar(&source_avatar) });
    }

    /// Drops the current connection and searches for VRChat again.
    pub fn reconnect(&self) {
        let inner = self.inner.clone();
        self.inner.rt.spawn(async move { inner.reconnect() });
    }

    /// Stops advertising via mDNS. Call before exiting.
    pub fn shutdown(&self) {
        if let Some(d) = lock(&self.inner.discovery).take() {
            d.shutdown();
        }
    }

    /// Runs a module's operation on the runtime and reports its outcome as a notice.
    fn spawn<F>(&self, fut: F)
    where
        F: Future<Output = Result<String>> + Send + 'static,
    {
        let inner = self.inner.clone();
        self.inner.rt.spawn(async move {
            match fut.await {
                Ok(msg) => inner.push_notice(NoticeLevel::Success, msg),
                Err(e) => inner.push_notice(NoticeLevel::Error, format!("{e:#}")),
            }
        });
    }
}
