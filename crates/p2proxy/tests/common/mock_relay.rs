//! Mock Relay Server for Testing
//!
//! This module provides a mock implementation of a libp2p relay server for testing
//! relay-mediated connections in P2Proxy. It can simulate relay reservations,
//! connection forwarding, and various relay-specific behaviors.
//!
//! # Examples
//!
//! ```no_run
//! use p2proxy::tests::common::mock_relay::{MockRelay, MockRelayConfig};
//! use libp2p::PeerId;
//!
//! #[tokio::test]
//! async fn test_relay() {
//!     let config = MockRelayConfig::default();
//!     let mut relay = MockRelay::new(config);
//!
//!     let peer_id = PeerId::random();
//!     relay.accept_reservation(peer_id).await.unwrap();
//!
//!     let address = relay.get_address();
//!     println!("Relay listening on: {}", address);
//! }
//! ```

use libp2p::{Multiaddr, PeerId};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Configuration for the mock relay server
#[derive(Debug, Clone)]
pub struct MockRelayConfig {
    /// Simulated relay latency (one-way)
    pub latency: Duration,

    /// Maximum number of reservations
    pub max_reservations: usize,

    /// Maximum number of concurrent connections per reservation
    pub max_connections_per_peer: usize,

    /// Success rate for relay operations
    pub success_rate: f64,

    /// Random seed for deterministic behavior
    pub seed: Option<u64>,

    /// Whether to simulate circuit limits
    pub enforce_limits: bool,

    /// Reservation duration
    pub reservation_duration: Duration,
}

impl Default for MockRelayConfig {
    fn default() -> Self {
        Self {
            latency: Duration::from_millis(20),
            max_reservations: 1000,
            max_connections_per_peer: 10,
            success_rate: 1.0,
            seed: None,
            enforce_limits: true,
            reservation_duration: Duration::from_secs(3600), // 1 hour
        }
    }
}

/// Information about a peer reservation
#[derive(Debug, Clone)]
pub struct ReservationInfo {
    pub peer_id: PeerId,
    pub created_at: Instant,
    pub expires_at: Instant,
    pub active_connections: usize,
}

/// Information about a forwarded connection
#[derive(Debug, Clone)]
pub struct ForwardedConnection {
    pub source_peer: PeerId,
    pub destination_peer: PeerId,
    pub established_at: Instant,
    pub bytes_forwarded: u64,
}

/// Events that the mock relay can generate
#[derive(Debug, Clone, PartialEq)]
pub enum MockRelayEvent {
    /// Reservation accepted
    ReservationAccepted {
        peer_id: PeerId,
        expires_at: Instant,
    },

    /// Reservation denied
    ReservationDenied { peer_id: PeerId, reason: String },

    /// Connection forwarded
    ConnectionForwarded {
        source_peer: PeerId,
        destination_peer: PeerId,
    },

    /// Connection forward failed
    ConnectionForwardFailed {
        source_peer: PeerId,
        destination_peer: PeerId,
        reason: String,
    },

    /// Reservation expired
    ReservationExpired { peer_id: PeerId },
}

/// Mock implementation of a libp2p relay server
pub struct MockRelay {
    /// Relay peer ID
    peer_id: PeerId,

    /// Configuration
    config: MockRelayConfig,

    /// Active reservations
    reservations: HashMap<PeerId, ReservationInfo>,

    /// Active forwarded connections
    connections: Vec<ForwardedConnection>,

    /// Random number generator
    rng: StdRng,

    /// Listening address
    listen_addr: Multiaddr,
}

impl MockRelay {
    /// Create a new MockRelay with the given configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let config = MockRelayConfig::default();
    /// let relay = MockRelay::new(config);
    /// ```
    pub fn new(config: MockRelayConfig) -> Self {
        let rng = if let Some(seed) = config.seed {
            StdRng::seed_from_u64(seed)
        } else {
            StdRng::from_seed(rand::random())
        };

        let peer_id = PeerId::random();
        let listen_addr = format!("/ip4/127.0.0.1/tcp/4001/p2p/{}", peer_id)
            .parse()
            .expect("Valid multiaddr");

        Self {
            peer_id,
            config,
            reservations: HashMap::new(),
            connections: Vec::new(),
            rng,
            listen_addr,
        }
    }

    /// Create a new MockRelay with a specific peer ID
    pub fn with_peer_id(config: MockRelayConfig, peer_id: PeerId) -> Self {
        let mut relay = Self::new(config);
        relay.peer_id = peer_id;
        relay.listen_addr = format!("/ip4/127.0.0.1/tcp/4001/p2p/{}", peer_id)
            .parse()
            .expect("Valid multiaddr");
        relay
    }

