//! Mock Peer for Testing
//!
//! This module provides a mock implementation of a remote peer for testing
//! P2Proxy peer-to-peer interactions. It can simulate various peer behaviors,
//! network conditions, and failure scenarios.
//!
//! # Examples
//!
//! ```no_run
//! use p2proxy::tests::common::mock_peer::{MockPeer, MockPeerConfig};
//! use libp2p::PeerId;
//! use std::time::Duration;
//!
//! #[tokio::test]
//! async fn test_peer() {
//!     let config = MockPeerConfig {
//!         bandwidth: 100_000_000, // 100 Mbps
//!         latency: Duration::from_millis(50),
//!         failure_rate: 0.01, // 1% failure rate
//!         ..Default::default()
//!     };
//!
//!     let mut peer = MockPeer::new(config);
//!     let response = peer.respond_to_query(b"test query").await.unwrap();
//!     println!("Peer response: {:?}", response);
//! }
//! ```

use libp2p::{Multiaddr, PeerId};
use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Configuration for the mock peer
#[derive(Debug, Clone)]
pub struct MockPeerConfig {
    /// Peer's bandwidth capacity in bytes per second
    pub bandwidth: u64,

    /// Peer's network latency (one-way)
    pub latency: Duration,

    /// Failure rate for operations (0.0 to 1.0)
    pub failure_rate: f64,

    /// Random seed for deterministic behavior
    pub seed: Option<u64>,

    /// Maximum concurrent connections this peer can handle
    pub max_connections: usize,

    /// Whether the peer supports relay connections
    pub supports_relay: bool,

    /// Whether the peer supports DCUtR (hole punching)
    pub supports_dcutr: bool,

    /// Geographic region (for country-based filtering)
    pub country: Option<String>,

    /// Whether the peer is currently online
    pub is_online: bool,

    /// Response delay variability (jitter)
    pub jitter: Duration,
}

impl Default for MockPeerConfig {
    fn default() -> Self {
        Self {
            bandwidth: 100_000_000, // 100 Mbps
            latency: Duration::from_millis(50),
            failure_rate: 0.0,
            seed: None,
            max_connections: 50,
            supports_relay: true,
            supports_dcutr: true,
            country: None,
            is_online: true,
            jitter: Duration::from_millis(10),
        }
    }
}

/// Query types that a peer can respond to
#[derive(Debug, Clone, PartialEq)]
pub enum QueryType {
    /// Find specific node
    FindNode(PeerId),
    
    /// Find nodes matching criteria
    FindNodes {
        country: Option<String>,
        min_bandwidth: Option<u64>,
        limit: usize,
    },
    
    /// Ping request
    Ping,
    
    /// Custom query
    Custom(Vec<u8>),
}

/// Response from a peer
#[derive(Debug, Clone, PartialEq)]
pub enum QueryResponse {
    /// Peer information
    PeerInfo {
        peer_id: PeerId,
        addresses: Vec<Multiaddr>,
        bandwidth: u64,
        country: Option<String>,
    },
    
    /// List of peers
    Peers(Vec<PeerId>),
    
    /// Pong response
    Pong,
    
    /// Error response
    Error(String),
    
    /// Custom response
    Custom(Vec<u8>),
}

/// Statistics for a mock peer
#[derive(Debug, Clone, Default)]
pub struct PeerStats {
    /// Total bytes sent
    pub bytes_sent: u64,
    
    /// Total bytes received
    pub bytes_received: u64,
    
    /// Number of queries received
    pub queries_received: u64,
    
    /// Number of connections handled
    pub connections_handled: u64,
    
    /// Number of failures
    pub failures: u64,
}

/// Mock implementation of a remote peer
pub struct MockPeer {
    /// Peer ID
    peer_id: PeerId,
    
    /// Configuration
    config: MockPeerConfig,
    
    /// Current connections
    active_connections: HashMap<PeerId, Instant>,
    
    /// Random number generator
    rng: StdRng,
    
    /// Listening addresses
    addresses: Vec<Multiaddr>,
    
    /// Statistics
    stats: PeerStats,
    
    /// Bandwidth tracking
    bytes_transferred_this_second: u64,
    last_bandwidth_check: Instant,
}

impl MockPeer {
    /// Create a new MockPeer with the given configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let config = MockPeerConfig::default();
    /// let peer = MockPeer::new(config);
    /// ```
    pub fn new(config: MockPeerConfig) -> Self {
        let rng = if let Some(seed) = config.seed {
            StdRng::seed_from_u64(seed)
        } else {
            StdRng::from_seed(rand::random())
        };

        let peer_id = PeerId::random();
        let addresses = vec![
            format!("/ip4/192.168.1.100/tcp/4001/p2p/{}", peer_id).parse().unwrap(),
            format!("/ip4/192.168.1.100/udp/4001/quic-v1/p2p/{}", peer_id).parse().unwrap(),
        ];

        Self {
            peer_id,
            config,
            active_connections: HashMap::new(),
            rng,
            addresses,
            stats: PeerStats::default(),
            bytes_transferred_this_second: 0,
            last_bandwidth_check: Instant::now(),
        }
    }

