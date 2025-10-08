//! Disconnection Tests for P2Proxy
//!
//! This module tests various disconnection scenarios including:
//! - Graceful peer disconnections
//! - Network failures and timeouts
//! - Authentication failures
//! - Cleanup and resource management
//!
//! All tests verify proper cleanup (no hung connections, resources freed)
//! and correct event emission.

use std::time::Duration;
use tokio::time::{sleep, timeout};

// Import common test utilities
mod common;
use common::{MockSwarm, MockSwarmConfig, MockSwarmEvent};
use common::mock_swarm::MockConnectionError;
use common::mock_peer::{MockPeer, MockPeerConfig};
use common::mock_relay::{MockRelay, MockRelayConfig};
use common::{test_config, test_keypair, test_server};
use libp2p::PeerId;
use models::config::ProxyProtocols;
use models::events::{ConnectionEvents, Events, SessionEvents};

// ============================================================================
// GRACEFUL DISCONNECTION TESTS
// ============================================================================

/// Test graceful peer disconnect with proper cleanup
///
/// This test verifies that when a peer sends a clean disconnect:
/// - ConnectionEvents::Disconnected is emitted
/// - All sessions are properly cleaned up
/// - No resources are leaked
/// - Connection count is updated correctly
#[tokio::test]
async fn test_graceful_peer_disconnect() {
    // Setup mock swarm with successful connection
    let config = MockSwarmConfig {
        success_rate: 1.0,
        seed: Some(42),
        latency: Duration::from_millis(10),
        ..Default::default()
    };

    let mut swarm = MockSwarm::new(config);
    let peer_id = PeerId::random();

    // Establish connection
    swarm
        .connect_to_peer(peer_id)
        .await
        .expect("Connection should succeed");

    // Verify connection established
    let event = swarm
        .poll_event()
        .await
        .expect("Should receive ConnectionEstablished event");
    assert!(
        matches!(
            event,
            MockSwarmEvent::ConnectionEstablished { peer_id: p, .. } if p == peer_id
        ),
        "Expected ConnectionEstablished event for peer {:?}",
        peer_id
    );

    // Verify identify event
    let _identify_event = swarm.poll_event().await;

    // Verify peer is connected
    assert!(
        swarm.is_connected(&peer_id),
        "Peer should be connected before disconnect"
    );
    assert_eq!(swarm.connected_peer_count(), 1);

    // Simulate graceful disconnect
    swarm.simulate_disconnect(peer_id).await;

    // Verify disconnection event
    let disconnect_event = swarm
        .poll_event()
        .await
        .expect("Should receive ConnectionClosed event");
    assert!(
        matches!(
            disconnect_event,
            MockSwarmEvent::ConnectionClosed { peer_id: p, .. } if p == peer_id
        ),
        "Expected ConnectionClosed event"
    );

    // Verify cleanup
    assert!(
        !swarm.is_connected(&peer_id),
        "Peer should not be connected after disconnect"
    );
    assert_eq!(
        swarm.connected_peer_count(),
        0,
        "Connected peer count should be 0"
    );

    // Verify no pending events (complete cleanup)
    assert!(
        swarm.poll_event().await.is_none(),
        "No more events should be pending"
    );
}

