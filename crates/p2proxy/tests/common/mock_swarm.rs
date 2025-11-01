//! Mock Swarm for Testing P2P Network Behavior
//!
//! This module provides a mock implementation of libp2p Swarm for testing P2Proxy
//! without requiring actual network connections. It allows you to simulate various
//! network conditions including latency, packet loss, and connection failures.
//!
//! # Examples
//!
//! ```no_run
//! use p2proxy::tests::common::mock_swarm::{MockSwarm, MockSwarmConfig};
//! use libp2p::PeerId;
//! use std::time::Duration;
//!
//! #[tokio::test]
//! async fn test_connection() {
//!     let config = MockSwarmConfig {
//!         latency: Duration::from_millis(50),
//!         packet_loss_rate: 0.01, // 1% packet loss
//!         bandwidth_limit: Some(100_000_000), // 100 Mbps
//!         success_rate: 0.99,
//!         seed: Some(42), // Deterministic behavior
//!         ..Default::default()
//!     };
//!
//!     let mut swarm = MockSwarm::new(config);
//!     let peer_id = PeerId::random();
//!
//!     // Simulate connection
//!     swarm.connect_to_peer(peer_id).await.unwrap();
//!
//!     // Poll for events
//!     if let Some(event) = swarm.poll_event().await {
//!         println!("Event: {:?}", event);
//!     }
//! }
//! ```

use libp2p::{Multiaddr, PeerId};
use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Configuration for the mock swarm behavior
#[derive(Debug, Clone)]
pub struct MockSwarmConfig {
    /// Simulated network latency (one-way)
    pub latency: Duration,

    /// Packet loss rate (0.0 to 1.0)
    pub packet_loss_rate: f64,

    /// Bandwidth limit in bytes per second (None = unlimited)
    pub bandwidth_limit: Option<u64>,

    /// Success rate for operations (0.0 to 1.0)
    /// Used to simulate intermittent failures
    pub success_rate: f64,

    /// Random seed for deterministic behavior (None = non-deterministic)
    pub seed: Option<u64>,

    /// Maximum number of concurrent connections
    pub max_connections: usize,

    /// Connection timeout duration
    pub connection_timeout: Duration,

    /// Whether to simulate relay connections
    pub use_relay: bool,
}

impl Default for MockSwarmConfig {
    fn default() -> Self {
        Self {
            latency: Duration::from_millis(10),
            packet_loss_rate: 0.0,
            bandwidth_limit: None,
            success_rate: 1.0,
            seed: None,
            max_connections: 100,
            connection_timeout: Duration::from_secs(30),
            use_relay: false,
        }
    }
}

/// Event types that the mock swarm can generate
#[derive(Debug, Clone, PartialEq)]
pub enum MockSwarmEvent {
    /// Connection established with a peer
    ConnectionEstablished {
        peer_id: PeerId,
        endpoint: MockEndpoint,
        num_established: u32,
    },

    /// Connection closed with a peer
    ConnectionClosed {
        peer_id: PeerId,
        num_established: u32,
    },

    /// Outgoing connection error
    OutgoingConnectionError {
        peer_id: Option<PeerId>,
        error: MockConnectionError,
    },

    /// New listening address
    NewListenAddr {
        address: Multiaddr,
    },

    /// Identify event received
    IdentifyReceived {
        peer_id: PeerId,
        info: MockIdentifyInfo,
    },

    /// Relay reservation accepted
    RelayReservationAccepted {
        relay_peer_id: PeerId,
    },

    /// DCUtR (Direct Connection Upgrade through Relay) event
    DcutrEvent {
        peer_id: PeerId,
        result: Result<(), String>,
    },

    /// Custom event for testing
    Custom(String),
}

/// Mock endpoint type (dialer or listener)
#[derive(Debug, Clone, PartialEq)]
pub enum MockEndpoint {
    Dialer { address: Multiaddr },
    Listener { local_addr: Multiaddr },
}

/// Mock connection errors
#[derive(Debug, Clone, PartialEq)]
pub enum MockConnectionError {
    Timeout,
    ConnectionRefused,
    NoAddresses,
    Transport(String),
}

/// Mock identify protocol information
#[derive(Debug, Clone, PartialEq)]
pub struct MockIdentifyInfo {
    pub public_key: Vec<u8>,
    pub protocol_version: String,
    pub agent_version: String,
    pub listen_addrs: Vec<Multiaddr>,
    pub protocols: Vec<String>,
}

