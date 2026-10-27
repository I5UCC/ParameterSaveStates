//! The application service ties VRChat (OSC / OSCQuery / mDNS) to the profile
//! store. It is UI agnostic: the desktop UI, the VR overlay and a possible
//! HTTP API all drive the same [`Service`].
//!
//! * Read state via [`Service::status`] (cheap clone).
//! * Fire-and-forget operations (`save_profile`, `apply_profile`, ...) run on
//!   the tokio runtime and report results as [`Notice`]s.
//! * Pure storage operations go directly through [`Service::store`].
//! * Register [`Service::add_on_change`] to get notified (e.g. to request a repaint).
//! * Each UI reads notices with its own [`NoticeCursor`].

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use psstates_core::model::display_avatar;
use psstates_core::rules::{
    effective_fixed_parameters, fixed_messages, profile_messages, select_auto_sync, NameSet,
};
use psstates_core::{ParamValue, Store};
use psstates_osc::discovery::{Discovery, DiscoveryEvent};
use psstates_osc::osc::{self, IncomingMessage, OscSender};
use psstates_osc::oscquery::{HostInfo, OscQueryClient};
use psstates_osc::{server, SERVICE_NAME, VRCHAT_DEFAULT_OSC_PORT};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

/// Application version (overridable at build time via `PSSTATES_VERSION`).
pub const VERSION: &str = match option_env!("PSSTATES_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

const AVATAR_CHANGE: &str = "/avatar/change";
const AVATAR_PARAMETERS: &str = "/avatar/parameters/";
/// How often a live connection to VRChat is checked.
const HEALTH_INTERVAL: Duration = Duration::from_secs(5);
/// How often we retry connecting to VRChat instances we already know about.
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
/// While not connected, mDNS is queried again every this many retry ticks.
const REFRESH_EVERY_TICKS: u32 = 5;
const MAX_NOTICES: usize = 50;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: NoticeLevel,
    pub text: String,
}

struct Connection {
    instance: String,
    client: OscQueryClient,
}

#[derive(Default)]
struct ParamCache {
    /// Values of the current avatar as reported by VRChat.
    current: HashMap<String, ParamValue>,
    /// Values of the previous avatar (used for auto-sync).
    previous: HashMap<String, ParamValue>,
}

type ChangeCallback = Arc<dyn Fn() + Send + Sync>;

struct Inner {
    store: Store,
    rt: Handle,
    sender: OscSender,
    status: RwLock<Status>,
    connection: RwLock<Option<Connection>>,
    cache: Mutex<ParamCache>,
    /// Recent notices with sequence numbers; every UI keeps its own cursor.
    notices: Mutex<NoticeLog>,
    on_change: RwLock<Vec<ChangeCallback>>,
    discovery: Mutex<Option<Discovery>>,
    /// VRChat instances seen via mDNS. They stay known until VRChat says goodbye
    /// or stops answering, so a failed connection attempt can be retried.
    known: Mutex<HashMap<String, Vec<SocketAddr>>>,
    /// A connection attempt is running.
    connecting: AtomicBool,
}

#[derive(Default)]
struct NoticeLog {
    next_seq: u64,
    entries: VecDeque<(u64, Notice)>,
}

/// Per-consumer position in the notice log (see [`Service::notices_since`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoticeCursor(u64);

fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}
fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}
fn lock<T>(l: &Mutex<T>) -> MutexGuard<'_, T> {
    l.lock().unwrap_or_else(|e| e.into_inner())
}

/// Cloneable handle to the running service.
#[derive(Clone)]
pub struct Service {
    inner: Arc<Inner>,
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
        let log = lock(&self.inner.notices);
        let out = log
            .entries
            .iter()
            .filter(|(seq, _)| *seq >= cursor.0)
            .map(|(_, n)| n.clone())
            .collect();
        cursor.0 = log.next_seq;
        out
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
        self.spawn(async move {
            let (current, previous) = {
                let st = read(&inner.status);
                (st.current_avatar.clone(), st.previous_avatar.clone())
            };
            let current = current.ok_or_else(|| anyhow!("No current avatar detected"))?;
            let previous = previous.ok_or_else(|| anyhow!("There is no previous avatar"))?;
            inner.copy_profiles(&previous.id, &current.id)
        });
    }

    /// Copies all profiles of `source_avatar` to the current avatar.
    pub fn copy_from_avatar(&self, source_avatar: String) {
        let inner = self.inner.clone();
        self.spawn(async move {
            let current = inner
                .current_avatar_id()
                .ok_or_else(|| anyhow!("No current avatar detected"))?;
            inner.copy_profiles(&source_avatar, &current)
        });
    }

    /// Drops the current connection and searches for VRChat again.
    pub fn reconnect(&self) {
        let inner = self.inner.clone();
        self.inner.rt.spawn(async move {
            inner.disconnect("Reconnecting...");
            inner.refresh_discovery();
            // Instances we already know about can be tried right away.
            inner.spawn_connect();
        });
    }

    /// Stops advertising via mDNS. Call before exiting.
    pub fn shutdown(&self) {
        if let Some(d) = lock(&self.inner.discovery).take() {
            d.shutdown();
        }
    }

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