    /// Create a new MockPeer with a specific peer ID
    pub fn with_peer_id(config: MockPeerConfig, peer_id: PeerId) -> Self {
        let mut peer = Self::new(config);
        peer.peer_id = peer_id;
        peer.addresses = vec![
            format!("/ip4/192.168.1.100/tcp/4001/p2p/{}", peer_id).parse().unwrap(),
            format!("/ip4/192.168.1.100/udp/4001/quic-v1/p2p/{}", peer_id).parse().unwrap(),
        ];
        peer
    }

    /// Get the peer ID
    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    /// Get the peer's addresses
    pub fn addresses(&self) -> &[Multiaddr] {
        &self.addresses
    }

    /// Get peer statistics
    pub fn stats(&self) -> &PeerStats {
        &self.stats
    }

    /// Set the peer's online/offline status
    pub fn set_online(&mut self, online: bool) {
        self.config.is_online = online;
    }

    /// Check if the peer is online
    pub fn is_online(&self) -> bool {
        self.config.is_online
    }

    /// Respond to a query
    ///
    /// This simulates the peer processing and responding to various types of queries.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let query = b"find nodes";
    /// let response = peer.respond_to_query(query).await.unwrap();
    /// ```
    pub async fn respond_to_query(&mut self, query: &[u8]) -> Result<QueryResponse, String> {
        // Check if peer is online
        if !self.config.is_online {
            return Err("Peer is offline".to_string());
        }

        // Simulate network latency with jitter
        let jitter = Duration::from_millis(self.rng.random_range(0..self.config.jitter.as_millis() as u64));
        sleep(self.config.latency + jitter).await;

        // Track query
        self.stats.queries_received += 1;
        self.stats.bytes_received += query.len() as u64;

        // Check if operation should fail
        if self.should_fail() {
            self.stats.failures += 1;
            return Err("Query failed".to_string());
        }

        // Simple query parsing (in real scenario, would use actual protocol)
        let response = if query.starts_with(b"ping") {
            QueryResponse::Pong
        } else if query.starts_with(b"find_nodes") {
            // Generate some mock peer IDs
            let peers: Vec<PeerId> = (0..5).map(|_| PeerId::random()).collect();
            QueryResponse::Peers(peers)
        } else if query.starts_with(b"peer_info") {
            QueryResponse::PeerInfo {
                peer_id: self.peer_id,
                addresses: self.addresses.clone(),
                bandwidth: self.config.bandwidth,
                country: self.config.country.clone(),
            }
        } else {
            QueryResponse::Custom(query.to_vec())
        };

        // Track response bytes
        let response_size = match &response {
            QueryResponse::PeerInfo { .. } => 200, // Approximate size
            QueryResponse::Peers(peers) => peers.len() * 50, // Approximate size per peer
            QueryResponse::Pong => 4,
            QueryResponse::Custom(data) => data.len(),
            QueryResponse::Error(msg) => msg.len(),
        };
        self.stats.bytes_sent += response_size as u64;

        Ok(response)
    }

    /// Simulate data transfer through this peer
    ///
    /// This simulates transferring data through the peer with bandwidth limits.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// peer.simulate_data_transfer(1024 * 1024).await.unwrap(); // 1 MB
    /// ```
    pub async fn simulate_data_transfer(&mut self, bytes: u64) -> Result<Duration, String> {
        // Check if peer is online
        if !self.config.is_online {
            return Err("Peer is offline".to_string());
        }

        // Check if operation should fail
        if self.should_fail() {
            self.stats.failures += 1;
            return Err("Data transfer failed".to_string());
        }

        let start = Instant::now();

        // Apply bandwidth limiting
        self.bytes_transferred_this_second += bytes;
        self.apply_bandwidth_limit().await;

        // Simulate latency
        sleep(self.config.latency).await;

        // Update stats
        self.stats.bytes_sent += bytes;

        let duration = start.elapsed();
        Ok(duration)
    }

    /// Accept a connection from another peer
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let source_peer = PeerId::random();
    /// peer.accept_connection(source_peer).await.unwrap();
    /// ```
    pub async fn accept_connection(&mut self, from_peer: PeerId) -> Result<(), String> {
        // Check if peer is online
        if !self.config.is_online {
            return Err("Peer is offline".to_string());
        }

        // Check connection limit
        if self.active_connections.len() >= self.config.max_connections {
            return Err("Maximum connections reached".to_string());
        }

        // Simulate connection latency
        sleep(self.config.latency).await;

        // Check if operation should fail
        if self.should_fail() {
            self.stats.failures += 1;
            return Err("Connection failed".to_string());
        }

        // Accept connection
        self.active_connections.insert(from_peer, Instant::now());
        self.stats.connections_handled += 1;

        Ok(())
    }

    /// Close a connection with another peer
    pub async fn close_connection(&mut self, peer: PeerId) -> Result<(), String> {
        self.active_connections.remove(&peer)
            .ok_or_else(|| "Connection not found".to_string())?;
        
        Ok(())
    }

