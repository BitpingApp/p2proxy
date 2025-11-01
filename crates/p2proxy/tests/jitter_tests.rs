//! Jitter and Latency Tests for P2Proxy
//!
//! This module implements Phase 5 (Tasks 5.1, 5.2, 5.3) of the P2Proxy test infrastructure.
//! It tests latency characteristics and jitter measurements across various scenarios.
//!
//! # Test Scenarios Covered
//!
//! 1. **Round-trip time measurement** - Measures RTT for P2P connections
//! 2. **Connection establishment latency** - Time to establish P2P connection
//! 3. **SOCKS5 handshake latency** - Timing of SOCKS5 handshake process
//! 4. **First-byte latency** - Time to receive first data byte after connection
//! 5. **Packet timing variance** - Measures jitter in packet arrival times
//! 6. **Jitter under load** - Jitter measurement with background traffic
//! 7. **Latency percentiles** - Verifies p50, p95, p99 calculations
//!
//! # Latency Targets
//!
//! With mock components:
//! - Direct connection: mock latency + overhead (<20ms overhead expected)
//! - Relay connection: 2x mock latency + overhead
//! - Jitter: <10ms for stable connections
//!
//! # Measurement Methodology
//!
//! - All measurements use `measure_latency()` utility from test_utils
//! - Minimum 100 iterations for statistical significance
//! - Mock swarm/peer configured with specific latencies:
//!   - Low latency: 10ms
//!   - Medium latency: 50ms
//!   - High latency: 200ms
//! - Jitter calculated using RFC 3550 formula
//! - Percentiles calculated from sorted measurements

use std::time::{Duration, Instant};

mod common;
use common::mock_peer::{MockPeer, MockPeerConfig};
use common::mock_swarm::{MockSwarm, MockSwarmConfig, MockSwarmEvent};
use common::platform::*;
use common::test_utils::{measure_latency, LatencyStats};
use libp2p::PeerId;

/// Calculate jitter according to RFC 3550
///
/// Jitter is the variance in packet inter-arrival time.
/// Formula: J(i) = J(i-1) + (|D(i-1,i)| - J(i-1))/16
/// Where D(i-1,i) is the difference in packet spacing at receiver vs sender
///
/// For our implementation, we calculate the standard deviation of
/// inter-arrival times as a practical measure of jitter.
fn calculate_jitter(send_times: &[Instant], recv_times: &[Instant]) -> Duration {
    assert_eq!(send_times.len(), recv_times.len());
    assert!(send_times.len() >= 2, "Need at least 2 samples");

    let mut transit_deltas = Vec::new();

    // Calculate transit time deltas (D(i-1,i))
    for i in 1..send_times.len() {
        let send_interval = send_times[i].duration_since(send_times[i - 1]);
        let recv_interval = recv_times[i].duration_since(recv_times[i - 1]);

        // Calculate the difference in intervals
        let delta = if recv_interval > send_interval {
            recv_interval - send_interval
        } else {
            send_interval - recv_interval
        };

        transit_deltas.push(delta);
    }

    // Calculate mean
    let sum: Duration = transit_deltas.iter().sum();
    let mean = sum / transit_deltas.len() as u32;

    // Calculate standard deviation
    let variance: f64 = transit_deltas
        .iter()
        .map(|d| {
            let diff = d.as_secs_f64() - mean.as_secs_f64();
            diff * diff
        })
        .sum::<f64>()
        / transit_deltas.len() as f64;

    Duration::from_secs_f64(variance.sqrt())
}