async fn event_loop(
    inner: Arc<Inner>,
    mut osc_rx: mpsc::UnboundedReceiver<IncomingMessage>,
    mut disc_rx: mpsc::UnboundedReceiver<DiscoveryEvent>,
) {
    let mut health = tokio::time::interval(HEALTH_INTERVAL);
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut retry = tokio::time::interval(RETRY_INTERVAL);
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut failures = 0u32;
    let mut idle_ticks = 0u32;
    loop {
        tokio::select! {
            Some(msg) = osc_rx.recv() => inner.handle_osc(msg),
            Some(ev) = disc_rx.recv() => inner.handle_discovery(ev),
            _ = health.tick() => inner.health_check(&mut failures).await,
            _ = retry.tick() => inner.retry_tick(&mut idle_ticks).await,
            else => break,
        }
    }
}

impl Inner {
    fn notify(&self) {
        let callbacks = read(&self.on_change).clone();
        for cb in callbacks {
            cb();
        }
    }

    fn push_notice(&self, level: NoticeLevel, text: String) {
        match level {
            NoticeLevel::Error => tracing::warn!("{text}"),
            _ => tracing::info!("{text}"),
        }
        {
            let mut log = lock(&self.notices);
            let seq = log.next_seq;
            log.next_seq += 1;
            log.entries.push_back((seq, Notice { level, text }));
            while log.entries.len() > MAX_NOTICES {
                log.entries.pop_front();
            }
        }
        self.notify();
    }

    fn bump_revision(&self) {
        write(&self.status).revision += 1;
        self.notify();
    }

    fn set_busy(&self, busy: Option<&str>) {
        write(&self.status).busy = busy.map(str::to_string);
        self.notify();
    }

    fn current_avatar_id(&self) -> Option<String> {
        read(&self.status)
            .current_avatar
            .as_ref()
            .map(|a| a.id.clone())
    }

    fn client(&self) -> Option<OscQueryClient> {
        read(&self.connection).as_ref().map(|c| c.client.clone())
    }

    fn refresh_discovery(&self) {
        if let Some(d) = lock(&self.discovery).as_ref() {
            if let Err(e) = d.refresh() {
                tracing::warn!("mDNS refresh failed: {e:#}");
            }
        }
    }

    // ------------------------------------------------------------- incoming

    fn handle_osc(self: &Arc<Self>, msg: IncomingMessage) {
        if msg.addr == AVATAR_CHANGE {
            if let Some(id) = msg.first_string().filter(|s| !s.is_empty()) {
                self.on_avatar_change(id.to_string(), false);
            }
        } else if msg.addr.starts_with(AVATAR_PARAMETERS) {
            if let Some(v) = msg.param_value() {
                lock(&self.cache).current.insert(msg.addr, v);
            }
        }
    }

    /// Handles an avatar change. With `force`, fixed parameters are re-applied
    /// even if the avatar did not change (e.g. after reconnecting).
    fn on_avatar_change(self: &Arc<Self>, id: String, force: bool) {
        let changed = {
            let mut st = write(&self.status);
            let same = st.current_avatar.as_ref().is_some_and(|a| a.id == id);
            if same && !force {
                return;
            }
            if !same {
                let name = self.store.avatar(&id).ok().and_then(|a| a.name);
                st.previous_avatar = st.current_avatar.take();
                st.current_avatar = Some(AvatarRef {
                    id: id.clone(),
                    name,
                });
                st.revision += 1;
            }
            !same
        };
        tracing::info!("Avatar changed to {id}");

        if changed {
            let mut cache = lock(&self.cache);
            // Keep the older snapshot if nothing was received for the avatar
            // in between (rapid switching).
            if !cache.current.is_empty() {
                cache.previous = std::mem::take(&mut cache.current);
            }
        }
        if changed {
            self.apply_auto_sync(&id);
        }
        self.apply_fixed(&id);
        self.notify();

        let me = self.clone();
        self.rt
            .spawn(async move { me.resolve_avatar_name(id).await });
    }

