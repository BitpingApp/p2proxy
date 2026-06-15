use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use chrono::{DateTime, Utc};
use libp2p::{Multiaddr, PeerId, multiaddr::Protocol};
use p2p_bandwidth_protocol::TargetAddr;

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

pub struct ProxySession {
    pub id: SessionId,
    pub peer_id: PeerId,
    pub endpoint: TargetAddr,
}

#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub protocol: String,
    pub port: u16,
    pub active_sessions: usize,
    pub upload_rate: f64,   // KB/s
    pub download_rate: f64, // KB/s
}

#[derive(Debug, Default)]
pub struct PrometheusMetrics {
    pub total_sessions: usize,
    pub total_peers: usize,
    pub total_upload_bytes: u64,
    pub total_download_bytes: u64,
    pub upload_rate: f64,   // KB/s
    pub download_rate: f64, // KB/s
    pub servers: Vec<ServerInfo>,
}

impl PrometheusMetrics {
    pub fn parse_from_text(text: &str) -> Self {
        let mut metrics = PrometheusMetrics::default();

        for line in text.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }

            if let Some((metric_name, value_str)) = line.split_once(' ') {
                if let Ok(value) = value_str.parse::<f64>() {
                    match metric_name {
                        "p2proxy_sessions_initialized_total" => {
                            metrics.total_sessions = value as usize;
                        }
                        "p2proxy_peers_connected" => {
                            metrics.total_peers = value as usize;
                        }
                        "p2proxy_upload_bytes_total" => {
                            metrics.total_upload_bytes = value as u64;
                        }
                        "p2proxy_download_bytes_total" => {
                            metrics.total_download_bytes = value as u64;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Calculate rates (simplified - in real implementation you'd track over time)
        metrics.upload_rate = (metrics.total_upload_bytes as f64) / 1024.0 / 60.0; // Rough KB/s
        metrics.download_rate = (metrics.total_download_bytes as f64) / 1024.0 / 60.0; // Rough KB/s

        // Mock server data for now - in real implementation, parse from metrics
        metrics.servers = vec![ServerInfo {
            protocol: "SOCKS5".to_string(),
            port: 1080,
            active_sessions: metrics.total_sessions,
            upload_rate: metrics.upload_rate,
            download_rate: metrics.download_rate,
        }];

        metrics
    }
}

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
    /// circuit. Used for FindNodes candidates, which have no live
    /// connection to ask `endpoint.is_relayed()`; live routes are
    /// classified from the endpoint instead.
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
    pub sessions: HashMap<SessionId, ProxySession>,
    pub servers: Vec<ServerInfo>,

    pub total_upload: u64,
    pub total_download: u64,
    pub upload_rate: f64,   // Current KB/s
    pub download_rate: f64, // Current KB/s

    // Store data as (x, y) points where:
    // x is the time in seconds
    // y is the bandwidth in KB/s
    pub upload_graph: Vec<(DateTime<Utc>, f64)>,
    pub download_graph: Vec<(DateTime<Utc>, f64)>,

    /// Most recent non-fatal error surfaced from the swarm (e.g. "FindNodes
    /// returned 0 peers after 20 attempts"). Cleared when a peer connects
    /// successfully. Rendered as a banner on the Overview tab so the
    /// operator sees why p2proxy isn't routing traffic.
    pub last_error: Option<String>,

    /// Latest FindNodes pool per server, keyed by listen port. Updated
    /// every time `discover_peer` runs — the previous list is replaced
    /// wholesale because peers that fell out of the country/bandwidth
    /// filter shouldn't show up anymore.
    pub server_pools: HashMap<u16, Vec<PeerId>>,

    /// Currently-selected destination peer per server. Tracks the
    /// `ArcSwap<Option<PeerId>>` map on the swarm side; `Events::
    /// ActiveDestination` keeps the two in sync.
    pub active_destinations: HashMap<u16, PeerId>,

    /// How each server's active destination was chosen (pinned rank /
    /// sticky / discovered). Keyed by port, in lock-step with
    /// `active_destinations`.
    pub destination_sources: HashMap<u16, models::events::DestinationSource>,

    /// Per-rank resolvability of each server's pinned preference list,
    /// refreshed on every pinned connect pass (BIT-597). Stale entries
    /// render with a STALE badge in the NETWORK tab.
    pub pinned_statuses: HashMap<u16, Vec<models::events::PinnedPeerStatus>>,

    /// Map a session ID back to the peer it's routed through. Lets
    /// us attribute `BandwidthEvents` (which only carry the session
    /// ID) to a peer so we can show per-peer throughput.
    pub session_peer: HashMap<SessionId, PeerId>,

    /// Accumulator: per-peer total bytes (upload, download). Survives
    /// across session boundaries — see the note on `peers` about why
    /// we don't remove entries proactively.
    pub peer_bandwidth: HashMap<PeerId, (u64, u64)>,

    /// Best-known dial address per peer, fed by FindNodes candidates and
    /// live connection endpoints. The `source` priority means a direct
    /// hole-punched route is never clobbered by a later relay/candidate
    /// address. Rendered in the NETWORK tab's rotation pool.
    pub peer_addresses: HashMap<PeerId, PeerAddr>,
}

impl UIState {
    pub fn new() -> Self {
        UIState {
            local_peer_id: None,
            connection_status: ConnectionStatus::default(),
            peers: HashSet::new(),
            sessions: HashMap::new(),
            servers: Vec::new(),
            total_upload: 0,
            total_download: 0,
            upload_rate: 0.0,
            download_rate: 0.0,
            upload_graph: Vec::with_capacity(1000),
            download_graph: Vec::with_capacity(1000),
            last_error: None,
            server_pools: HashMap::new(),
            active_destinations: HashMap::new(),
            destination_sources: HashMap::new(),
            pinned_statuses: HashMap::new(),
            session_peer: HashMap::new(),
            peer_bandwidth: HashMap::new(),
            peer_addresses: HashMap::new(),
        }
    }

    /// Record an address for `peer`, keeping only the most authoritative
    /// one seen. A lower-priority source (e.g. a fresh FindNodes
    /// candidate) never overwrites a higher-priority one (e.g. the
    /// peer's live direct route), so the active peer's egress IP sticks.
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

        // Update upload rate (simplified - in real implementation you'd track over time)
        let now = chrono::Utc::now();
        self.upload_graph.push((now, bytes as f64 / 1024.0)); // Convert to KB

        // Keep only last 30 seconds of data
        let cutoff_time = now - chrono::Duration::seconds(30);
        self.upload_graph.retain(|(time, _)| *time >= cutoff_time);
    }

    pub fn add_download(&mut self, bytes: u64) {
        self.total_download += bytes;

        // Update download rate (simplified - in real implementation you'd track over time)
        let now = chrono::Utc::now();
        self.download_graph.push((now, bytes as f64 / 1024.0)); // Convert to KB

        // Keep only last 30 seconds of data
        let cutoff_time = now - chrono::Duration::seconds(30);
        self.download_graph.retain(|(time, _)| *time >= cutoff_time);
    }

    pub fn update_from_metrics(&mut self, metrics: PrometheusMetrics) {
        // Update totals
        self.total_upload = metrics.total_upload_bytes;
        self.total_download = metrics.total_download_bytes;
        self.upload_rate = metrics.upload_rate;
        self.download_rate = metrics.download_rate;

        // Update servers
        self.servers = metrics.servers;

        // Update connection status based on peer count
        if metrics.total_peers > 0 {
            // For now, use a dummy peer ID - in real implementation, get from metrics
            self.connection_status = ConnectionStatus::Connected(libp2p::PeerId::random());
        } else {
            self.connection_status = ConnectionStatus::Disconnected;
        }

        // Add bandwidth data points
        let now = chrono::Utc::now();

        if self.upload_rate > 0.0 {
            self.upload_graph.push((now, self.upload_rate));
        }
        if self.download_rate > 0.0 {
            self.download_graph.push((now, self.download_rate));
        }

        // Keep only last 30 seconds of data
        let cutoff_time = now - chrono::Duration::seconds(30);
        self.upload_graph.retain(|(time, _)| *time >= cutoff_time);
        self.download_graph.retain(|(time, _)| *time >= cutoff_time);
    }
}