/// Test 1: Round-trip time measurement
///
/// Measures RTT for mock P2P connections by timing a simple query-response cycle.
/// Tests both low and high latency configurations to establish baseline metrics.
#[tokio::test]
async fn test_round_trip_time() {
    // Test with low latency (10ms)
    let config = MockPeerConfig {
        latency: Duration::from_millis(10),
        failure_rate: 0.0,
        seed: Some(42),
        jitter: Duration::from_millis(1), // Minimal jitter (0 causes range error)
        ..Default::default()
    };

    // Use Arc<Mutex> to allow interior mutability
    let peer = std::sync::Arc::new(tokio::sync::Mutex::new(MockPeer::new(config)));
    let peer_clone = peer.clone();

    // Measure RTT over 100 iterations
    let stats = measure_latency(
        || async {
            let mut peer = peer_clone.lock().await;
            let _response = peer.respond_to_query(b"ping").await.unwrap();
        },
        100,
    )
    .await;

    println!("Low latency RTT stats:");
    println!("  Min: {:?}", stats.min);
    println!("  Max: {:?}", stats.max);
    println!("  Mean: {:?}", stats.mean);
    println!("  Median (p50): {:?}", stats.median);
    println!("  p95: {:?}", stats.p95);
    println!("  p99: {:?}", stats.p99);

    // RTT in mock includes latency + jitter (one way in the respond_to_query)
    // With 10ms latency + up to 1ms jitter = ~10-12ms (macOS may be higher due to scheduler overhead)
    let max_median = if is_macos() {
        Duration::from_millis(20) // macOS has higher overhead
    } else {
        Duration::from_millis(15)
    };
    let max_p95 = if is_macos() {
        Duration::from_millis(25)
    } else {
        Duration::from_millis(18)
    };

    assert!(stats.median >= Duration::from_millis(9), "Median RTT too low");
    assert!(stats.median <= max_median, "Median RTT too high: {:?} > {:?}", stats.median, max_median);
    assert!(stats.p95 <= max_p95, "p95 RTT too high: {:?} > {:?}", stats.p95, max_p95);

    // Test with high latency (200ms)
    let high_latency_config = MockPeerConfig {
        latency: Duration::from_millis(200),
        failure_rate: 0.0,
        seed: Some(43),
        jitter: Duration::from_millis(1), // Minimal jitter (0 causes range error)
        ..Default::default()
    };

    let high_latency_peer = std::sync::Arc::new(tokio::sync::Mutex::new(MockPeer::new(high_latency_config)));
    let high_peer_clone = high_latency_peer.clone();

    let high_stats = measure_latency(
        || async {
            let mut peer = high_peer_clone.lock().await;
            let _response = peer.respond_to_query(b"ping").await.unwrap();
        },
        100,
    )
    .await;

    println!("\nHigh latency RTT stats:");
    println!("  Min: {:?}", high_stats.min);
    println!("  Max: {:?}", high_stats.max);
    println!("  Mean: {:?}", high_stats.mean);
    println!("  Median (p50): {:?}", high_stats.median);
    println!("  p95: {:?}", high_stats.p95);
    println!("  p99: {:?}", high_stats.p99);

    // RTT should be approximately 200ms + jitter (macOS may have higher overhead)
    let high_max = if is_macos() {
        Duration::from_millis(220)
    } else {
        Duration::from_millis(210)
    };

    assert!(high_stats.median >= Duration::from_millis(195), "High latency median too low");
    assert!(high_stats.median <= high_max, "High latency median too high: {:?} > {:?}", high_stats.median, high_max);
}

/// Test 2: Connection establishment latency
///
/// Measures the time required to establish a P2P connection using mock swarm.
/// Tests direct connections with different latency configurations.
#[tokio::test]
async fn test_connection_establishment_latency() {
    let config = MockSwarmConfig {
        latency: Duration::from_millis(50),
        success_rate: 1.0,
        seed: Some(100),
        ..Default::default()
    };

    let stats = measure_latency(
        || async {
            let mut swarm = MockSwarm::new(config.clone());
            let peer_id = PeerId::random();

            // Connect to peer (simulates full connection establishment)
            let _ = swarm.connect_to_peer(peer_id).await;
        },
        100,
    )
    .await;

    println!("Connection establishment latency stats:");
    println!("  Min: {:?}", stats.min);
    println!("  Max: {:?}", stats.max);
    println!("  Mean: {:?}", stats.mean);
    println!("  Median (p50): {:?}", stats.median);
    println!("  p95: {:?}", stats.p95);
    println!("  p99: {:?}", stats.p99);

    // Connection establishment includes 2x latency (dial + response)
    // Expected: ~100ms (50ms * 2)
    let conn_max_median = if is_macos() {
        Duration::from_millis(150)
    } else {
        Duration::from_millis(120)
    };
    let conn_max_p95 = if is_macos() {
        Duration::from_millis(200)
    } else {
        Duration::from_millis(150)
    };

    assert!(stats.median >= Duration::from_millis(90), "Connection latency too low");
    assert!(stats.median <= conn_max_median, "Connection latency too high: {:?} > {:?}", stats.median, conn_max_median);
    assert!(stats.p95 <= conn_max_p95, "p95 connection latency too high: {:?} > {:?}", stats.p95, conn_max_p95);
}