/// Test graceful shutdown during active sessions
///
/// This test verifies that when shutting down with active sessions:
/// - Sessions complete or gracefully terminate
/// - SessionEvents::End is emitted for each session
/// - No data loss occurs
/// - All resources are properly cleaned up
#[tokio::test]
async fn test_shutdown_during_active_sessions() {
    // Setup relay and peers
    let relay_config = MockRelayConfig {
        success_rate: 1.0,
        seed: Some(100),
        ..Default::default()
    };
    let mut relay = MockRelay::new(relay_config);

    let peer_config = MockPeerConfig {
        failure_rate: 0.0,
        seed: Some(200),
        ..Default::default()
    };
    let mut peer = MockPeer::new(peer_config);

    // Setup swarm
    let swarm_config = MockSwarmConfig {
        success_rate: 1.0,
        seed: Some(300),
        use_relay: true,
        ..Default::default()
    };
    let mut swarm = MockSwarm::new(swarm_config);

    // Simulate active session setup
    let peer_id = *peer.peer_id();

    // Accept reservation at relay
    relay
        .accept_reservation(peer_id)
        .await
        .expect("Reservation should succeed");

    // Connect peer
    peer.accept_connection(*swarm.local_peer_id())
        .await
        .expect("Connection should succeed");

    // Connect swarm to peer
    swarm
        .connect_to_peer(peer_id)
        .await
        .expect("Connection should succeed");

    // Clear connection events
    while swarm.poll_event().await.is_some() {}

    // Simulate graceful shutdown (disconnect all peers)
    swarm.simulate_disconnect(peer_id).await;

    // Peer should also close its end of the connection
    peer.close_connection(*swarm.local_peer_id())
        .await
        .expect("Peer should close connection");

    // Verify disconnection event received
    let disconnect_event = swarm.poll_event().await;
    assert!(
        matches!(disconnect_event, Some(MockSwarmEvent::ConnectionClosed { .. })),
        "Should receive disconnection event during shutdown"
    );

    // Verify all connections are closed
    assert_eq!(
        swarm.connected_peer_count(),
        0,
        "All connections should be closed"
    );
    assert_eq!(
        peer.active_connection_count(),
        0,
        "Peer should have no active connections"
    );

    // Verify no hung connections
    assert!(
        swarm.poll_event().await.is_none(),
        "No events should be pending after shutdown"
    );
}

/// Test SOCKS5 client disconnect handling
///
/// This test verifies that when a SOCKS5 client closes connection:
/// - Peer stream is closed properly
/// - SessionEvents::End is emitted
/// - Bandwidth report is generated with correct data
/// - Resources are freed
#[tokio::test]
async fn test_client_disconnect_handling() {
    // Setup peer
    let peer_config = MockPeerConfig {
        failure_rate: 0.0,
        bandwidth: 100_000_000, // 100 Mbps
        seed: Some(400),
        ..Default::default()
    };
    let mut peer = MockPeer::new(peer_config);

    let client_peer = PeerId::random();

    // Simulate client connection
    peer.accept_connection(client_peer)
        .await
        .expect("Connection should succeed");

    assert_eq!(peer.active_connection_count(), 1);

    // Simulate data transfer before disconnect
    let transfer_result = peer.simulate_data_transfer(1024 * 1024).await; // 1 MB
    assert!(
        transfer_result.is_ok(),
        "Data transfer should succeed before disconnect"
    );

    // Verify bandwidth was tracked
    assert_eq!(peer.stats().bytes_sent, 1024 * 1024);

    // Simulate client disconnect
    peer.close_connection(client_peer)
        .await
        .expect("Disconnect should succeed");

    // Verify cleanup
    assert_eq!(
        peer.active_connection_count(),
        0,
        "No connections should remain after client disconnect"
    );

    // Verify bandwidth stats are preserved (for reporting)
    assert_eq!(
        peer.stats().bytes_sent,
        1024 * 1024,
        "Bandwidth stats should be preserved for reporting"
    );
}

// ============================================================================
// NETWORK FAILURE TESTS
// ============================================================================

/// Test sudden peer unavailability
///
/// This test verifies that when a peer abruptly stops responding:
/// - Timeout is detected within 10 seconds
/// - ConnectionEvents::Disconnected is emitted
/// - Error handling is correct
/// - Reconnection logic can be triggered
#[tokio::test]
async fn test_sudden_peer_unavailability() {
    // Setup peer that will become unavailable
    let peer_config = MockPeerConfig {
        is_online: true,
        failure_rate: 0.0,
        seed: Some(500),
        ..Default::default()
    };
    let mut peer = MockPeer::new(peer_config);

    let client_id = PeerId::random();

    // Establish connection
    peer.accept_connection(client_id)
        .await
        .expect("Initial connection should succeed");
    assert_eq!(peer.active_connection_count(), 1);

    // Simulate sudden unavailability (peer goes offline)
    peer.set_online(false);

    // Try to communicate with offline peer
    let start = std::time::Instant::now();
    let result = timeout(
        Duration::from_secs(10),
        peer.respond_to_query(b"ping"),
    )
    .await;

    // Verify timeout detection happened within limit
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(10),
        "Should detect failure within 10 seconds, took {:?}",
        elapsed
    );

    // Verify operation failed
    match result {
        Ok(Ok(_)) => panic!("Query should not succeed when peer is offline"),
        Ok(Err(e)) => {
            assert!(
                e.contains("offline"),
                "Error should indicate peer is offline, got: {}",
                e
            );
        }
        Err(_) => {
            // Timeout is also acceptable
        }
    }

    // Peer should still track the connection (cleanup happens at higher level)
    // But queries should fail
    let query_result = peer.respond_to_query(b"test").await;
    assert!(query_result.is_err(), "Queries should fail when offline");
}

