//! mDNS advertising (so VRChat finds us) and browsing (so we find VRChat).

use std::net::{IpAddr, SocketAddr};

use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::mpsc;

use crate::VRCHAT_INSTANCE_PREFIX;

pub const OSC_SERVICE_TYPE: &str = "_osc._udp.local.";
pub const OSCQUERY_SERVICE_TYPE: &str = "_oscjson._tcp.local.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A VRChat OSCQuery service was resolved. Addresses are ordered by
    /// preference (loopback first).
    Found {
        instance: String,
        addrs: Vec<SocketAddr>,
    },
    /// A VRChat OSCQuery service went away.
    Lost { instance: String },
}

/// Extracts the instance name from a full mDNS service name.
fn instance_name(fullname: &str) -> &str {
    fullname
        .strip_suffix(OSCQUERY_SERVICE_TYPE)
        .map(|s| s.trim_end_matches('.'))
        .unwrap_or(fullname)
}

fn is_vrchat(instance: &str) -> bool {
    instance.starts_with(VRCHAT_INSTANCE_PREFIX)
}

fn sort_addrs(mut addrs: Vec<IpAddr>, port: u16) -> Vec<SocketAddr> {
    addrs.sort_by_key(|ip| (!ip.is_loopback(), !ip.is_ipv4()));
    addrs.dedup();
    addrs
        .into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect()
}

pub struct Discovery {
    daemon: ServiceDaemon,
    registered: Vec<String>,
    events: mpsc::UnboundedSender<DiscoveryEvent>,
}

impl Discovery {
    /// Advertises our OSC and OSCQuery services and starts browsing for VRChat.
    pub fn start(
        instance: &str,
        osc_port: u16,
        oscquery_port: u16,
        events: mpsc::UnboundedSender<DiscoveryEvent>,
    ) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("Failed to start mDNS daemon")?;
        let host = format!("psstates-{}.local.", std::process::id());
        let mut registered = Vec::new();
        for (ty, port) in [
            (OSC_SERVICE_TYPE, osc_port),
            (OSCQUERY_SERVICE_TYPE, oscquery_port),
        ] {
            let info = ServiceInfo::new(
                ty,
                instance,
                &host,
                "",
                port,
                None::<std::collections::HashMap<String, String>>,
            )
            .context("Invalid mDNS service info")?
            .enable_addr_auto();
            registered.push(info.get_fullname().to_string());
            daemon
                .register(info)
                .with_context(|| format!("Failed to advertise {ty}"))?;
        }
        let me = Self {
            daemon,
            registered,
            events,
        };
        me.browse()?;
        Ok(me)
    }

    fn browse(&self) -> Result<()> {
        let receiver = self
            .daemon
            .browse(OSCQUERY_SERVICE_TYPE)
            .context("Failed to browse for OSCQuery services")?;
        let events = self.events.clone();
        std::thread::Builder::new()
            .name("mdns-browse".into())
            .spawn(move || {
                while let Ok(event) = receiver.recv() {
                    let out = match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let instance = instance_name(info.get_fullname()).to_string();
                            if !is_vrchat(&instance) {
                                continue;
                            }
                            let addrs = sort_addrs(
                                info.get_addresses().iter().copied().collect(),
                                info.get_port(),
                            );
                            tracing::info!("Found {instance} at {addrs:?}");
                            DiscoveryEvent::Found { instance, addrs }
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            let instance = instance_name(&fullname).to_string();
                            if !is_vrchat(&instance) {
                                continue;
                            }
                            tracing::info!("Lost {instance}");
                            DiscoveryEvent::Lost { instance }
                        }
                        ServiceEvent::SearchStopped(_) => break,
                        _ => continue,
                    };
                    if events.send(out).is_err() {
                        break;
                    }
                }
            })
            .context("Failed to spawn mDNS browse thread")?;
        Ok(())
    }

    /// Restarts browsing, which re-queries the network for VRChat instances.
    pub fn refresh(&self) -> Result<()> {
        let _ = self.daemon.stop_browse(OSCQUERY_SERVICE_TYPE);
        self.browse()
    }

    pub fn shutdown(&self) {
        for name in &self.registered {
            let _ = self.daemon.unregister(name);
        }
        let _ = self.daemon.stop_browse(OSCQUERY_SERVICE_TYPE);
        let _ = self.daemon.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn names_and_addr_order() {
        assert_eq!(
            instance_name("VRChat-Client-ABC123._oscjson._tcp.local."),
            "VRChat-Client-ABC123"
        );
        assert!(is_vrchat("VRChat-Client-ABC123"));
        assert!(!is_vrchat("VRCParameterSaveStates"));
        let addrs = sort_addrs(
            vec![
                IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)),
                IpAddr::V4(Ipv4Addr::LOCALHOST),
            ],
            80,
        );
        assert_eq!(addrs[0].ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
}