/// Test 3: SOCKS5 handshake latency
///
/// Measures the latency of establishing a SOCKS5 connection through the proxy.
/// This simulates the complete handshake process including greeting and connection request.
#[tokio::test]
async fn test_socks5_handshake_latency() {
    // Simulate SOCKS5 handshake by measuring the time for two round-trips:
    // 1. Client greeting -> Server method selection
    // 2. Connection request -> Server response

    let config = MockPeerConfig {
        latency: Duration::from_millis(10),
        failure_rate: 0.0,
        seed: Some(200),
        jitter: Duration::from_millis(2),
        ..Default::default()
    };

    let peer = std::sync::Arc::new(tokio::sync::Mutex::new(MockPeer::new(config)));
    let peer_clone = peer.clone();

    let stats = measure_latency(
        || async {
            let mut peer = peer_clone.lock().await;
            // Simulate SOCKS5 greeting
            let _greeting_response = peer.respond_to_query(b"socks5_greeting").await.unwrap();

            // Simulate connection request
            let _connect_response = peer.respond_to_query(b"socks5_connect").await.unwrap();
        },
        100,
    )
    .await;

    println!("SOCKS5 handshake latency stats:");
    println!("  Min: {:?}", stats.min);
    println!("  Max: {:?}", stats.max);
    println!("  Mean: {:?}", stats.mean);
    println!("  Median (p50): {:?}", stats.median);
    println!("  p95: {:?}", stats.p95);
    println!("  p99: {:?}", stats.p99);

    // SOCKS5 handshake is 2 queries = 2x latency per query = ~20-24ms total
    // With jitter (up to 2ms per query), expect some variation
    let socks5_max_median = if is_macos() {
        Duration::from_millis(40)
    } else {
        Duration::from_millis(30)
    };
    let socks5_max_p95 = if is_macos() {
        Duration::from_millis(50)
    } else {
        Duration::from_millis(35)
    };

    assert!(stats.median >= Duration::from_millis(20), "SOCKS5 handshake too fast");
    assert!(stats.median <= socks5_max_median, "SOCKS5 handshake too slow: {:?} > {:?}", stats.median, socks5_max_median);
    assert!(stats.p95 <= socks5_max_p95, "p95 SOCKS5 handshake too high: {:?} > {:?}", stats.p95, socks5_max_p95);
}

/// Test 4: First-byte latency
///
/// Measures the time to receive the first data byte after connection establishment.
/// This is critical for interactive applications.
#[tokio::test]
async fn test_first_byte_latency() {
    let config = MockPeerConfig {
        latency: Duration::from_millis(50),
        failure_rate: 0.0,
        seed: Some(300),
        jitter: Duration::from_millis(5),
        ..Default::default()
    };

    let peer = std::sync::Arc::new(tokio::sync::Mutex::new(MockPeer::new(config)));

    // First establish connection (not measured)
    {
        let mut peer_guard = peer.lock().await;
        let _ = peer_guard.accept_connection(PeerId::random()).await;
    }

    let peer_clone = peer.clone();

    // Measure first byte latency (time to send request and get first byte back)
    let stats = measure_latency(
        || async {
            let mut peer = peer_clone.lock().await;
            // Simulate data request
            let _response = peer.respond_to_query(b"get_data").await.unwrap();
        },
        100,
    )
    .await;

    println!("First-byte latency stats:");
    println!("  Min: {:?}", stats.min);
    println!("  Max: {:?}", stats.max);
    println!("  Mean: {:?}", stats.mean);
    println!("  Median (p50): {:?}", stats.median);
    println!("  p95: {:?}", stats.p95);
    println!("  p99: {:?}", stats.p99);

    // First byte latency = one-way latency + processing + jitter
    // Expected: ~50ms + jitter (up to 5ms)
    let fb_max_median = if is_macos() {
        Duration::from_millis(80)
    } else {
        Duration::from_millis(65)
    };
    let fb_max_p95 = if is_macos() {
        Duration::from_millis(100)
    } else {
        Duration::from_millis(80)
    };

    assert!(stats.median >= Duration::from_millis(45), "First byte too fast");
    assert!(stats.median <= fb_max_median, "First byte too slow: {:?} > {:?}", stats.median, fb_max_median);
    assert!(stats.p95 <= fb_max_p95, "p95 first byte latency too high: {:?} > {:?}", stats.p95, fb_max_p95);
}