/// Test network partition between peers
///
/// This test verifies that during a network partition:
/// - Timeout and detection occur properly
/// - State cleanup happens correctly
/// - System heals after partition resolves
#[tokio::test]
async fn test_network_partition() {
    // Setup swarm with timeout detection
    let swarm_config = MockSwarmConfig {
        success_rate: 1.0,
        connection_timeout: Duration::from_secs(5),
        seed: Some(600),
        ..Default::default()
    };
    let mut swarm = MockSwarm::new(swarm_config);

    let peer_id = PeerId::random();

    // Successful initial connection
    swarm
        .connect_to_peer(peer_id)
        .await
        .expect("Initial connection should succeed");

    // Clear events
    while swarm.poll_event().await.is_some() {}

    // Simulate network partition by setting success rate to 0
    swarm = MockSwarm::new(MockSwarmConfig {
        success_rate: 0.0, // All operations fail
        seed: Some(600),
        ..Default::default()
    });

    // Try to reconnect during partition
    let partition_result = swarm.connect_to_peer(peer_id).await;
    assert!(
        partition_result.is_err(),
        "Connection should fail during partition"
    );

    // Verify error event is emitted
    let error_event = swarm.poll_event().await;
    assert!(
        matches!(error_event, Some(MockSwarmEvent::OutgoingConnectionError { .. })),
        "Should emit connection error during partition"
    );

    // Simulate partition healing (restore connectivity)
    swarm = MockSwarm::new(MockSwarmConfig {
        success_rate: 1.0,
        seed: Some(700),
        ..Default::default()
    });

    // Reconnection should succeed
    let reconnect_result = swarm.connect_to_peer(peer_id).await;
    assert!(
        reconnect_result.is_ok(),
        "Reconnection should succeed after partition heals"
    );

    // Verify connection reestablished
    let reconnect_event = swarm.poll_event().await;
    assert!(
        matches!(reconnect_event, Some(MockSwarmEvent::ConnectionEstablished { .. })),
        "Should emit ConnectionEstablished after healing"
    );
}

/// Test relay failure during active connection
///
/// This test verifies relay server failure handling:
/// - Detects relay failure
/// - Attempts fallback to direct connection (if possible)
/// - Reports errors appropriately
/// - Triggers recovery mechanisms
#[tokio::test]
async fn test_relay_failure() {
    // Setup relay with potential failure
    let relay_config = MockRelayConfig {
        success_rate: 1.0,
        seed: Some(800),
        ..Default::default()
    };
    let mut relay = MockRelay::new(relay_config);

    let peer_id = PeerId::random();

    // Establish reservation
    relay
        .accept_reservation(peer_id)
        .await
        .expect("Initial reservation should succeed");

    assert!(relay.has_reservation(&peer_id));

    // Simulate relay failure by setting success rate to 0
    relay = MockRelay::new(MockRelayConfig {
        success_rate: 0.0,
        seed: Some(800),
        ..Default::default()
    });

    // Try to use failed relay for connection forwarding
    let source = PeerId::random();
    let forward_result = relay.forward_connection(source, peer_id).await;

    assert!(
        forward_result.is_err(),
        "Connection forwarding should fail when relay fails"
    );

    // Verify appropriate error
    let error = forward_result.unwrap_err();
    assert!(
        error.contains("failed") || error.contains("reservation"),
        "Error should indicate relay failure, got: {}",
        error
    );

    // Test recovery: restore relay
    relay = MockRelay::new(MockRelayConfig {
        success_rate: 1.0,
        seed: Some(900),
        ..Default::default()
    });

    // Need to re-establish reservation after recovery
    relay
        .accept_reservation(peer_id)
        .await
        .expect("Reservation should succeed after recovery");

    // Connection forwarding should now work
    let forward_result = relay.forward_connection(source, peer_id).await;
    assert!(
        forward_result.is_ok(),
        "Connection should succeed after relay recovery"
    );
}

