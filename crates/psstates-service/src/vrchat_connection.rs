//! Finding VRChat (mDNS), connecting to its OSCQuery server, and keeping that
//! connection healthy.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use psstates_osc::discovery::DiscoveryEvent;
use psstates_osc::osc::IncomingMessage;
use psstates_osc::oscquery::{HostInfo, OscQueryClient};
use psstates_osc::VRCHAT_DEFAULT_OSC_PORT;
use tokio::sync::mpsc;

use crate::notices::NoticeLevel;
use crate::state::{lock, read, write, Connection, Inner};

/// How often a live connection to VRChat is checked.
const HEALTH_INTERVAL: Duration = Duration::from_secs(5);
/// How often we retry connecting to VRChat instances we already know about.
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
/// While not connected, mDNS is queried again every this many retry ticks.
const REFRESH_EVERY_TICKS: u32 = 5;

pub(crate) async fn event_loop(
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
    pub(crate) fn handle_discovery(self: &Arc<Self>, ev: DiscoveryEvent) {
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

    pub(crate) fn disconnect(&self, reason: &str) {
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

    /// Drops the current connection and searches for VRChat again.
    pub(crate) fn reconnect(self: &Arc<Self>) {
        self.disconnect("Reconnecting...");
        self.refresh_discovery();
        // Instances we already know about can be tried right away.
        self.spawn_connect();
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
}