    /// Get number of active connections
    pub fn active_connection_count(&self) -> usize {
        self.active_connections.len()
    }

    /// Get list of connected peers
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.active_connections.keys().cloned().collect()
    }

    /// Simulate a temporary network issue
    ///
    /// Takes the peer offline for the specified duration.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// peer.simulate_network_issue(Duration::from_secs(10)).await;
    /// ```
    pub async fn simulate_network_issue(&mut self, duration: Duration) {
        self.config.is_online = false;
        sleep(duration).await;
        self.config.is_online = true;
    }

    /// Adjust peer's bandwidth capacity
    pub fn set_bandwidth(&mut self, bandwidth: u64) {
        self.config.bandwidth = bandwidth;
    }

    /// Adjust peer's latency
    pub fn set_latency(&mut self, latency: Duration) {
        self.config.latency = latency;
    }

    /// Adjust peer's failure rate
    pub fn set_failure_rate(&mut self, rate: f64) {
        self.config.failure_rate = rate.clamp(0.0, 1.0);
    }

    // Internal helper methods

    /// Determine if an operation should fail based on configured failure rate
    fn should_fail(&mut self) -> bool {
        let random: f64 = self.rng.random();
        random < self.config.failure_rate
    }

    /// Apply bandwidth limiting by sleeping if necessary
    async fn apply_bandwidth_limit(&mut self) {
        let elapsed = self.last_bandwidth_check.elapsed();
        let max_bytes = (self.config.bandwidth as f64 * elapsed.as_secs_f64()) as u64;

        if self.bytes_transferred_this_second > max_bytes {
            // Calculate how long to sleep to stay within bandwidth limit
            let excess_bytes = self.bytes_transferred_this_second - max_bytes;
            let sleep_duration = Duration::from_secs_f64(excess_bytes as f64 / self.config.bandwidth as f64);
            sleep(sleep_duration).await;
        }

        // Reset counter periodically
        if elapsed >= Duration::from_secs(1) {
            self.bytes_transferred_this_second = 0;
            self.last_bandwidth_check = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_peer_creation() {
        let config = MockPeerConfig::default();
        let peer = MockPeer::new(config);
        assert!(peer.is_online());
        assert_eq!(peer.active_connection_count(), 0);
    }

    #[tokio::test]
    async fn test_respond_to_query() {
        let config = MockPeerConfig {
            failure_rate: 0.0,
            seed: Some(42),
            ..Default::default()
        };
        let mut peer = MockPeer::new(config);

        let response = peer.respond_to_query(b"ping").await.unwrap();
        assert_eq!(response, QueryResponse::Pong);
        assert_eq!(peer.stats().queries_received, 1);
    }

    #[tokio::test]
    async fn test_data_transfer() {
        let config = MockPeerConfig {
            bandwidth: 1_000_000, // 1 MB/s
            failure_rate: 0.0,
            seed: Some(123),
            ..Default::default()
        };
        let mut peer = MockPeer::new(config);

        let result = peer.simulate_data_transfer(1024).await;
        assert!(result.is_ok());
        assert_eq!(peer.stats().bytes_sent, 1024);
    }

    #[tokio::test]
    async fn test_connection_limit() {
        let config = MockPeerConfig {
            max_connections: 2,
            failure_rate: 0.0,
            seed: Some(456),
            ..Default::default()
        };
        let mut peer = MockPeer::new(config);

        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        let peer3 = PeerId::random();

        // Accept two connections (at limit)
        peer.accept_connection(peer1).await.unwrap();
        peer.accept_connection(peer2).await.unwrap();

        // Third should fail
        let result = peer.accept_connection(peer3).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_offline_peer() {
        let config = MockPeerConfig {
            is_online: false,
            ..Default::default()
        };
        let mut peer = MockPeer::new(config);

        let result = peer.respond_to_query(b"ping").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_network_issue_simulation() {
        let config = MockPeerConfig::default();
        let mut peer = MockPeer::new(config);

        assert!(peer.is_online());

        // Simulate brief network issue (reduced for test speed)
        tokio::spawn(async move {
            peer.simulate_network_issue(Duration::from_millis(100)).await;
        });

        // Give it a moment to go offline
        sleep(Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn test_failure_rate() {
        let config = MockPeerConfig {
            failure_rate: 1.0, // 100% failure rate
            seed: Some(789),
            ..Default::default()
        };
        let mut peer = MockPeer::new(config);

        let result = peer.respond_to_query(b"ping").await;
        assert!(result.is_err());
        assert!(peer.stats().failures > 0);
    }

    #[tokio::test]
    async fn test_bandwidth_configuration() {
        let config = MockPeerConfig {
            bandwidth: 100_000_000,
            ..Default::default()
        };
        let mut peer = MockPeer::new(config);

        // Change bandwidth
        peer.set_bandwidth(50_000_000);
        
        let response = peer.respond_to_query(b"peer_info").await.unwrap();
        if let QueryResponse::PeerInfo { bandwidth, .. } = response {
            assert_eq!(bandwidth, 50_000_000);
        } else {
            panic!("Expected PeerInfo response");
        }
    }
}
