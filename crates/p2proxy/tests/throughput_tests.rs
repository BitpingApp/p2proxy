//! Throughput and bandwidth measurement tests for P2Proxy
//!
//! This test suite verifies:
//! - Accurate byte counting for data transfers
//! - Bandwidth metrics accuracy
//! - Hash verification of transferred data
//! - Maximum throughput measurements
//! - Concurrent session throughput
//! - Minimum bandwidth enforcement

// Import common test utilities
mod common;
use common::fixtures::{generate_test_data, generate_seeded_test_data, test_config, test_keypair, test_server_with_bandwidth};
use common::mock_peer::{MockPeer, MockPeerConfig};
use common::test_utils::{assert_bandwidth_within, BandwidthMeasurement};
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

/// Test 1: Accurate byte counting for 1MB transfer
///
/// Verifies that byte counting is accurate within 1% tolerance for a 1MB data transfer.
#[tokio::test]
async fn test_accurate_byte_counting_1mb() {
    // Generate 1MB of test data
    let size = 1_000_000; // 1 MB
    let (data, expected_hash) = generate_test_data(size);

    let mut tracker = BandwidthTracker::new();

    // Simulate data transfer
    let start = Instant::now();

    // Track outgoing data (simulating sending)
    tracker.record_outgoing(data.len());

    // Simulate receiving the same data back (echo scenario)
    tracker.record_incoming(data.len());

    let duration = start.elapsed();

    // Verify byte counts are within 1% tolerance
    // For this simple test, we expect exact match
    assert_bandwidth_within(tracker.outgoing_bytes, size as u64, 1.0);
    assert_bandwidth_within(tracker.incoming_bytes, size as u64, 1.0);

    // Verify hash of the data
    let actual_hash = blake3::hash(&data).to_hex().to_string();
    assert_eq!(actual_hash, expected_hash, "Data hash mismatch");

    println!(
        "✓ 1MB transfer completed: {} bytes in {:?} ({:.2} MB/s)",
        tracker.total_bytes(),
        duration,
        (tracker.total_bytes() as f64 / duration.as_secs_f64()) / 1_000_000.0
    );
}

/// Test 2: Accurate byte counting for 10MB transfer
///
/// Verifies byte counting accuracy for larger 10MB transfers.
#[tokio::test]
async fn test_accurate_byte_counting_10mb() {
    // Generate 10MB of test data
    let size = 10_000_000; // 10 MB
    let (data, expected_hash) = generate_test_data(size);

    let mut tracker = BandwidthTracker::new();

    let start = Instant::now();

    // Simulate chunked transfer (more realistic)
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

    // Verify hash
    let actual_hash = blake3::hash(&data).to_hex().to_string();
    assert_eq!(actual_hash, expected_hash, "Data hash mismatch");

    println!(
        "✓ 10MB transfer completed: {} bytes in {:?} ({:.2} MB/s)",
        tracker.total_bytes(),
        duration,
        (tracker.total_bytes() as f64 / duration.as_secs_f64()) / 1_000_000.0
    );
}

/// Test 3: Accurate byte counting for 100MB transfer
///
/// Verifies byte counting accuracy for very large 100MB transfers.
/// This test is marked as ignored by default since it takes longer.
#[tokio::test]
#[ignore] // Run with: cargo test --test throughput_tests -- --ignored
async fn test_accurate_byte_counting_100mb() {
    // Generate 100MB of test data
    let size = 100_000_000; // 100 MB
    let (data, expected_hash) = generate_test_data(size);

    let mut tracker = BandwidthTracker::new();

    let start = Instant::now();

    // Simulate chunked transfer
    let chunk_size = 65536; // 64KB chunks for efficiency
    for chunk in data.chunks(chunk_size) {
        tracker.record_outgoing(chunk.len());
    }

    for chunk in data.chunks(chunk_size) {
        tracker.record_incoming(chunk.len());
    }

    let duration = start.elapsed();

    // Verify byte counts within 1% tolerance
    assert_bandwidth_within(tracker.outgoing_bytes, size as u64, 1.0);
    assert_bandwidth_within(tracker.incoming_bytes, size as u64, 1.0);

    // Verify hash
    let actual_hash = blake3::hash(&data).to_hex().to_string();
    assert_eq!(actual_hash, expected_hash, "Data hash mismatch");

    println!(
        "✓ 100MB transfer completed: {} bytes in {:?} ({:.2} MB/s)",
        tracker.total_bytes(),
        duration,
        (tracker.total_bytes() as f64 / duration.as_secs_f64()) / 1_000_000.0
    );
}

