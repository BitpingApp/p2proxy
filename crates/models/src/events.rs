use p2p_bandwidth_protocol::TargetAddr;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Events {
    LocalPeerId(PeerId),
    Connection(ConnectionEvents),
    Session(SessionEvents),
    Bandwidth(BandwidthEvents),
    /// Non-fatal error surface for the TUI. Emitted when something the
    /// user could actually fix went wrong — e.g. FindNodes kept returning
    /// zero peers for the configured country/min_bandwidth filter for
    /// several minutes. The proxy continues retrying; the UI displays
    /// the message so the operator knows to tweak `Config.yaml`.
    Error(String),
    /// Updated FindNodes result for a server — the hub returned this set
    /// of candidate peers matching the server's country/bandwidth
    /// filters. Lets the TUI's NETWORK tab show "pool: 7 candidates"
    /// alongside the active destination. Replaces whatever pool was
    /// previously known for `port`.
    ServerPool {
        port: u16,
        peers: Vec<libp2p::PeerId>,
    },
    /// The active destination peer for a server changed. Fired when a
    /// fresh `discover_and_connect_to_peer` lands (initial discovery
    /// or post-disconnect rediscovery). `peer` may be `None` to
    /// represent "discovery in progress, no peer yet".
    ActiveDestination {
        port: u16,
        peer: Option<libp2p::PeerId>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionEvents {
    Connecting,
    Connected(PeerId),
    /// A specific peer's libp2p connection closed. Carries the peer
    /// ID so the TUI can remove just that peer from `state.peers` and
    /// drop it from any server's rotation pool — previously the
    /// variant was a unit (`Disconnected`) which made the TUI clear
    /// the entire peer set on a single peer dropping.
    Disconnected(PeerId),
}

pub type SessionId = uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvents {
    New(SessionId, TargetAddr, PeerId),
    End(SessionId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BandwidthEvents {
    Upload(SessionId, u64),
    Download(SessionId, u64),
}