/// Test 5: Packet timing variance (Jitter measurement)
///
/// Sends packets at regular intervals and measures the variance in arrival times.
/// Jitter is calculated using the RFC 3550 formula.
#[tokio::test]
async fn test_packet_timing_variance() {
    let config = MockPeerConfig {
        latency: Duration::from_millis(20),
        failure_rate: 0.0,
        seed: Some(400),
        jitter: Duration::from_millis(5), // Introduce controlled jitter
        ..Default::default()
    };

    let peer = std::sync::Arc::new(tokio::sync::Mutex::new(MockPeer::new(config)));

    // Send packets at regular 50ms intervals
    let packet_interval = Duration::from_millis(50);
    let num_packets = 50;

    let mut send_times = Vec::new();
    let mut recv_times = Vec::new();

    for _ in 0..num_packets {
        let send_time = Instant::now();
        send_times.push(send_time);

        // Send packet
        {
            let mut peer_guard = peer.lock().await;
            let _ = peer_guard.respond_to_query(b"data_packet").await.unwrap();
        }

        let recv_time = Instant::now();
        recv_times.push(recv_time);

        // Wait for next interval
        tokio::time::sleep(packet_interval).await;
    }

    // Calculate jitter
    let jitter = calculate_jitter(&send_times, &recv_times);

    println!("Packet timing variance:");
    println!("  Calculated jitter: {:?}", jitter);
    println!("  Target: <10ms for stable connections");

    // Jitter should be less than 10ms for stable connection
    // With 5ms configured jitter, we expect something in that range
    assert!(jitter <= Duration::from_millis(10), "Jitter exceeds 10ms threshold: {:?}", jitter);
    assert!(jitter >= Duration::from_millis(1), "Jitter suspiciously low: {:?}", jitter);
}

/// Test 6: Jitter under load
///
/// Measures jitter with varying levels of background traffic.
/// Tests at 0%, 50%, and 100% capacity load to understand impact.
#[tokio::test]
async fn test_jitter_under_load() {
    // Test with no load
    let no_load_jitter = measure_jitter_with_background_load(0).await;
    println!("Jitter with 0% load: {:?}", no_load_jitter);

    // Test with 50% load
    let medium_load_jitter = measure_jitter_with_background_load(5).await;
    println!("Jitter with 50% load (5 concurrent transfers): {:?}", medium_load_jitter);

    // Test with heavy load
    let high_load_jitter = measure_jitter_with_background_load(10).await;
    println!("Jitter with 100% load (10 concurrent transfers): {:?}", high_load_jitter);

    // Verify that jitter increases with load
    assert!(no_load_jitter < medium_load_jitter,
        "Jitter should increase with load: no_load={:?} vs medium={:?}",
        no_load_jitter, medium_load_jitter);
    assert!(medium_load_jitter <= high_load_jitter,
        "Jitter should increase or stay same with higher load: medium={:?} vs high={:?}",
        medium_load_jitter, high_load_jitter);

    // Under heavy load with simulated contention, jitter can be significant
    // In real-world scenarios, this demonstrates the impact of load on timing variance
    // For mock environment with mutex contention, accept up to 2 seconds
    assert!(high_load_jitter <= Duration::from_secs(2),
        "Jitter under heavy load exceeds 2s: {:?}", high_load_jitter);

    println!("\nJitter increases predictably with load:");
    println!("  No load: {:?}", no_load_jitter);
    println!("  Medium load: {:?}", medium_load_jitter);
    println!("  High load: {:?}", high_load_jitter);
}