impl Default for MockIdentifyInfo {
    fn default() -> Self {
        Self {
            public_key: vec![],
            protocol_version: "/ipfs/0.1.0".to_string(),
            agent_version: "bitping-federated/1.0.0".to_string(),
            listen_addrs: vec![],
            protocols: vec![],
        }
    }
}

/// State of a connection to a peer
#[derive(Debug, Clone)]
struct PeerConnection {
    peer_id: PeerId,
    address: Multiaddr,
    connected_at: Instant,
    endpoint: MockEndpoint,
}

/// Mock implementation of a libp2p Swarm
pub struct MockSwarm {
    /// Local peer ID
    local_peer_id: PeerId,

    /// Configuration for mock behavior
    config: MockSwarmConfig,

    /// Currently connected peers
    connected_peers: HashMap<PeerId, PeerConnection>,

    /// Queue of events to be polled
    event_queue: VecDeque<MockSwarmEvent>,

    /// Random number generator (for deterministic testing)
    rng: StdRng,

    /// Addresses the swarm is listening on
    listen_addresses: Vec<Multiaddr>,

    /// External addresses
    external_addresses: Vec<Multiaddr>,

    /// Total bytes transferred (for bandwidth simulation)
    bytes_transferred: u64,
    last_bandwidth_check: Instant,
}

impl MockSwarm {
    /// Create a new MockSwarm with the given configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let config = MockSwarmConfig::default();
    /// let swarm = MockSwarm::new(config);
    /// ```
    pub fn new(config: MockSwarmConfig) -> Self {
        let rng = if let Some(seed) = config.seed {
            StdRng::seed_from_u64(seed)
        } else {
            StdRng::from_seed(rand::random())
        };

        Self {
            local_peer_id: PeerId::random(),
            config,
            connected_peers: HashMap::new(),
            event_queue: VecDeque::new(),
            rng,
            listen_addresses: Vec::new(),
            external_addresses: Vec::new(),
            bytes_transferred: 0,
            last_bandwidth_check: Instant::now(),
        }
    }

    /// Create a new MockSwarm with a specific peer ID
    pub fn with_peer_id(config: MockSwarmConfig, peer_id: PeerId) -> Self {
        let mut swarm = Self::new(config);
        swarm.local_peer_id = peer_id;
        swarm
    }

    /// Get the local peer ID
    pub fn local_peer_id(&self) -> &PeerId {
        &self.local_peer_id
    }

    /// Start listening on an address
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use libp2p::multiaddr::multiaddr;
    ///
    /// swarm.listen_on(multiaddr!(Ip4([127, 0, 0, 1]), Tcp(0u16))).await.unwrap();
    /// ```
    pub async fn listen_on(&mut self, addr: Multiaddr) -> Result<(), String> {
        // Simulate latency
        sleep(self.config.latency).await;

        // Check if operation should succeed
        if !self.should_succeed() {
            return Err("Failed to listen on address".to_string());
        }

        self.listen_addresses.push(addr.clone());
        self.event_queue.push_back(MockSwarmEvent::NewListenAddr { address: addr });

        Ok(())
    }

    /// Add an external address
    pub fn add_external_address(&mut self, addr: Multiaddr) {
        self.external_addresses.push(addr);
    }

