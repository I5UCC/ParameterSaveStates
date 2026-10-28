//! Shared state behind a [`crate::Service`]: what is connected, which avatar
//! is loaded, and the plumbing every module relies on.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use psstates_core::model::display_avatar;
use psstates_core::{ParamValue, Store};
use psstates_osc::discovery::Discovery;
use psstates_osc::osc::OscSender;
use psstates_osc::oscquery::OscQueryClient;
use tokio::runtime::Handle;

use crate::notices::NoticeLog;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarRef {
    pub id: String,
    pub name: Option<String>,
}

impl AvatarRef {
    pub fn display(&self) -> String {
        display_avatar(&self.id, self.name.as_deref())
    }

    /// Name if known, otherwise the id.
    pub fn short(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// Snapshot of the service state.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub connected: bool,
    pub vrchat_instance: Option<String>,
    pub current_avatar: Option<AvatarRef>,
    pub previous_avatar: Option<AvatarRef>,
    /// Port we receive OSC on.
    pub osc_port: u16,
    /// Port of our OSCQuery server.
    pub oscquery_port: u16,
    /// Where OSC messages are sent to.
    pub osc_target: Option<SocketAddr>,
    /// Long running operation in progress (e.g. "Saving profile...").
    pub busy: Option<String>,
    /// Incremented whenever stored data or the avatar changes, so views know
    /// when to reload cached data.
    pub revision: u64,
    pub discovery_error: Option<String>,
}

pub(crate) struct Connection {
    pub(crate) instance: String,
    pub(crate) client: OscQueryClient,
}

#[derive(Default)]
pub(crate) struct ParamCache {
    /// Values of the current avatar as reported by VRChat.
    pub(crate) current: HashMap<String, ParamValue>,
    /// Values of the previous avatar (used for auto-sync).
    pub(crate) previous: HashMap<String, ParamValue>,
}

pub(crate) type ChangeCallback = Arc<dyn Fn() + Send + Sync>;

pub(crate) struct Inner {
    pub(crate) store: Store,
    pub(crate) rt: Handle,
    pub(crate) sender: OscSender,
    pub(crate) status: RwLock<Status>,
    pub(crate) connection: RwLock<Option<Connection>>,
    pub(crate) cache: Mutex<ParamCache>,
    /// Recent notices with sequence numbers; every UI keeps its own cursor.
    pub(crate) notices: Mutex<NoticeLog>,
    pub(crate) on_change: RwLock<Vec<ChangeCallback>>,
    pub(crate) discovery: Mutex<Option<Discovery>>,
    /// VRChat instances seen via mDNS. They stay known until VRChat says goodbye
    /// or stops answering, so a failed connection attempt can be retried.
    pub(crate) known: Mutex<HashMap<String, Vec<SocketAddr>>>,
    /// A connection attempt is running.
    pub(crate) connecting: AtomicBool,
}

pub(crate) fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}
pub(crate) fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}
pub(crate) fn lock<T>(l: &Mutex<T>) -> MutexGuard<'_, T> {
    l.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inner {
    pub(crate) fn notify(&self) {
        let callbacks = read(&self.on_change).clone();
        for cb in callbacks {
            cb();
        }
    }

    pub(crate) fn bump_revision(&self) {
        write(&self.status).revision += 1;
        self.notify();
    }

    pub(crate) fn set_busy(&self, busy: Option<&str>) {
        write(&self.status).busy = busy.map(str::to_string);
        self.notify();
    }

    pub(crate) fn current_avatar_id(&self) -> Option<String> {
        read(&self.status)
            .current_avatar
            .as_ref()
            .map(|a| a.id.clone())
    }

    pub(crate) fn client(&self) -> Option<OscQueryClient> {
        read(&self.connection).as_ref().map(|c| c.client.clone())
    }

    pub(crate) fn refresh_discovery(&self) {
        if let Some(d) = lock(&self.discovery).as_ref() {
            if let Err(e) = d.refresh() {
                tracing::warn!("mDNS refresh failed: {e:#}");
            }
        }
    }
}