    fn apply_auto_sync(&self, avatar_id: &str) {
        let (Ok(settings), Ok(avatar)) = (self.store.settings(), self.store.avatar(avatar_id))
        else {
            return;
        };
        let names = NameSet::new(
            settings
                .auto_sync_parameters
                .iter()
                .chain(&avatar.auto_sync_parameters),
        );
        if names.is_empty() {
            return;
        }
        let msgs = select_auto_sync(&lock(&self.cache).previous, &names);
        let sent = self.sender.send_all(&msgs);
        tracing::info!(
            "Auto-sync for avatar '{avatar_id}': {} configured name(s), {} matched, {sent} sent",
            names.len(),
            msgs.len()
        );
    }

    fn apply_fixed(&self, avatar_id: &str) -> usize {
        let (Ok(settings), Ok(avatar)) = (self.store.settings(), self.store.avatar(avatar_id))
        else {
            return 0;
        };
        let fixed =
            effective_fixed_parameters(&settings.fixed_parameters, &avatar.fixed_parameters);
        if fixed.is_empty() {
            return 0;
        }
        let sent = self.sender.send_all(&fixed_messages(&fixed));
        tracing::info!(
            "Fixed parameters for avatar '{avatar_id}': {sent}/{} sent",
            fixed.len()
        );
        sent
    }

    async fn resolve_avatar_name(self: Arc<Self>, id: String) {
        // Give VRChat a moment to update its OSCQuery tree.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let Some(client) = self.client() else { return };
        let name = match client.current_avatar_name().await {
            Ok(Some(name)) => name,
            Ok(None) => return,
            Err(e) => {
                tracing::debug!("Could not read avatar name: {e:#}");
                return;
            }
        };
        if let Err(e) = self.store.set_avatar_name(&id, &name) {
            tracing::warn!("Failed to store avatar name: {e}");
        }
        {
            let mut st = write(&self.status);
            match st.current_avatar.as_mut() {
                Some(a) if a.id == id => a.name = Some(name),
                _ => return,
            }
            st.revision += 1;
        }
        self.notify();
    }

    fn handle_discovery(self: &Arc<Self>, ev: DiscoveryEvent) {
        match ev {
            DiscoveryEvent::Found { instance, addrs } => {
                lock(&self.known).insert(instance, addrs);
                self.spawn_connect();
            }
            DiscoveryEvent::Lost { instance } => {
                lock(&self.known).remove(&instance);
                if self.connected_instance().as_deref() == Some(instance.as_str()) {
                    self.disconnect("VRChat disconnected");
                }
            }
        }
    }

    fn connected_instance(&self) -> Option<String> {
        read(&self.connection).as_ref().map(|c| c.instance.clone())
    }

    /// Tries to connect to a known VRChat instance in the background.
    ///
    /// VRChat announces itself via mDNS slightly before its OSCQuery HTTP server
    /// answers, so a single attempt is not enough; [`Inner::retry_tick`] keeps
    /// calling this until a connection is established.
    fn spawn_connect(self: &Arc<Self>) {
        if read(&self.connection).is_some() || lock(&self.known).is_empty() {
            return;
        }
        if self.connecting.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        self.rt.spawn(async move {
            me.connect_known().await;
            me.connecting.store(false, Ordering::SeqCst);
        });
    }

    async fn connect_known(self: &Arc<Self>) {
        let candidates: Vec<(String, Vec<SocketAddr>)> = lock(&self.known)
            .iter()
            .map(|(instance, addrs)| (instance.clone(), addrs.clone()))
            .collect();
        for (instance, addrs) in candidates {
            for addr in addrs {
                let client = OscQueryClient::new(addr);
                match client.host_info().await {
                    Ok(info) => {
                        self.connect(instance, client, info).await;
                        return;
                    }
                    Err(e) => tracing::debug!("{instance} not reachable at {addr}: {e:#}"),
                }
            }
        }
    }

    async fn connect(self: &Arc<Self>, instance: String, client: OscQueryClient, info: HostInfo) {
        let ip = info
            .osc_ip
            .parse::<IpAddr>()
            .ok()
            .filter(|ip| !ip.is_unspecified())
            .unwrap_or_else(|| client.addr().ip());
        let port = if info.osc_port != 0 {
            info.osc_port
        } else {
            VRCHAT_DEFAULT_OSC_PORT
        };
        let target = SocketAddr::new(ip, port);
        self.sender.set_target(target);
        *write(&self.connection) = Some(Connection {
            instance: instance.clone(),
            client: client.clone(),
        });
        {
            let mut st = write(&self.status);
            st.connected = true;
            st.vrchat_instance = Some(instance.clone());
            st.osc_target = Some(target);
            st.revision += 1;
        }
        self.push_notice(NoticeLevel::Info, format!("Connected to {instance}"));

        match client.current_avatar_id().await {
            Ok(Some(id)) => self.on_avatar_change(id, true),
            Ok(None) => {}
            Err(e) => tracing::warn!("Could not read current avatar: {e:#}"),
        }
    }