    /// Connect to a peer at the given address
    ///
    /// This simulates dialing a peer and establishing a connection.
    /// The connection may fail based on the configured success rate.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use libp2p::PeerId;
    /// use libp2p::multiaddr::multiaddr;
    ///
    /// let peer_id = PeerId::random();
    /// let addr = multiaddr!(Ip4([127, 0, 0, 1]), Tcp(4001u16));
    /// swarm.connect_to_peer_with_addr(peer_id, addr).await.unwrap();
    /// ```
    pub async fn connect_to_peer_with_addr(
        &mut self,
        peer_id: PeerId,
        addr: Multiaddr,
    ) -> Result<(), MockConnectionError> {
        // Check connection limit
        if self.connected_peers.len() >= self.config.max_connections {
            return Err(MockConnectionError::ConnectionRefused);
        }

        // Simulate connection establishment latency (reduced from 2x to 1x for faster tests)
        sleep(self.config.latency).await;

        // Check if connection should succeed
        if !self.should_succeed() {
            let error = MockConnectionError::Timeout;
            self.event_queue.push_back(MockSwarmEvent::OutgoingConnectionError {
                peer_id: Some(peer_id),
                error: error.clone(),
            });
            return Err(error);
        }

        // Simulate packet loss
        if self.should_drop_packet() {
            let error = MockConnectionError::Timeout;
            self.event_queue.push_back(MockSwarmEvent::OutgoingConnectionError {
                peer_id: Some(peer_id),
                error: error.clone(),
            });
            return Err(error);
        }

        // Create connection
        let num_established = self.connected_peers.len() as u32 + 1;
        let endpoint = MockEndpoint::Dialer {
            address: addr.clone(),
        };

        self.connected_peers.insert(
            peer_id,
            PeerConnection {
                peer_id,
                address: addr,
                connected_at: Instant::now(),
                endpoint: endpoint.clone(),
            },
        );

        // Queue connection established event
        self.event_queue.push_back(MockSwarmEvent::ConnectionEstablished {
            peer_id,
            endpoint,
            num_established,
        });

        // Queue identify event (simulating libp2p identify protocol)
        let mut info = MockIdentifyInfo::default();
        info.listen_addrs = self.listen_addresses.clone();
        self.event_queue.push_back(MockSwarmEvent::IdentifyReceived {
            peer_id,
            info,
        });

        // If using relay, queue relay reservation event
        if self.config.use_relay {
            self.event_queue.push_back(MockSwarmEvent::RelayReservationAccepted {
                relay_peer_id: peer_id,
            });
        }

        Ok(())
    }

    /// Connect to a peer (simplified version without explicit address)
    pub async fn connect_to_peer(&mut self, peer_id: PeerId) -> Result<(), MockConnectionError> {
        // Generate a mock address
        let addr = format!("/ip4/127.0.0.1/tcp/4001/p2p/{}", peer_id)
            .parse()
            .map_err(|_| MockConnectionError::NoAddresses)?;

        self.connect_to_peer_with_addr(peer_id, addr).await
    }

    /// Simulate a peer disconnecting
    ///
    /// # Examples
    ///
    /// ```no_run
    /// swarm.simulate_disconnect(peer_id).await;
    /// ```
    pub async fn simulate_disconnect(&mut self, peer_id: PeerId) {
        if let Some(_) = self.connected_peers.remove(&peer_id) {
            // Simulate latency for disconnect event
            sleep(self.config.latency).await;

            let num_established = self.connected_peers.len() as u32;
            self.event_queue.push_back(MockSwarmEvent::ConnectionClosed {
                peer_id,
                num_established,
            });
        }
    }

    /// Inject a custom event into the event queue
    ///
    /// Useful for testing specific event handling scenarios.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// swarm.inject_event(MockSwarmEvent::Custom("test_event".to_string()));
    /// ```
    pub fn inject_event(&mut self, event: MockSwarmEvent) {
        self.event_queue.push_back(event);
    }

    /// Poll for the next event
    ///
    /// Returns `None` if there are no pending events.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// while let Some(event) = swarm.poll_event().await {
    ///     match event {
    ///         MockSwarmEvent::ConnectionEstablished { peer_id, .. } => {
    ///             println!("Connected to {}", peer_id);
    ///         }
    ///         _ => {}
    ///     }
    /// }
    /// ```
    pub async fn poll_event(&mut self) -> Option<MockSwarmEvent> {
        // Apply bandwidth limits if configured
        self.apply_bandwidth_limit().await;

        self.event_queue.pop_front()
    }

    /// Check if connected to a specific peer
    pub fn is_connected(&self, peer_id: &PeerId) -> bool {
        self.connected_peers.contains_key(peer_id)
    }

    /// Get the number of connected peers
    pub fn connected_peer_count(&self) -> usize {
        self.connected_peers.len()
    }