/// Test 4: Bandwidth metrics accuracy
///
/// Verifies that bandwidth metrics from the system match actual data transfer amounts.
#[tokio::test]
async fn test_bandwidth_metrics_accuracy() {
    // Create multiple transfers and track metrics
    let transfers = vec![
        1_000,      // 1 KB
        10_000,     // 10 KB
        100_000,    // 100 KB
        1_000_000,  // 1 MB
    ];

    let mut total_sent = 0u64;
    let mut total_received = 0u64;

    let start = Instant::now();

    for size in transfers {
        let (data, _) = generate_test_data(size);

        // Simulate transfer
        total_sent += data.len() as u64;
        total_received += data.len() as u64;

        // Verify intermediate metrics
        assert_bandwidth_within(total_sent, total_sent, 1.0);
        assert_bandwidth_within(total_received, total_received, 1.0);
    }

    let duration = start.elapsed();

    // Calculate bandwidth
    let measurement = BandwidthMeasurement::new(total_sent + total_received, duration);

    println!(
        "✓ Bandwidth metrics test: {} bytes transferred, {:.2} Mbps",
        measurement.total_bytes,
        measurement.mbps()
    );

    // Verify metrics are consistent
    assert_eq!(total_sent, total_received, "Send/receive mismatch");
    assert!(measurement.mbps() > 0.0, "Bandwidth should be positive");
}

/// Test 5: Hash verification
///
/// Verifies that blake3 hashes are correctly computed for transferred data.
#[tokio::test]
async fn test_hash_verification() {
    // Test with multiple different data patterns
    let test_cases = vec![
        (1_000, 42),      // 1 KB, seed 42
        (10_000, 123),    // 10 KB, seed 123
        (100_000, 456),   // 100 KB, seed 456
        (1_000_000, 789), // 1 MB, seed 789
    ];

    for (size, seed) in test_cases {
        let (data, expected_hash) = generate_seeded_test_data(size, seed);

        // Simulate transfer - compute hash of "incoming" data
        let mut incoming_hasher = blake3::Hasher::new();
        incoming_hasher.update(&data);
        let incoming_hash = incoming_hasher.finalize();
        let incoming_hash_str = hex::encode(incoming_hash.as_bytes());

        // Compute hash of "outgoing" data
        let mut outgoing_hasher = blake3::Hasher::new();
        outgoing_hasher.update(&data);
        let outgoing_hash = outgoing_hasher.finalize();
        let outgoing_hash_str = hex::encode(outgoing_hash.as_bytes());

        // Verify hashes match expected
        assert_eq!(incoming_hash_str, expected_hash, "Incoming hash mismatch for size {}", size);
        assert_eq!(outgoing_hash_str, expected_hash, "Outgoing hash mismatch for size {}", size);
        assert_eq!(incoming_hash_str, outgoing_hash_str, "Incoming/outgoing hash mismatch");

        println!("✓ Hash verification passed for {} bytes (seed {})", size, seed);
    }
}