    fn disconnect(&self, reason: &str) {
        let had = write(&self.connection).take().is_some();
        {
            let mut st = write(&self.status);
            st.connected = false;
            st.vrchat_instance = None;
            st.revision += 1;
        }
        if had {
            self.push_notice(NoticeLevel::Info, reason.to_string());
        } else {
            self.notify();
        }
    }

    /// Checks that a live connection still answers.
    async fn health_check(&self, failures: &mut u32) {
        let Some(client) = self.client() else {
            *failures = 0;
            return;
        };
        match client.host_info().await {
            Ok(_) => *failures = 0,
            Err(e) => {
                *failures += 1;
                tracing::debug!("VRChat health check failed ({failures}): {e:#}");
                if *failures >= 2 {
                    *failures = 0;
                    // Forget the instance; mDNS brings it back if it is still there.
                    if let Some(instance) = self.connected_instance() {
                        lock(&self.known).remove(&instance);
                    }
                    self.disconnect("Lost connection to VRChat");
                    self.refresh_discovery();
                }
            }
        }
    }

    /// Runs every [`RETRY_INTERVAL`]: retries known instances while not
    /// connected and fetches the avatar if VRChat had none loaded yet.
    async fn retry_tick(self: &Arc<Self>, idle_ticks: &mut u32) {
        let Some(client) = self.client() else {
            *idle_ticks += 1;
            self.spawn_connect();
            if idle_ticks.is_multiple_of(REFRESH_EVERY_TICKS) {
                self.refresh_discovery();
            }
            return;
        };
        *idle_ticks = 0;
        if self.current_avatar_id().is_none() {
            match client.current_avatar_id().await {
                Ok(Some(id)) => self.on_avatar_change(id, true),
                Ok(None) => {}
                Err(e) => tracing::debug!("Could not read current avatar: {e:#}"),
            }
        }
    }

    // ----------------------------------------------------------- operations

    async fn save_profile(
        &self,
        name: Option<String>,
        override_name: Option<String>,
    ) -> Result<String> {
        let avatar_id = self
            .current_avatar_id()
            .ok_or_else(|| anyhow!("No current avatar detected"))?;
        let client = self
            .client()
            .ok_or_else(|| anyhow!("Unable to save profile. Is VRChat connected?"))?;

        self.set_busy(Some("Saving profile..."));
        let params: Result<BTreeMap<String, ParamValue>> = client.avatar_parameters().await;
        self.set_busy(None);
        let params = params.context("Unable to read avatar parameters from VRChat")?;
        if params.is_empty() {
            bail!("VRChat reported no avatar parameters");
        }
        let count = params.len();

        let msg = match override_name {
            Some(target) => {
                self.store
                    .update_avatar(&avatar_id, |a| a.override_profile(&target, params))?;
                format!("Overrode profile '{target}' ({count} parameters)")
            }
            None => {
                let final_name = self
                    .store
                    .update_avatar(&avatar_id, |a| a.add_profile(name.as_deref(), params))?;
                format!("Saved profile '{final_name}' ({count} parameters)")
            }
        };
        self.bump_revision();
        Ok(msg)
    }

    fn apply_profile(&self, name: &str) -> Result<String> {
        let avatar_id = self
            .current_avatar_id()
            .ok_or_else(|| anyhow!("No current avatar detected"))?;
        let settings = self.store.settings()?;
        let avatar = self.store.avatar(&avatar_id)?;
        let profile = avatar
            .find_profile(name)
            .ok_or_else(|| anyhow!("Profile '{name}' not found"))?;
        let excluded = NameSet::new(
            settings
                .excluded_parameters
                .iter()
                .chain(&avatar.excluded_parameters),
        );
        let msgs = profile_messages(profile, &excluded);
        let sent = self.sender.send_all(&msgs);
        let skipped = profile.parameters.len() - msgs.len();
        let fixed = self.apply_fixed(&avatar_id);

        let mut text = format!("Applied '{name}': {sent} parameter(s)");
        if skipped > 0 {
            text.push_str(&format!(", {skipped} excluded"));
        }
        if fixed > 0 {
            text.push_str(&format!(", {fixed} fixed"));
        }
        if !read(&self.status).connected {
            text.push_str(" (VRChat not connected)");
        }
        Ok(text)
    }

