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
    /// fresh discover-and-connect lands (initial discovery or
    /// post-disconnect rediscovery). `peer` may be `None` to
    /// represent "discovery in progress, no peer yet"; `source` says how
    /// the peer was chosen (pinned rank / sticky reuse / fresh discovery).
    ActiveDestination {
        port: u16,
        peer: Option<libp2p::PeerId>,
        source: Option<DestinationSource>,
    },
    /// Per-rank resolvability of a server's pinned preference list
    /// (BIT-597), refreshed on every pinned connect pass. Lets the
    /// NETWORK tab flag stale entries ("pinned peer no longer reachable
    /// anywhere in the mesh") before the operator notices traffic moved
    /// to a lower-preference peer.
    PinnedPeerStatuses {
        port: u16,
        statuses: Vec<PinnedPeerStatus>,
    },
}

/// How a server's active destination was selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DestinationSource {
    /// From the `destination_peers` list; `rank` 0 is the most preferred.
    Pinned { rank: usize },
    /// Remembered exit peer reused from the sticky store.
    Sticky,
    /// Fresh attribute-filtered FindNodes discovery.
    Discovered,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedPeerStatus {
    pub peer_id: PeerId,
    pub rank: usize,
    /// `true` when the peer currently has a dialable route (hub-resolved or
    /// verbatim multiaddr); `false` means stale — not connected anywhere in
    /// the reachable hub mesh.
    pub resolvable: bool,
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