    /// Get list of connected peer IDs
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.connected_peers.keys().cloned().collect()
    }

    /// Get listening addresses
    pub fn listeners(&self) -> &[Multiaddr] {
        &self.listen_addresses
    }

    /// Get external addresses
    pub fn external_addresses(&self) -> &[Multiaddr] {
        &self.external_addresses
    }

    /// Simulate data transfer for bandwidth limiting
    ///
    /// Call this when simulating data transfer to apply bandwidth limits.
    pub async fn simulate_transfer(&mut self, bytes: u64) {
        self.bytes_transferred += bytes;
        self.apply_bandwidth_limit().await;
    }

    // Internal helper methods

    /// Determine if an operation should succeed based on configured success rate
    fn should_succeed(&mut self) -> bool {
        let random: f64 = self.rng.random();
        random < self.config.success_rate
    }

    /// Determine if a packet should be dropped based on configured packet loss rate
    fn should_drop_packet(&mut self) -> bool {
        let random: f64 = self.rng.random();
        random < self.config.packet_loss_rate
    }

    /// Apply bandwidth limiting by sleeping if necessary
    async fn apply_bandwidth_limit(&mut self) {
        if let Some(limit) = self.config.bandwidth_limit {
            let elapsed = self.last_bandwidth_check.elapsed();
            let max_bytes = (limit as f64 * elapsed.as_secs_f64()) as u64;

            if self.bytes_transferred > max_bytes {
                // Calculate how long to sleep to stay within bandwidth limit
                let excess_bytes = self.bytes_transferred - max_bytes;
                let sleep_duration = Duration::from_secs_f64(excess_bytes as f64 / limit as f64);
                sleep(sleep_duration).await;
            }

            // Reset counter periodically
            if elapsed >= Duration::from_secs(1) {
                self.bytes_transferred = 0;
                self.last_bandwidth_check = Instant::now();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_swarm_creation() {
        let config = MockSwarmConfig::default();
        let swarm = MockSwarm::new(config);
        assert_eq!(swarm.connected_peer_count(), 0);
    }

    #[tokio::test]
    async fn test_deterministic_behavior() {
        let config1 = MockSwarmConfig {
            seed: Some(42),
            success_rate: 0.5,
            ..Default::default()
        };
        let config2 = MockSwarmConfig {
            seed: Some(42),
            success_rate: 0.5,
            ..Default::default()
        };

        let mut swarm1 = MockSwarm::new(config1);
        let mut swarm2 = MockSwarm::new(config2);

        let peer1 = PeerId::random();
        let peer2 = PeerId::random();

        let result1 = swarm1.connect_to_peer(peer1).await;
        let result2 = swarm2.connect_to_peer(peer2).await;

        // Both should have the same success/failure with same seed
        assert_eq!(result1.is_ok(), result2.is_ok());
    }

    #[tokio::test]
    async fn test_connection_and_disconnection() {
        let config = MockSwarmConfig {
            success_rate: 1.0,
            seed: Some(123),
            ..Default::default()
        };
        let mut swarm = MockSwarm::new(config);
        let peer_id = PeerId::random();

        // Connect
        swarm.connect_to_peer(peer_id).await.unwrap();
        assert!(swarm.is_connected(&peer_id));
        assert_eq!(swarm.connected_peer_count(), 1);

        // Should have events queued (ConnectionEstablished and IdentifyReceived)
        let event1 = swarm.poll_event().await;
        assert!(matches!(event1, Some(MockSwarmEvent::ConnectionEstablished { .. })));

        // Poll identify event
        let _event2 = swarm.poll_event().await;

        // Disconnect
        swarm.simulate_disconnect(peer_id).await;
        assert!(!swarm.is_connected(&peer_id));
        assert_eq!(swarm.connected_peer_count(), 0);

        // Should have disconnect event
        let disconnect_event = swarm.poll_event().await;
        assert!(matches!(disconnect_event, Some(MockSwarmEvent::ConnectionClosed { .. })));
    }

    #[tokio::test]
    async fn test_packet_loss() {
        let config = MockSwarmConfig {
            packet_loss_rate: 1.0, // 100% packet loss
            seed: Some(456),
            ..Default::default()
        };
        let mut swarm = MockSwarm::new(config);
        let peer_id = PeerId::random();

        // Should fail due to packet loss
        let result = swarm.connect_to_peer(peer_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_connection_limit() {
        let config = MockSwarmConfig {
            max_connections: 2,
            success_rate: 1.0,
            seed: Some(789),
            ..Default::default()
        };
        let mut swarm = MockSwarm::new(config);

        // Connect to max peers
        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        swarm.connect_to_peer(peer1).await.unwrap();
        swarm.connect_to_peer(peer2).await.unwrap();

        // Third connection should fail
        let peer3 = PeerId::random();
        let result = swarm.connect_to_peer(peer3).await;
        assert!(matches!(result, Err(MockConnectionError::ConnectionRefused)));
    }
}
