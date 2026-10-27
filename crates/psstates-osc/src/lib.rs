//! Networking used to talk to VRChat:
//!
//! * [`osc`]: plain OSC over UDP (send parameter values, receive changes)
//! * [`oscquery`]: OSCQuery JSON model + HTTP client (read VRChat's tree)
//! * [`server`]: our own minimal OSCQuery HTTP server
//! * [`discovery`]: mDNS advertising / browsing

pub mod discovery;
pub mod osc;
pub mod oscquery;
pub mod server;

/// Service name advertised via mDNS / OSCQuery.
pub const SERVICE_NAME: &str = "VRCParameterSaveStates";

/// Prefix of the mDNS instance name VRChat clients use.
pub const VRCHAT_INSTANCE_PREFIX: &str = "VRChat-Client";

/// VRChat's default OSC input port (used if HOST_INFO is unavailable).
pub const VRCHAT_DEFAULT_OSC_PORT: u16 = 9000;
