use libp2p::{Multiaddr, PeerId};
use p2p_bandwidth_protocol::TargetAddr;
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
    /// previously known for `port`. Each candidate carries the dial
    /// routes the hub resolved for it (usually a relay-circuit address;
    /// a direct address when the homing hub advertises one) so the
    /// rotation pool can show where a peer is reachable.
    ServerPool {
        port: u16,
        peers: Vec<PoolPeer>,
    },
    /// A peer's current connection endpoint, observed on every
    /// `ConnectionEstablished` — including the fresh one a DCUtR
    /// hole-punch raises when it upgrades a relayed circuit to a direct
    /// link. Lets the NETWORK tab show the real egress address of the
    /// peer actually carrying traffic instead of just its relay route.
    /// `relayed` is taken from the connection endpoint itself (so an
    /// inbound relayed connection, whose remote address is a bare
    /// `/p2p/<id>` with no circuit marker, is still classified correctly).
    PeerRoute {
        peer_id: PeerId,
        address: Multiaddr,
        relayed: bool,
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
    /// The remembered sticky standby pool for a server (from
    /// `sticky_peers.json`), so the NETWORK tab shows every remembered exit —
    /// not just the active one. Emitted on each discovery pass; replaces the
    /// previously-known sticky pool for `port`.
    StickyPool {
        port: u16,
        peers: Vec<PoolPeer>,
    },
}

/// One candidate in a server's FindNodes pool: the peer plus the dial
/// routes the hub resolved for it this pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolPeer {
    pub peer_id: PeerId,
    pub addresses: Vec<Multiaddr>,
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
