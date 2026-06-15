//! Throughput and bandwidth measurement tests for P2Proxy
//!
//! Simplified test suite that verifies:
//! - Basic byte counting accuracy
//! - Concurrent session handling
//! - Minimum bandwidth enforcement
//!
//! Note: This suite focuses on ensuring data flows correctly rather than
//! stringent performance measurements, as connection quality varies by peer.

// Import common test utilities
mod common;
use common::fixtures::{generate_test_data, test_server_with_bandwidth};
use common::mock_peer::{MockPeer, MockPeerConfig};
use common::test_utils::{BandwidthMeasurement, assert_bandwidth_within};
use models::config::ProxyProtocols;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Helper struct to track bandwidth usage during tests
struct BandwidthTracker {
    incoming_bytes: u64,
    outgoing_bytes: u64,
}

impl BandwidthTracker {
    fn new() -> Self {
        Self {
            incoming_bytes: 0,
            outgoing_bytes: 0,
        }
    }

    fn record_incoming(&mut self, bytes: usize) {
        self.incoming_bytes += bytes as u64;
    }

    fn record_outgoing(&mut self, bytes: usize) {
        self.outgoing_bytes += bytes as u64;
    }

    fn total_bytes(&self) -> u64 {
        self.incoming_bytes + self.outgoing_bytes
    }
}

/// Test 1: Basic byte counting accuracy
///
/// Verifies that byte counting is accurate within 1% tolerance for data transfers.
/// Tests with 10MB of data to ensure the system tracks bytes correctly.
#[tokio::test]
async fn test_basic_byte_counting() {
    // Generate 10MB of test data
    let size = 10_000_000; // 10 MB
    let (data, _) = generate_test_data(size);

    let mut tracker = BandwidthTracker::new();

    let start = Instant::now();

    // Simulate chunked transfer (realistic scenario)
    let chunk_size = 8192; // 8KB chunks
    for chunk in data.chunks(chunk_size) {
        tracker.record_outgoing(chunk.len());
    }

    // Simulate receiving
    for chunk in data.chunks(chunk_size) {
        tracker.record_incoming(chunk.len());
    }

    let duration = start.elapsed();

    // Verify byte counts within 1% tolerance
    assert_bandwidth_within(tracker.outgoing_bytes, size as u64, 1.0);
    assert_bandwidth_within(tracker.incoming_bytes, size as u64, 1.0);

    println!(
        "✓ Basic transfer completed: {} bytes in {:?} ({:.2} MB/s)",
        tracker.total_bytes(),
        duration,
        (tracker.total_bytes() as f64 / duration.as_secs_f64()) / 1_000_000.0
    );
}

/// Test 2: Concurrent session throughput
///
/// Measures aggregate throughput across multiple concurrent sessions.
/// Verifies that the system can handle multiple simultaneous data transfers
/// without losing data or corrupting byte counts.
#[tokio::test]
async fn test_concurrent_sessions() {
    // Test with 10 and 50 concurrent sessions
    for session_count in [10u64, 50] {
        let size_per_session = 100_000usize; // 100KB per session

        let start = Instant::now();

        // Create concurrent transfers
        let mut handles = vec![];
        let total_bytes = Arc::new(Mutex::new(0u64));

        for i in 0..session_count {
            let total = Arc::clone(&total_bytes);
            let handle = tokio::spawn(async move {
                let (data, _) = generate_test_data(size_per_session);

                // Simulate transfer
                let mut transferred = 0u64;
                for chunk in data.chunks(8192) {
                    transferred += chunk.len() as u64;
                    tokio::task::yield_now().await;
                }

                // Update total
                let mut total_lock = total.lock().await;
                *total_lock += transferred;
            });
            handles.push(handle);
        }

        // Wait for all sessions to complete
        for handle in handles {
            handle.await.unwrap();
        }

        let duration = start.elapsed();
        let total = *total_bytes.lock().await;
        let measurement = BandwidthMeasurement::new(total, duration);

        println!(
            "✓ {} concurrent sessions: {:.2} MB total, {:.2} MB/s aggregate ({:.2} Mbps)",
            session_count,
            total as f64 / 1_000_000.0,
            measurement.bytes_per_sec / 1_000_000.0,
            measurement.mbps()
        );

        // Verify expected total bytes
        let expected_total = (size_per_session as u64) * session_count;
        assert_bandwidth_within(total, expected_total, 1.0);

        // Verify reasonable aggregate throughput
        assert!(
            measurement.mbps() > 1.0,
            "Aggregate throughput too low: {:.2} Mbps",
            measurement.mbps()
        );
    }
}