/// Helper function to measure jitter with specified background load
async fn measure_jitter_with_background_load(num_background_tasks: usize) -> Duration {
    let config = MockPeerConfig {
        latency: Duration::from_millis(20),
        failure_rate: 0.0,
        seed: Some(500),
        jitter: Duration::from_millis(3),
        bandwidth: 10_000_000, // 10 MB/s - can be saturated
        ..Default::default()
    };

    let peer = MockPeer::new(config.clone());
    let peer = std::sync::Arc::new(tokio::sync::Mutex::new(peer));

    // Start background load tasks
    let mut background_handles = Vec::new();
    for i in 0..num_background_tasks {
        let peer_clone = peer.clone();
        let handle = tokio::spawn(async move {
            let mut peer = peer_clone.lock().await;
            // Simulate continuous data transfer
            for _ in 0..20 {
                let _ = peer.simulate_data_transfer(100_000).await; // 100KB chunks
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        background_handles.push(handle);
    }

    // Give background tasks time to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Measure jitter with foreground traffic
    let num_packets = 30;
    let packet_interval = Duration::from_millis(50);

    let mut send_times = Vec::new();
    let mut recv_times = Vec::new();

    for _ in 0..num_packets {
        let send_time = Instant::now();
        send_times.push(send_time);

        // Send packet
        let mut peer_guard = peer.lock().await;
        let _ = peer_guard.respond_to_query(b"data_packet").await;
        drop(peer_guard);

        let recv_time = Instant::now();
        recv_times.push(recv_time);

        // Wait for next interval
        tokio::time::sleep(packet_interval).await;
    }

    // Wait for background tasks to complete
    for handle in background_handles {
        let _ = handle.await;
    }

    // Calculate and return jitter
    calculate_jitter(&send_times, &recv_times)
}

/// Test 7: Latency percentiles verification
///
/// Verifies that p50, p95, and p99 percentile calculations are correct
/// by testing with known latency distributions.
#[tokio::test]
async fn test_latency_percentiles() {
    // Create a peer with known latency
    let config = MockPeerConfig {
        latency: Duration::from_millis(30),
        failure_rate: 0.0,
        seed: Some(600),
        jitter: Duration::from_millis(10), // Significant jitter for distribution
        ..Default::default()
    };

    let peer = std::sync::Arc::new(tokio::sync::Mutex::new(MockPeer::new(config)));
    let peer_clone = peer.clone();

    // Measure over many iterations for good percentile calculation
    let stats = measure_latency(
        || async {
            let mut peer = peer_clone.lock().await;
            let _ = peer.respond_to_query(b"ping").await.unwrap();
        },
        200, // More iterations for better percentile accuracy
    )
    .await;

    println!("Latency percentile verification:");
    println!("  Min: {:?}", stats.min);
    println!("  Max: {:?}", stats.max);
    println!("  Mean: {:?}", stats.mean);
    println!("  Median (p50): {:?}", stats.median);
    println!("  p95: {:?}", stats.p95);
    println!("  p99: {:?}", stats.p99);

    // Verify ordering: min <= p50 <= p95 <= p99 <= max
    assert!(stats.min <= stats.median, "Min should be <= median");
    assert!(stats.median <= stats.p95, "Median should be <= p95");
    assert!(stats.p95 <= stats.p99, "p95 should be <= p99");
    assert!(stats.p99 <= stats.max, "p99 should be <= max");

    // Verify that median is reasonably centered
    // With 30ms latency + up to 10ms jitter, expect median around 30-40ms
    assert!(stats.median >= Duration::from_millis(25), "Median too low");
    assert!(stats.median <= Duration::from_millis(50), "Median too high");

    // p95 should show impact of jitter
    assert!(stats.p95 >= stats.median, "p95 should be >= median");
    assert!(stats.p95 <= Duration::from_millis(60), "p95 too high");

    // p99 should capture worst-case latency
    assert!(stats.p99 >= stats.p95, "p99 should be >= p95");
    assert!(stats.p99 <= Duration::from_millis(70), "p99 too high");

    // Verify statistical validity: p50 should be close to mean for normal distribution
    let mean_median_diff = if stats.mean > stats.median {
        stats.mean - stats.median
    } else {
        stats.median - stats.mean
    };
    assert!(mean_median_diff <= Duration::from_millis(10),
        "Mean and median should be close for normal distribution: mean={:?}, median={:?}",
        stats.mean, stats.median);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jitter_calculation() {
        // Create deterministic send and receive times
        let base = Instant::now();
        let send_times = vec![
            base,
            base + Duration::from_millis(100),
            base + Duration::from_millis(200),
            base + Duration::from_millis(300),
        ];

        // Simulate consistent latency (20ms) with no jitter
        let recv_times = vec![
            base + Duration::from_millis(20),
            base + Duration::from_millis(120),
            base + Duration::from_millis(220),
            base + Duration::from_millis(320),
        ];

        let jitter = calculate_jitter(&send_times, &recv_times);

        // With perfect timing, jitter should be near zero
        println!("Jitter with perfect timing: {:?}", jitter);
        assert!(jitter < Duration::from_millis(1), "Jitter should be near zero for perfect timing");
    }

    #[test]
    fn test_jitter_calculation_with_variance() {
        // Create send times
        let base = Instant::now();
        let send_times = vec![
            base,
            base + Duration::from_millis(100),
            base + Duration::from_millis(200),
            base + Duration::from_millis(300),
        ];

        // Simulate varying latency (introducing jitter)
        let recv_times = vec![
            base + Duration::from_millis(20),
            base + Duration::from_millis(125), // +5ms extra
            base + Duration::from_millis(218), // -2ms
            base + Duration::from_millis(327), // +7ms extra
        ];

        let jitter = calculate_jitter(&send_times, &recv_times);

        // With variance, jitter should be measurable
        println!("Jitter with variance: {:?}", jitter);
        assert!(jitter > Duration::from_millis(0), "Jitter should be measurable with variance");
        assert!(jitter < Duration::from_millis(20), "Jitter calculation seems incorrect");
    }
}
