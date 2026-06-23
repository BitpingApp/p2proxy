use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use libp2p::{Multiaddr, PeerId, multiaddr::Protocol};

#[derive(Default)]
pub enum ConnectionStatus {
    #[default]
    Connecting,
    Connected(PeerId),
    Disconnected,
}

impl ConnectionStatus {
    pub fn as_str(&self) -> &str {
        match self {
            ConnectionStatus::Connecting => "Connecting...",
            ConnectionStatus::Connected(_) => "Connected",
            ConnectionStatus::Disconnected => "Disconnected",
        }
    }
}

pub type SessionId = uuid::Uuid;

/// Where a peer's known address came from, ordered worst → best. A
/// confirmed live route always wins over a hub-advertised candidate, and
/// a direct hole-punched route wins over a relay circuit — so the NETWORK
/// tab shows the most authoritative address we have for each peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AddrSource {
    /// A dial route the hub returned for a FindNodes candidate — reachable
    /// in principle, but we've never connected to confirm it.
    Candidate,
    /// A live connection that's still riding a relay circuit.
    Relayed,
    /// A live, direct (DCUtR-upgraded) connection — the peer's real egress.
    Direct,
}

impl AddrSource {
    /// Classify a candidate dial route by whether it hops through a relay
    /// circuit. Live routes are classified from the endpoint instead.
    pub fn classify_candidate(addr: &Multiaddr) -> Self {
        if addr.iter().any(|p| matches!(p, Protocol::P2pCircuit)) {
            AddrSource::Relayed
        } else {
            AddrSource::Direct
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeerAddr {
    pub addr: Multiaddr,
    pub source: AddrSource,
}

pub struct UIState {
    pub local_peer_id: Option<PeerId>,
    pub connection_status: ConnectionStatus,
    pub peers: HashSet<PeerId>,
    pub sessions: HashSet<SessionId>,

    pub total_upload: u64,
    pub total_download: u64,

    /// Bandwidth samples as (timestamp, KB) — the overview chart buckets these
    /// into a seconds-ago Mbps line.
    pub upload_graph: Vec<(DateTime<Utc>, f64)>,
    pub download_graph: Vec<(DateTime<Utc>, f64)>,

    /// Most recent non-fatal error surfaced from the swarm. Cleared when a peer
    /// connects. Rendered as a banner on the Overview tab.
    pub last_error: Option<String>,

    /// Latest FindNodes pool per server, keyed by listen port. Replaced
    /// wholesale each discovery so filtered-out peers drop off.
    pub server_pools: HashMap<u16, Vec<PeerId>>,

    /// Remembered sticky standby pool per server (from `sticky_peers.json`),
    /// surfaced so the NETWORK tab shows every remembered exit, not just the
    /// active one. Replaced wholesale each discovery pass.
    pub sticky_pools: HashMap<u16, Vec<PeerId>>,

    /// Currently-selected destination peer per server, in lock-step with the
    /// swarm-side `ArcSwap` via `Events::ActiveDestination`.
    pub active_destinations: HashMap<u16, PeerId>,

    /// How each server's active destination was chosen (pinned rank / sticky /
    /// discovered), keyed by port.
    pub destination_sources: HashMap<u16, proxy_core::events::DestinationSource>,

    /// Per-rank resolvability of each server's pinned preference list,
    /// refreshed on every pinned connect pass.
    pub pinned_statuses: HashMap<u16, Vec<proxy_core::events::PinnedPeerStatus>>,

    /// Map a session back to its peer so `BandwidthEvents` (which carry only the
    /// session ID) can be attributed for per-peer throughput.
    pub session_peer: HashMap<SessionId, PeerId>,

    /// Per-peer total bytes (upload, download). Survives session boundaries.
    pub peer_bandwidth: HashMap<PeerId, (u64, u64)>,

    /// Best-known dial address per peer; `source` priority keeps a direct route
    /// from being clobbered by a later relay/candidate address.
    pub peer_addresses: HashMap<PeerId, PeerAddr>,
}

impl UIState {
    pub fn new() -> Self {
        UIState {
            local_peer_id: None,
            connection_status: ConnectionStatus::default(),
            peers: HashSet::new(),
            sessions: HashSet::new(),
            total_upload: 0,
            total_download: 0,
            upload_graph: Vec::with_capacity(1000),
            download_graph: Vec::with_capacity(1000),
            last_error: None,
            server_pools: HashMap::new(),
            sticky_pools: HashMap::new(),
            active_destinations: HashMap::new(),
            destination_sources: HashMap::new(),
            pinned_statuses: HashMap::new(),
            session_peer: HashMap::new(),
            peer_bandwidth: HashMap::new(),
            peer_addresses: HashMap::new(),
        }
    }

    /// The rotation-pool peers for `port` in render order: the FindNodes pool
    /// merged with remembered sticky exits, sorted directly-connected-first so a
    /// relay-only peer always sinks to the bottom. The order is independent of
    /// which peer is active — the `▶` marker moves between rows, the rows
    /// themselves don't reshuffle — so the cursor stays put when you switch
    /// exits. The NETWORK peer cursor indexes into exactly this Vec, so render
    /// and cursor never diverge.
    pub fn ordered_pool_for(&self, port: u16) -> Vec<PeerId> {
        let mut pool = self.server_pools.get(&port).cloned().unwrap_or_default();
        for peer in self.sticky_pools.get(&port).into_iter().flatten() {
            if !pool.contains(peer) {
                pool.push(*peer);
            }
        }
        pool.sort_by_key(|peer| self.pool_sort_key(*peer));
        pool
    }

    /// Sort key (smaller sorts first/top): directly-connected peers, then
    /// advertised candidates, with relay-only/unknown peers last — relay is
    /// never the steady state. Stable sort keeps MRU order within a tier.
    fn pool_sort_key(&self, peer: PeerId) -> u8 {
        match self.peer_addresses.get(&peer).map(|a| a.source) {
            Some(AddrSource::Direct) => 0,
            Some(AddrSource::Candidate) => 1,
            Some(AddrSource::Relayed) | None => 2,
        }
    }

    /// Record an address for `peer`, keeping only the most authoritative one
    /// seen — a lower-priority source never overwrites a higher one.
    pub fn note_peer_address(&mut self, peer: PeerId, addr: Multiaddr, source: AddrSource) {
        let keep_existing = self
            .peer_addresses
            .get(&peer)
            .is_some_and(|existing| existing.source > source);
        if keep_existing {
            return;
        }
        self.peer_addresses.insert(peer, PeerAddr { addr, source });
    }

    pub fn add_upload(&mut self, bytes: u64) {
        self.total_upload += bytes;
        let now = chrono::Utc::now();
        self.upload_graph.push((now, bytes as f64 / 1024.0));
        let cutoff = now - chrono::Duration::seconds(30);
        self.upload_graph.retain(|(time, _)| *time >= cutoff);
    }

    pub fn add_download(&mut self, bytes: u64) {
        self.total_download += bytes;
        let now = chrono::Utc::now();
        self.download_graph.push((now, bytes as f64 / 1024.0));
        let cutoff = now - chrono::Duration::seconds(30);
        self.download_graph.retain(|(time, _)| *time >= cutoff);
    }
}
