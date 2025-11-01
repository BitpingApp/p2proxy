//! Common test utilities and mock components for P2Proxy testing
//!
//! This module provides reusable test infrastructure including:
//! - Mock libp2p Swarm for simulating P2P network behavior
//! - Mock Relay server for testing relay-mediated connections
//! - Mock Peer for simulating remote peers
//!
//! # Examples
//!
//! ```no_run
//! use p2proxy::tests::common::mock_swarm::{MockSwarm, MockSwarmConfig};
//! use p2proxy::tests::common::mock_relay::{MockRelay, MockRelayConfig};
//! use p2proxy::tests::common::mock_peer::{MockPeer, MockPeerConfig};
//! use std::time::Duration;
//!
//! #[tokio::test]
//! async fn test_p2p_connection() {
//!     // Create a relay
//!     let relay_config = MockRelayConfig::default();
//!     let mut relay = MockRelay::new(relay_config);
//!
//!     // Create two peers
//!     let swarm_config = MockSwarmConfig {
//!         use_relay: true,
//!         ..Default::default()
//!     };
//!     let mut swarm1 = MockSwarm::new(swarm_config.clone());
//!     let mut swarm2 = MockSwarm::new(swarm_config);
//!
//!     // Connect peers through relay
//!     let peer2_id = *swarm2.local_peer_id();
//!     relay.accept_reservation(peer2_id).await.unwrap();
//!     swarm1.connect_to_peer(peer2_id).await.unwrap();
//! }
//! ```

pub mod mock_swarm;
pub mod mock_relay;
pub mod mock_peer;
pub mod fixtures;
pub mod test_utils;
pub mod platform;

// Re-export commonly used types
pub use mock_swarm::{MockSwarm, MockSwarmConfig, MockSwarmEvent};
pub use mock_relay::{MockRelay, MockRelayConfig};
pub use mock_peer::{MockPeer, MockPeerConfig};
pub use fixtures::*;
pub use test_utils::*;
pub use platform::*;