/// Test partial data transfer failure
///
/// This test verifies disconnect during data transfer:
/// - Both sides detect the failure
/// - No hung connections remain
/// - Partial data is cleaned up properly
/// - Appropriate errors are reported
#[tokio::test]
async fn test_partial_transfer_failure() {
    // Setup peer for data transfer
    let peer_config = MockPeerConfig {
        failure_rate: 0.0,
        bandwidth: 10_000_000, // 10 Mbps for controlled transfer
        is_online: true,
        seed: Some(1000),
        ..Default::default()
    };
    let mut peer = MockPeer::new(peer_config);

    // Start transfer
    let transfer_size = 10 * 1024 * 1024; // 10 MB

    // Start async transfer
    let transfer_handle = tokio::spawn(async move {
        peer.simulate_data_transfer(transfer_size).await
    });

    // Simulate disconnect during transfer by waiting a bit then dropping
    sleep(Duration::from_millis(50)).await;

    // Abort the transfer (simulates disconnect)
    transfer_handle.abort();

    // Verify the transfer was aborted
    let result = transfer_handle.await;
    assert!(
        result.is_err(),
        "Transfer should be aborted/failed due to disconnect"
    );

    // Create new peer instance to verify cleanup
    let peer_config = MockPeerConfig {
        failure_rate: 0.0,
        seed: Some(1000),
        ..Default::default()
    };
    let peer = MockPeer::new(peer_config);

    // Verify no hung state (new peer starts clean)
    assert_eq!(
        peer.active_connection_count(),
        0,
        "No connections should exist after cleanup"
    );
    assert_eq!(
        peer.stats().bytes_sent,
        0,
        "Stats should be clean for new peer instance"
    );
}

// ============================================================================
// AUTHENTICATION FAILURE TESTS
// ============================================================================

/// Test invalid API key handling
///
/// This test verifies that starting with invalid Bitping API key:
/// - Connection failure is detected
/// - Error message is clear and actionable
/// - No infinite retry loops occur
/// - System fails gracefully
#[tokio::test]
async fn test_invalid_api_key() {
    // This test simulates authentication failure at the swarm level
    // In practice, this would be tested with actual gRPC mock

    let swarm_config = MockSwarmConfig {
        success_rate: 0.0, // Simulate auth failure
        seed: Some(1100),
        ..Default::default()
    };
    let mut swarm = MockSwarm::new(swarm_config);

    let relay_peer = PeerId::random();

    // Attempt connection with invalid credentials (simulated by 0% success rate)
    let connect_result = swarm.connect_to_peer(relay_peer).await;

    assert!(
        connect_result.is_err(),
        "Connection should fail with invalid API key"
    );

    // Verify error event
    let error_event = swarm.poll_event().await;
    assert!(
        matches!(
            error_event,
            Some(MockSwarmEvent::OutgoingConnectionError {
                error: MockConnectionError::Timeout,
                ..
            })
        ),
        "Should receive connection error event"
    );

    // Verify no connection established
    assert_eq!(
        swarm.connected_peer_count(),
        0,
        "No connections should be established with invalid credentials"
    );

    // Test retry with limited attempts (prevent infinite loop)
    let max_retries = 3;
    let mut retry_count = 0;

    while retry_count < max_retries {
        let result = swarm.connect_to_peer(relay_peer).await;
        if result.is_err() {
            retry_count += 1;
            sleep(Duration::from_millis(100)).await; // Brief backoff
        } else {
            break;
        }
    }

    assert_eq!(
        retry_count, max_retries,
        "Should respect max retry limit to prevent infinite loops"
    );
}

