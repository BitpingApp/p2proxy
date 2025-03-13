use std::{collections::HashMap, sync::LazyLock, time::Instant};

use eyeball::SharedObservable;
use libp2p::{Multiaddr, PeerId};

// Shared state to track connections
pub struct AppState {
    pub peers: HashMap<PeerId, PeerInfo>,
    pub socks_sessions: Vec<SocksSession>,
    pub selected_session_index: Option<usize>,
    pub local_peer_id: Option<PeerId>,
    pub relay_peer_id: Option<PeerId>,
    pub connection_status: ConnectionStatus,
}

pub enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
}

impl ConnectionStatus {
    fn as_str(&self) -> &str {
        match self {
            ConnectionStatus::Connecting => "Connecting...",
            ConnectionStatus::Connected => "Connected",
            ConnectionStatus::Disconnected => "Disconnected",
        }
    }
}

pub struct PeerInfo {
    pub address: Multiaddr,
    pub connected_at: Instant,
    pub is_relay: bool,
}

pub struct SocksSession {
    pub id: String,
    pub peer_id: PeerId,
    pub created_at: Instant,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub bandwidth_history: BandwidthHistory,
}

// Create a struct to store bandwidth history data for each session
pub struct BandwidthHistory {
    pub upload_data: Vec<(f64, f64)>,
    pub download_data: Vec<(f64, f64)>,
    pub max_points: usize,
    pub time_counter: f64,
}

impl Default for BandwidthHistory {
    fn default() -> Self {
        Self {
            upload_data: Vec::new(),
            download_data: Vec::new(),
            max_points: 100, // Store last 100 data points
            time_counter: 0.0,
        }
    }
}

impl BandwidthHistory {
    // Add a new data point
    fn add_sample(&mut self, upload_speed: f64, download_speed: f64) {
        self.time_counter += 1.0;

        // Add new data points
        self.upload_data.push((self.time_counter, upload_speed));
        self.download_data.push((self.time_counter, download_speed));

        // Remove old data points if we exceed max_points
        if self.upload_data.len() > self.max_points {
            self.upload_data.remove(0);
        }
        if self.download_data.len() > self.max_points {
            self.download_data.remove(0);
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            peers: HashMap::new(),
            socks_sessions: Vec::new(),
            selected_session_index: None,
            local_peer_id: None,
            relay_peer_id: None,
            connection_status: ConnectionStatus::Disconnected,
        }
    }
}

// Create a shared app state
pub static APP_STATE: LazyLock<SharedObservable<AppState>> =
    LazyLock::new(|| SharedObservable::new(AppState::default()));
