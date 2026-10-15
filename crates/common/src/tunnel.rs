//! Constants shared by the GUI (which writes the leaf config) and the
//! privileged core (which sets up routing around it), plus the line protocol
//! the core speaks on stdout.

use serde::{Deserialize, Serialize};

pub const TUN_NAME: &str = "nthlink0";
pub const TUN_ADDR: &str = "172.19.0.1";
pub const TUN_PEER: &str = "172.19.0.2";
pub const TUN_NETMASK: &str = "255.255.255.252";
pub const TUN_MTU: u32 = 1500;
/// The TUN's own /30. Must never be routed `Direct`: leaf's direct socket
/// would send it straight back into the TUN.
pub const TUN_CIDR: &str = "172.19.0.0/30";
/// Address handed to the system resolver; any UDP/53 that reaches the TUN is
/// answered by leaf's fake DNS.
pub const TUN_DNS: &str = TUN_PEER;

/// Firewall mark leaf puts on its own sockets ("nt").
pub const FWMARK: u32 = 0x6e74;
/// Routing table holding the default route into the TUN.
pub const ROUTE_TABLE: u32 = 0x6e74;
/// `ip rule` priorities used by the core (all removed on cleanup).
pub const RULE_PREF_MARK: u32 = 5207;
pub const RULE_PREF_SUPPRESS: u32 = 5208;
pub const RULE_PREF_TABLE: u32 = 5209;

/// One JSON object per line on the core's stdout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CoreEvent {
    Starting,
    Connected { ipv6: bool, dns: String },
    Log { level: String, message: String },
    Error { message: String },
    Stopped,
}

impl CoreEvent {
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"event\":\"error\",\"message\":\"encode\"}".into())
    }
}