    /// Get the relay's peer ID
    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    /// Get the relay's listening address
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let address = relay.get_address();
    /// println!("Relay at: {}", address);
    /// ```
    pub fn get_address(&self) -> &Multiaddr {
        &self.listen_addr
    }

    /// Accept a reservation from a peer
    ///
    /// This simulates a peer reserving a circuit through the relay.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let peer_id = PeerId::random();
    /// relay.accept_reservation(peer_id).await.unwrap();
    /// ```
    pub async fn accept_reservation(&mut self, peer_id: PeerId) -> Result<ReservationInfo, String> {
        // Simulate network latency
        sleep(self.config.latency).await;

        // Check if operation should succeed
        if !self.should_succeed() {
            return Err("Relay operation failed".to_string());
        }

        // Check reservation limits
        if self.config.enforce_limits && self.reservations.len() >= self.config.max_reservations {
            return Err("Maximum reservations reached".to_string());
        }

        let now = Instant::now();
        let expires_at = now + self.config.reservation_duration;

        let info = ReservationInfo {
            peer_id,
            created_at: now,
            expires_at,
            active_connections: 0,
        };

        self.reservations.insert(peer_id, info.clone());

        Ok(info)
    }

    /// Forward a connection from source peer to destination peer
    ///
    /// This simulates relaying a connection between two peers.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let source = PeerId::random();
    /// let dest = PeerId::random();
    /// relay.forward_connection(source, dest).await.unwrap();
    /// ```
    pub async fn forward_connection(
        &mut self,
        source_peer: PeerId,
        destination_peer: PeerId,
    ) -> Result<(), String> {
        // Simulate network latency
        sleep(self.config.latency * 2).await;

        // Check if operation should succeed
        if !self.should_succeed() {
            return Err("Connection forward failed".to_string());
        }

        // Check if destination has a reservation
        let dest_reservation = self
            .reservations
            .get_mut(&destination_peer)
            .ok_or_else(|| "Destination peer has no reservation".to_string())?;

        // Check connection limits
        if self.config.enforce_limits
            && dest_reservation.active_connections >= self.config.max_connections_per_peer
        {
            return Err("Destination peer connection limit reached".to_string());
        }

        // Check if reservation is expired
        if dest_reservation.expires_at < Instant::now() {
            return Err("Destination peer reservation expired".to_string());
        }

        // Create forwarded connection
        let connection = ForwardedConnection {
            source_peer,
            destination_peer,
            established_at: Instant::now(),
            bytes_forwarded: 0,
        };

        dest_reservation.active_connections += 1;
        self.connections.push(connection);

        Ok(())
    }

    /// Simulate data transfer through a relayed connection
    ///
    /// # Examples
    ///
    /// ```no_run
    /// relay.transfer_data(source_peer, dest_peer, 1024).await;
    /// ```
    pub async fn transfer_data(
        &mut self,
        source_peer: PeerId,
        destination_peer: PeerId,
        bytes: u64,
    ) -> Result<(), String> {
        // Simulate network latency for data transfer
        sleep(self.config.latency).await;

        // Find the connection
        let connection = self
            .connections
            .iter_mut()
            .find(|c| c.source_peer == source_peer && c.destination_peer == destination_peer)
            .ok_or_else(|| "Connection not found".to_string())?;

        connection.bytes_forwarded += bytes;

        Ok(())
    }

    /// Close a forwarded connection
    pub async fn close_connection(
        &mut self,
        source_peer: PeerId,
        destination_peer: PeerId,
    ) -> Result<(), String> {
        // Find and remove the connection
        let pos = self
            .connections
            .iter()
            .position(|c| c.source_peer == source_peer && c.destination_peer == destination_peer)
            .ok_or_else(|| "Connection not found".to_string())?;

        self.connections.remove(pos);

        // Decrement active connections count
        if let Some(reservation) = self.reservations.get_mut(&destination_peer) {
            if reservation.active_connections > 0 {
                reservation.active_connections -= 1;
            }
        }

        Ok(())
    }