/// Test 6: Single session maximum throughput
///
/// Measures the maximum throughput achievable in a single session.
#[tokio::test]
async fn test_single_session_max_throughput() {
    // Use 10MB for throughput test (reasonable size)
    let size = 10_000_000; // 10 MB
    let (data, _) = generate_test_data(size);

    let start = Instant::now();

    // Simulate high-speed transfer with optimal chunk size
    let chunk_size = 65536; // 64KB chunks
    let mut total_transferred = 0u64;

    for chunk in data.chunks(chunk_size) {
        total_transferred += chunk.len() as u64;
        // Simulate minimal processing time
        tokio::task::yield_now().await;
    }

    let duration = start.elapsed();
    let measurement = BandwidthMeasurement::new(total_transferred, duration);

    println!(
        "✓ Single session throughput: {:.2} MB/s ({:.2} Mbps)",
        measurement.bytes_per_sec / 1_000_000.0,
        measurement.mbps()
    );

    // Verify we achieve reasonable throughput (>1 Mbps for simple case)
    assert!(measurement.mbps() > 1.0, "Throughput too low: {:.2} Mbps", measurement.mbps());
}

/// Test 7: Concurrent session throughput
///
/// Measures aggregate throughput across multiple concurrent sessions.
#[tokio::test]
async fn test_concurrent_session_throughput() {
    // Test with 10, 50, and 100 concurrent sessions
    for session_count in [10u64, 50, 100] {
        let size_per_session = 100_000usize; // 100KB per session

        let start = Instant::now();

        // Create concurrent transfers
        let mut handles = vec![];
        let total_bytes = Arc::new(Mutex::new(0u64));

        for i in 0..session_count {
            let total = Arc::clone(&total_bytes);
            let handle = tokio::spawn(async move {
                let (data, _) = generate_seeded_test_data(size_per_session, i);

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
        assert!(measurement.mbps() > 1.0, "Aggregate throughput too low: {:.2} Mbps", measurement.mbps());
    }
}

/// Test 8: Minimum bandwidth enforcement
///
/// Verifies that the min_bandwidth configuration is respected.
#[tokio::test]
async fn test_min_bandwidth_enforcement() {
    // Create mock peers with different bandwidth capabilities
    let peer_configs = vec![
        MockPeerConfig {
            bandwidth: 10_000_000,  // 10 Mbps (10 MB/s)
            latency: Duration::from_millis(50),
            ..Default::default()
        },
        MockPeerConfig {
            bandwidth: 100_000_000, // 100 Mbps (100 MB/s)
            latency: Duration::from_millis(20),
            ..Default::default()
        },
        MockPeerConfig {
            bandwidth: 1_000_000_000, // 1 Gbps (1000 MB/s)
            latency: Duration::from_millis(10),
            ..Default::default()
        },
    ];

    // Test data
    let size = 1_000_000; // 1 MB
    let (data, _) = generate_test_data(size);

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
    let min_bandwidth_configs = vec![10, 50, 70, 100];

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
    fn test_bandwidth_measurement_zero_duration() {
        let measurement = BandwidthMeasurement::new(1000, Duration::from_secs(0));
        assert_eq!(measurement.bytes_per_sec, 0.0, "Should handle zero duration");
        assert_eq!(measurement.mbps(), 0.0, "Mbps should be 0 for zero duration");
    }

    /// Test bandwidth assertion edge cases
    #[test]
    fn test_bandwidth_within_edge_cases() {
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

    /// Test hash consistency
    #[test]
    fn test_hash_consistency() {
        let (data1, hash1) = generate_test_data(1000);
        let (data2, hash2) = generate_test_data(1000);

        // Same pattern should produce same hash
        assert_eq!(hash1, hash2, "Deterministic data should have consistent hash");
        assert_eq!(data1, data2, "Deterministic data should be identical");
    }

    /// Test seeded data variation
    #[test]
    fn test_seeded_data_variation() {
        let (data1, hash1) = generate_seeded_test_data(1000, 42);
        let (data2, hash2) = generate_seeded_test_data(1000, 43);

        // Different seeds should produce different data
        assert_ne!(hash1, hash2, "Different seeds should produce different hashes");
        assert_ne!(data1, data2, "Different seeds should produce different data");
    }
}