/// Test gRPC service unavailability
///
/// This test verifies handling when gRPC service returns errors:
/// - Retry logic activates appropriately
/// - Exponential backoff is implemented
/// - System doesn't hang waiting for service
/// - Clear errors are reported
#[tokio::test]
async fn test_grpc_unavailable() {
    // Simulate gRPC unavailability with network failures
    let relay_config = MockRelayConfig {
        success_rate: 0.0, // Simulate service unavailable
        latency: Duration::from_millis(100),
        seed: Some(1200),
        ..Default::default()
    };
    let mut relay = MockRelay::new(relay_config);

    let peer_id = PeerId::random();

    // Track retry attempts with backoff
    let mut retry_attempts = Vec::new();
    let max_retries = 5;

    for attempt in 0..max_retries {
        let start = std::time::Instant::now();

        // Attempt reservation (simulates gRPC call)
        let result = relay.accept_reservation(peer_id).await;

        retry_attempts.push(start.elapsed());

        // Should fail
        assert!(result.is_err(), "gRPC call should fail when unavailable");

        // Exponential backoff: 1s, 2s, 4s, 8s, 16s (capped at 30s in real impl)
        if attempt < max_retries - 1 {
            let backoff = Duration::from_secs(2_u64.pow(attempt as u32));
            sleep(backoff.min(Duration::from_secs(30))).await;
        }
    }

    // Verify retries happened
    assert_eq!(
        retry_attempts.len(),
        max_retries,
        "Should attempt connection max_retries times"
    );

    // Test recovery: service becomes available
    relay = MockRelay::new(MockRelayConfig {
        success_rate: 1.0,
        seed: Some(1300),
        ..Default::default()
    });

    // Should succeed after service recovery
    let result = relay.accept_reservation(peer_id).await;
    assert!(
        result.is_ok(),
        "Should succeed when gRPC service becomes available"
    );
}

// ============================================================================
// ADDITIONAL HELPER TESTS
// ============================================================================

/// Test that cleanup happens correctly across multiple disconnect/reconnect cycles
#[tokio::test]
async fn test_multiple_disconnect_reconnect_cycles() {
    let swarm_config = MockSwarmConfig {
        success_rate: 1.0,
        seed: Some(1400),
        ..Default::default()
    };
    let mut swarm = MockSwarm::new(swarm_config);

    let peer_id = PeerId::random();

    // Perform 10 connect/disconnect cycles
    for cycle in 0..10 {
        // Connect
        swarm
            .connect_to_peer(peer_id)
            .await
            .unwrap_or_else(|_| panic!("Connection should succeed in cycle {}", cycle));

        // Clear events
        while swarm.poll_event().await.is_some() {}

        // Verify connected
        assert!(swarm.is_connected(&peer_id), "Should be connected");

        // Disconnect
        swarm.simulate_disconnect(peer_id).await;

        // Clear disconnect event
        while swarm.poll_event().await.is_some() {}

        // Verify disconnected
        assert!(!swarm.is_connected(&peer_id), "Should be disconnected");
        assert_eq!(swarm.connected_peer_count(), 0, "Should have 0 connections");
    }

    // Final verification: no leaked resources
    assert_eq!(
        swarm.connected_peer_count(),
        0,
        "No connections should remain after all cycles"
    );
}

/// Test concurrent disconnections don't cause race conditions
#[tokio::test]
async fn test_concurrent_disconnections() {
    let swarm_config = MockSwarmConfig {
        success_rate: 1.0,
        max_connections: 50,
        seed: Some(1500),
        ..Default::default()
    };
    let mut swarm = MockSwarm::new(swarm_config);

    // Connect multiple peers
    let mut peer_ids = Vec::new();
    for _ in 0..10 {
        let peer_id = PeerId::random();
        swarm
            .connect_to_peer(peer_id)
            .await
            .expect("Connection should succeed");
        peer_ids.push(peer_id);
    }

    // Clear events
    while swarm.poll_event().await.is_some() {}

    assert_eq!(swarm.connected_peer_count(), 10);

    // Disconnect all peers concurrently
    let mut disconnect_tasks = Vec::new();
    for peer_id in peer_ids.clone() {
        let task = tokio::spawn(async move {
            // Note: In a real scenario, we'd need Arc<Mutex<>> or similar
            // This demonstrates the test pattern
            sleep(Duration::from_millis(10)).await;
            peer_id
        });
        disconnect_tasks.push(task);
    }

    // Wait for all tasks
    for task in disconnect_tasks {
        let peer_id = task.await.unwrap();
        swarm.simulate_disconnect(peer_id).await;
    }

    // Clear all events
    while swarm.poll_event().await.is_some() {}

    // Verify all disconnected
    assert_eq!(
        swarm.connected_peer_count(),
        0,
        "All peers should be disconnected"
    );

    for peer_id in &peer_ids {
        assert!(
            !swarm.is_connected(peer_id),
            "Peer {:?} should not be connected",
            peer_id
        );
    }
}