    /// Get list of peers with active reservations
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.reservations.keys().cloned().collect()
    }

    /// Check if a peer has an active reservation
    pub fn has_reservation(&self, peer_id: &PeerId) -> bool {
        if let Some(reservation) = self.reservations.get(peer_id) {
            reservation.expires_at > Instant::now()
        } else {
            false
        }
    }

    /// Get reservation info for a peer
    pub fn get_reservation(&self, peer_id: &PeerId) -> Option<&ReservationInfo> {
        self.reservations.get(peer_id)
    }

    /// Get all active connections
    pub fn active_connections(&self) -> &[ForwardedConnection] {
        &self.connections
    }

    /// Get number of active connections
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Remove expired reservations
    ///
    /// Call this periodically to clean up expired reservations.
    pub fn cleanup_expired_reservations(&mut self) -> Vec<PeerId> {
        let now = Instant::now();
        let mut expired = Vec::new();

        self.reservations.retain(|peer_id, info| {
            if info.expires_at < now {
                expired.push(*peer_id);
                false
            } else {
                true
            }
        });

        expired
    }

    // Internal helper methods

    /// Determine if an operation should succeed based on configured success rate
    fn should_succeed(&mut self) -> bool {
        let random: f64 = self.rng.random();
        random < self.config.success_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_relay_creation() {
        let config = MockRelayConfig::default();
        let relay = MockRelay::new(config);
        assert_eq!(relay.connected_peers().len(), 0);
        assert_eq!(relay.connection_count(), 0);
    }

    #[tokio::test]
    async fn test_accept_reservation() {
        let config = MockRelayConfig {
            success_rate: 1.0,
            seed: Some(42),
            ..Default::default()
        };
        let mut relay = MockRelay::new(config);
        let peer_id = PeerId::random();

        let result = relay.accept_reservation(peer_id).await;
        assert!(result.is_ok());
        assert!(relay.has_reservation(&peer_id));
    }

    #[tokio::test]
    async fn test_forward_connection() {
        let config = MockRelayConfig {
            success_rate: 1.0,
            seed: Some(123),
            ..Default::default()
        };
        let mut relay = MockRelay::new(config);

        let source = PeerId::random();
        let dest = PeerId::random();

        // Accept reservation for destination
        relay.accept_reservation(dest).await.unwrap();

        // Forward connection
        let result = relay.forward_connection(source, dest).await;
        assert!(result.is_ok());
        assert_eq!(relay.connection_count(), 1);
    }

    #[tokio::test]
    async fn test_reservation_limit() {
        let config = MockRelayConfig {
            max_reservations: 2,
            enforce_limits: true,
            success_rate: 1.0,
            seed: Some(456),
            ..Default::default()
        };
        let mut relay = MockRelay::new(config);

        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        let peer3 = PeerId::random();

        // Accept two reservations (at limit)
        relay.accept_reservation(peer1).await.unwrap();
        relay.accept_reservation(peer2).await.unwrap();

        // Third should fail
        let result = relay.accept_reservation(peer3).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_connection_per_peer_limit() {
        let config = MockRelayConfig {
            max_connections_per_peer: 2,
            enforce_limits: true,
            success_rate: 1.0,
            seed: Some(789),
            ..Default::default()
        };
        let mut relay = MockRelay::new(config);

        let dest = PeerId::random();
        let source1 = PeerId::random();
        let source2 = PeerId::random();
        let source3 = PeerId::random();

        // Accept reservation for destination
        relay.accept_reservation(dest).await.unwrap();

        // Forward two connections (at limit)
        relay.forward_connection(source1, dest).await.unwrap();
        relay.forward_connection(source2, dest).await.unwrap();

        // Third should fail
        let result = relay.forward_connection(source3, dest).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_data_transfer() {
        let config = MockRelayConfig {
            success_rate: 1.0,
            seed: Some(999),
            ..Default::default()
        };
        let mut relay = MockRelay::new(config);

        let source = PeerId::random();
        let dest = PeerId::random();

        // Setup connection
        relay.accept_reservation(dest).await.unwrap();
        relay.forward_connection(source, dest).await.unwrap();

        // Transfer data
        relay.transfer_data(source, dest, 1024).await.unwrap();

        let connections = relay.active_connections();
        assert_eq!(connections[0].bytes_forwarded, 1024);
    }

    #[tokio::test]
    async fn test_close_connection() {
        let config = MockRelayConfig::default();
        let mut relay = MockRelay::new(config);

        let source = PeerId::random();
        let dest = PeerId::random();

        // Setup connection
        relay.accept_reservation(dest).await.unwrap();
        relay.forward_connection(source, dest).await.unwrap();
        assert_eq!(relay.connection_count(), 1);

        // Close connection
        relay.close_connection(source, dest).await.unwrap();
        assert_eq!(relay.connection_count(), 0);

        // Active connections should be decremented
        let reservation = relay.get_reservation(&dest).unwrap();
        assert_eq!(reservation.active_connections, 0);
    }
}