    fn copy_profiles(&self, from: &str, to: &str) -> Result<String> {
        if from == to {
            bail!("Source and target avatar are the same");
        }
        let source = self.store.avatar(from)?;
        if source.profiles.is_empty() {
            bail!("Selected avatar has no saved profiles");
        }
        let copied = self
            .store
            .update_avatar(to, |a| Ok(a.copy_profiles_from(&source)))?;
        self.bump_revision();
        Ok(format!(
            "Copied {copied} profile(s) from {}",
            source.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use psstates_core::model::FixedParam;
    use std::net::Ipv4Addr;

    fn service() -> (tokio::runtime::Runtime, Service, tempfile::TempDir) {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        // No mDNS: a VRChat running on the developer machine must not interfere.
        let svc = Service::start_with(rt.handle().clone(), store, false).unwrap();
        (rt, svc, dir)
    }

    #[test]
    fn avatar_change_applies_auto_sync_and_fixed() {
        let (rt, svc, _dir) = service();
        // Capture what we send.
        let sink = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        sink.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        svc.inner.sender.set_target(sink.local_addr().unwrap());

        svc.store()
            .update_settings(|s| {
                s.auto_sync_parameters = vec!["Hue".into()];
                s.fixed_parameters = vec![FixedParam::new("Glasses", ParamValue::Bool(true))];
                Ok(())
            })
            .unwrap();

        let inner = svc.inner.clone();
        rt.block_on(async move {
            inner.on_avatar_change("avtr_a".into(), false);
            inner.handle_osc(IncomingMessage {
                addr: "/avatar/parameters/Hue".into(),
                args: vec![rosc::OscType::Float(0.75)],
            });
            inner.on_avatar_change("avtr_b".into(), false);
        });

        let mut received = Vec::new();
        let mut buf = [0u8; 1024];
        // avtr_a: fixed; avtr_b: auto-sync Hue + fixed.
        for _ in 0..3 {
            let (n, _) = sink.recv_from(&mut buf).unwrap();
            match rosc::decoder::decode_udp(&buf[..n]).unwrap().1 {
                rosc::OscPacket::Message(m) => received.push(m.addr),
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(
            received,
            [
                "/avatar/parameters/Glasses",
                "/avatar/parameters/Hue",
                "/avatar/parameters/Glasses"
            ]
        );
        let st = svc.status();
        assert_eq!(st.current_avatar.unwrap().id, "avtr_b");
        assert_eq!(st.previous_avatar.unwrap().id, "avtr_a");
        svc.shutdown();
    }

    /// VRChat announces itself via mDNS before its OSCQuery HTTP server answers.
    /// The service must keep trying instead of giving up after the first failure.
    #[test]
    fn connects_when_vrchat_oscquery_comes_up_late() {
        use psstates_osc::oscquery::{OscQueryNode, ACCESS_READ_WRITE};
        use psstates_osc::server::{host_info, serve_oscquery};

        let (rt, svc, _dir) = service();

        // Reserve a port, then free it so the first connection attempts are refused.
        let port = {
            let l = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            l.local_addr().unwrap().port()
        };
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let inner = svc.inner.clone();
        rt.block_on(async {
            inner.handle_discovery(DiscoveryEvent::Found {
                instance: "VRChat-Client-TEST".into(),
                addrs: vec![addr],
            });
        });
        // Wait until the first attempt has completely failed (Windows retransmits a
        // refused SYN for a while), so only the retry path can make this test pass.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while svc.inner.connecting.load(Ordering::SeqCst) {
            assert!(
                std::time::Instant::now() < deadline,
                "first attempt never finished"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!svc.status().connected, "nothing is listening yet");

        // VRChat's OSCQuery server finally starts, with an avatar already loaded.
        let mut tree = OscQueryNode::container("/");
        tree.insert(
            "/avatar/change",
            OscQueryNode {
                full_path: "/avatar/change".into(),
                access: Some(ACCESS_READ_WRITE),
                osc_type: Some("s".into()),
                value: Some(vec![serde_json::json!("avtr_late")]),
                ..Default::default()
            },
        );
        rt.block_on(async {
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            serve_oscquery(listener, host_info("VRChat-Client-TEST", 9000), tree);
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        loop {
            let st = svc.status();
            if st.connected
                && st
                    .current_avatar
                    .as_ref()
                    .is_some_and(|a| a.id == "avtr_late")
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "did not connect in time: {st:?}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        svc.shutdown();
    }
}