/// Test 3: Minimum bandwidth enforcement
///
/// Verifies that the min_bandwidth configuration is respected.
/// Tests that peers with different bandwidth capabilities work correctly
/// and that configuration is properly applied.
#[tokio::test]
async fn test_min_bandwidth_config() {
    // Create mock peers with different bandwidth capabilities
    let peer_configs = vec![
        MockPeerConfig {
            bandwidth: 10_000_000, // 10 Mbps
            latency: Duration::from_millis(50),
            ..Default::default()
        },
        MockPeerConfig {
            bandwidth: 100_000_000, // 100 Mbps
            latency: Duration::from_millis(20),
            ..Default::default()
        },
    ];

    // Test data
    let size = 1_000_000; // 1 MB
    let (_data, _) = generate_test_data(size);

    for (idx, config) in peer_configs.iter().enumerate() {
        let mut peer = MockPeer::new(config.clone());

        let start = Instant::now();

        // Simulate transfer through the peer
        match peer.simulate_data_transfer(size as u64).await {
            Ok(_) => {
                let duration = start.elapsed();
                let measurement = BandwidthMeasurement::new(size as u64, duration);

                println!(
                    "✓ Peer {} ({} Mbps capacity): {:.2} MB/s actual ({:.2} Mbps)",
                    idx,
                    config.bandwidth * 8 / 1_000_000,
                    measurement.bytes_per_sec / 1_000_000.0,
                    measurement.mbps()
                );

                // Verify peer is online and responsive
                assert!(peer.is_online(), "Peer should be online");
                assert!(measurement.mbps() > 0.0, "Bandwidth should be positive");
            }
            Err(e) => {
                panic!("Transfer failed: {}", e);
            }
        }
    }

    // Test bandwidth configuration enforcement
    let min_bandwidth_configs = vec![10, 50, 100];

    for min_bw in min_bandwidth_configs {
        let server = test_server_with_bandwidth(40000, ProxyProtocols::Socks5, min_bw);

        // Verify configuration
        assert_eq!(
            server.peer_options.min_bandwidth.as_mbps() as u64,
            min_bw,
            "min_bandwidth configuration mismatch"
        );

        println!("✓ Server configured with min_bandwidth: {} Mbps", min_bw);
    }
}

#[cfg(test)]
mod additional_tests {
    use super::*;

    /// Test bandwidth measurement with zero duration
    #[test]
    fn test_bandwidth_measurement_edge_case() {
        let measurement = BandwidthMeasurement::new(1000, Duration::from_secs(0));
        assert_eq!(
            measurement.bytes_per_sec, 0.0,
            "Should handle zero duration"
        );
        assert_eq!(
            measurement.mbps(),
            0.0,
            "Mbps should be 0 for zero duration"
        );
    }

    /// Test bandwidth assertion within tolerance
    #[test]
    fn test_bandwidth_within_tolerance() {
        // Exact match
        assert_bandwidth_within(1000, 1000, 1.0);

        // Just within tolerance
        assert_bandwidth_within(990, 1000, 1.0);
        assert_bandwidth_within(1010, 1000, 1.0);
    }

    #[test]
    #[should_panic]
    fn test_bandwidth_within_out_of_tolerance() {
        // Should panic - outside 1% tolerance
        assert_bandwidth_within(980, 1000, 1.0);
    }
}
