use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Instant,
};

use bitping_tcp_proxy::TargetAddr;
use chrono::{DateTime, Utc};
use libp2p::PeerId;

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
    // pub bandwidth_upload: [(Instant, u64); 50],
    // pub bandwidth_download: [(Instant, u64); 50],
}

pub struct UIState {
    pub local_peer_id: Option<PeerId>,
    pub connection_status: ConnectionStatus,
    pub peers: HashSet<PeerId>,
    pub sessions: HashMap<SessionId, ProxySession>,

    pub total_upload: u64,
    pub total_download: u64,

    // Store data as (x, y) points where:
    // x is the time in seconds
    // y is the bandwidth in KB/s
    pub upload_graph: Vec<(DateTime<Utc>, f64)>,
    pub download_graph: Vec<(DateTime<Utc>, f64)>,
    pub bandwidth_history_max_size: usize,

    // Track the start time for our graph
    pub graph_start_time: Instant,
}

impl UIState {
    pub fn new() -> Self {
        UIState {
            local_peer_id: None,
            connection_status: ConnectionStatus::default(),
            peers: HashSet::new(),
            sessions: HashMap::new(),
            total_upload: 0,
            total_download: 0,
            upload_graph: Vec::with_capacity(1000),
            download_graph: Vec::with_capacity(1000),
            bandwidth_history_max_size: 1000,
            graph_start_time: Instant::now(),
        }
    }

    pub fn add_upload(&mut self, upload: u64) {
        self.total_upload += upload;

        // Calculate time position
        let time_of_upload = chrono::Utc::now();

        // Convert upload to Mbps
        let upload_mbps = upload as f64 / 1000.0;

        // Add the point
        self.upload_graph.push((time_of_upload, upload_mbps));

        // Limit the number of points
        if self.upload_graph.len() > self.bandwidth_history_max_size {
            self.upload_graph.remove(0);
        }
    }

    pub fn add_download(&mut self, download: u64) {
        self.total_download += download;

        // Calculate time position
        let time_of_download = chrono::Utc::now();

        let download_mbps = download as f64 / 1000.0;

        // Add the point
        self.download_graph.push((time_of_download, download_mbps));

        // Limit the number of points
        if self.download_graph.len() > self.bandwidth_history_max_size {
            self.download_graph.remove(0);
        }
    }

    pub fn get_upload_stats(&self) -> Option<(DateTime<Utc>, DateTime<Utc>, f64, f64)> {
        if self.upload_graph.is_empty() {
            return None;
        }

        let mut min_timestamp = self.upload_graph[0].0;
        let mut max_timestamp = self.upload_graph[0].0;
        let mut min_upload = f64::MAX;
        let mut max_upload = f64::MIN;

        for &(timestamp, upload) in &self.upload_graph {
            if timestamp < min_timestamp {
                min_timestamp = timestamp;
            }
            if timestamp > max_timestamp {
                max_timestamp = timestamp;
            }
            min_upload = min_upload.min(upload);
            max_upload = max_upload.max(upload);
        }

        Some((min_timestamp, max_timestamp, min_upload, max_upload))
    }

    pub fn get_download_stats(&self) -> Option<(DateTime<Utc>, DateTime<Utc>, f64, f64)> {
        if self.download_graph.is_empty() {
            return None;
        }

        let mut min_timestamp = self.download_graph[0].0;
        let mut max_timestamp = self.download_graph[0].0;
        let mut min_download = f64::MAX;
        let mut max_download = f64::MIN;

        for &(timestamp, download) in &self.download_graph {
            if timestamp < min_timestamp {
                min_timestamp = timestamp;
            }
            if timestamp > max_timestamp {
                max_timestamp = timestamp;
            }
            min_download = min_download.min(download);
            max_download = max_download.max(download);
        }

        Some((min_timestamp, max_timestamp, min_download, max_download))
    }
}
