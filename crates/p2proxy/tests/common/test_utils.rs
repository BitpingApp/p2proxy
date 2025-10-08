//! Test utility functions and helpers
//!
//! This module provides helper functions for common test operations:
//! - Creating test configurations with custom settings
//! - Creating deterministic test keypairs
//! - Waiting for connections with timeout
//! - Asserting bandwidth metrics
//! - Creating mock SOCKS5 connections
//! - Various assertion helpers

use color_eyre::eyre::{eyre, Result};
use models::events::Events;
use socks5_impl::protocol::Address;
use std::future::Future;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Measurement of bandwidth usage during an operation
///
/// Contains statistics about data transfer including total bytes transferred,
/// duration of the operation, and calculated bytes per second.
#[derive(Debug, Clone)]
pub struct BandwidthMeasurement {
    /// Total number of bytes transferred
    pub total_bytes: u64,
    /// Duration of the transfer
    pub duration: Duration,
    /// Transfer rate in bytes per second
    pub bytes_per_sec: f64,
}

impl BandwidthMeasurement {
    /// Create a new bandwidth measurement
    pub fn new(total_bytes: u64, duration: Duration) -> Self {
        let bytes_per_sec = if duration.as_secs_f64() > 0.0 {
            total_bytes as f64 / duration.as_secs_f64()
        } else {
            0.0
        };

        Self {
            total_bytes,
            duration,
            bytes_per_sec,
        }
    }

    /// Get throughput in megabytes per second
    pub fn mbps(&self) -> f64 {
        self.bytes_per_sec * 8.0 / 1_000_000.0
    }
}

/// Statistics about operation latency
///
/// Contains percentile-based latency measurements useful for understanding
/// the distribution of operation times.
#[derive(Debug, Clone)]
pub struct LatencyStats {
    /// Minimum latency observed
    pub min: Duration,
    /// Maximum latency observed
    pub max: Duration,
    /// Mean (average) latency
    pub mean: Duration,
    /// Median (50th percentile) latency
    pub median: Duration,
    /// 95th percentile latency
    pub p95: Duration,
    /// 99th percentile latency
    pub p99: Duration,
}

/// Creates a mock SOCKS5 client connection to the specified port
///
/// This function establishes a TCP connection to a SOCKS5 proxy server and
/// performs the SOCKS5 handshake. It's useful for testing proxy functionality.
///
/// # Arguments
///
/// * `port` - The port number of the SOCKS5 proxy server
/// * `target` - The target address to request through the proxy
///
/// # Returns
///
/// A `TcpStream` connected to the proxy and ready for data transfer
///
/// # Example
///
/// ```no_run
/// use common::test_utils::mock_socks5_client;
/// use socks5_impl::protocol::Address;
///
/// #[tokio::test]
/// async fn test_socks5_connection() {
///     let target = Address::DomainAddress("example.com".to_string(), 80);
///     let stream = mock_socks5_client(1080, target).await.unwrap();
///     // Use stream for data transfer
/// }
/// ```
pub async fn mock_socks5_client(port: u16, target: Address) -> Result<TcpStream> {
    // Connect to SOCKS5 proxy
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).await?;

    // Perform SOCKS5 handshake
    socks5_handshake(&mut stream).await?;

    // Send connection request
    socks5_connect(&mut stream, target).await?;

    Ok(stream)
}

/// Performs SOCKS5 handshake with the proxy server
///
/// This function sends the SOCKS5 greeting and negotiates the authentication method.
/// Currently only supports "no authentication" (0x00).
///
/// # Arguments
///
/// * `stream` - Mutable reference to the TCP stream connected to the proxy
///
/// # Returns
///
/// `Ok(())` if handshake succeeds, error otherwise
///
/// # Example
///
/// ```no_run
/// use common::test_utils::socks5_handshake;
/// use tokio::net::TcpStream;
///
/// #[tokio::test]
/// async fn test_handshake() {
///     let mut stream = TcpStream::connect("127.0.0.1:1080").await.unwrap();
///     socks5_handshake(&mut stream).await.unwrap();
/// }
/// ```
pub async fn socks5_handshake(stream: &mut TcpStream) -> Result<()> {
    // Send SOCKS5 greeting: version(5) + num_methods(1) + method(0 = no auth)
    stream.write_all(&[0x05, 0x01, 0x00]).await?;

    // Read server response: version(5) + chosen_method
    let mut response = [0u8; 2];
    stream.read_exact(&mut response).await?;

    if response[0] != 0x05 {
        return Err(eyre!("Invalid SOCKS version: {}", response[0]));
    }

    if response[1] != 0x00 {
        return Err(eyre!(
            "Server rejected no-auth method, selected: {}",
            response[1]
        ));
    }

    Ok(())
}

/// Sends SOCKS5 connection request to the proxy
///
/// After the handshake, this function sends a CONNECT request for the target address.
///
/// # Arguments
///
/// * `stream` - Mutable reference to the TCP stream
/// * `target` - Target address to connect to
///
/// # Returns
///
/// `Ok(())` if connection request succeeds
async fn socks5_connect(stream: &mut TcpStream, target: Address) -> Result<()> {
    let mut request = vec![0x05, 0x01, 0x00]; // version, connect command, reserved

    // Encode target address
    match target {
        Address::SocketAddress(socket_addr) => {
            match socket_addr {
                std::net::SocketAddr::V4(addr) => {
                    request.push(0x01); // IPv4 address type
                    request.extend_from_slice(&addr.ip().octets());
                    request.extend_from_slice(&addr.port().to_be_bytes());
                }
                std::net::SocketAddr::V6(addr) => {
                    request.push(0x04); // IPv6 address type
                    request.extend_from_slice(&addr.ip().octets());
                    request.extend_from_slice(&addr.port().to_be_bytes());
                }
            }
        }
        Address::DomainAddress(domain, port) => {
            request.push(0x03); // Domain name address type
            request.push(domain.len() as u8);
            request.extend_from_slice(domain.as_bytes());
            request.extend_from_slice(&port.to_be_bytes());
        }
    }

    stream.write_all(&request).await?;

    // Read server response (at least 10 bytes for IPv4)
    let mut response = vec![0u8; 10];
    stream.read_exact(&mut response[..4]).await?;

    if response[0] != 0x05 {
        return Err(eyre!("Invalid SOCKS version in response: {}", response[0]));
    }

    if response[1] != 0x00 {
        return Err(eyre!("Connection failed with code: {}", response[1]));
    }

    // Read rest of response based on address type
    match response[3] {
        0x01 => {
            // IPv4: 4 bytes IP + 2 bytes port
            stream.read_exact(&mut response[4..10]).await?;
        }
        0x03 => {
            // Domain: 1 byte len + domain + 2 bytes port
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut domain_and_port = vec![0u8; len[0] as usize + 2];
            stream.read_exact(&mut domain_and_port).await?;
        }
        0x04 => {
            // IPv6: 16 bytes IP + 2 bytes port
            let mut ipv6_and_port = vec![0u8; 18];
            stream.read_exact(&mut ipv6_and_port).await?;
        }
        _ => return Err(eyre!("Unknown address type: {}", response[3])),
    }

    Ok(())
}

/// Asserts that an actual bandwidth value is within a tolerance of the expected value
///
/// This is useful for bandwidth tests where exact matching is not practical due to
/// network overhead, timing variations, etc.
///
/// # Arguments
///
/// * `actual` - The actual measured bandwidth in bytes
/// * `expected` - The expected bandwidth in bytes
/// * `tolerance_pct` - Tolerance as a percentage (e.g., 5.0 for 5%)
///
/// # Panics
///
/// Panics if the actual value is outside the tolerance range
///
/// # Example
///
/// ```no_run
/// use common::test_utils::assert_bandwidth_within;
///
/// // Assert that 980KB is within 5% of 1MB
/// assert_bandwidth_within(980_000, 1_000_000, 5.0);
/// ```
pub fn assert_bandwidth_within(actual: u64, expected: u64, tolerance_pct: f64) {
    let tolerance = (expected as f64 * tolerance_pct / 100.0) as u64;
    let min = expected.saturating_sub(tolerance);
    let max = expected.saturating_add(tolerance);

    assert!(
        actual >= min && actual <= max,
        "Bandwidth {} is outside tolerance range [{}, {}] (expected {}, tolerance {}%)",
        actual,
        min,
        max,
        expected,
        tolerance_pct
    );
}

/// Waits for an event matching the predicate within the timeout period
///
/// This function polls for events and returns the first one that matches the predicate.
/// Useful for testing event-driven systems.
///
/// # Arguments
///
/// * `predicate` - A function that returns true when the desired event is found
/// * `timeout_duration` - Maximum time to wait for the event
///
/// # Returns
///
/// The matching `Events` instance if found within the timeout
///
/// # Example
///
/// ```no_run
/// use common::test_utils::wait_for_event;
/// use models::events::Events;
/// use std::time::Duration;
///
/// #[tokio::test]
/// async fn test_wait_for_connection() {
///     let event = wait_for_event(
///         |e| matches!(e, Events::Connection(_)),
///         Duration::from_secs(5)
///     ).await.unwrap();
/// }
/// ```
pub async fn wait_for_event<P>(
    predicate: P,
    timeout_duration: Duration,
) -> Result<Events>
where
    P: Fn(&Events) -> bool,
{
    // Note: This is a simplified implementation. In practice, you would need
    // access to the actual event stream/receiver from the system under test.
    // This signature matches the requirement but the implementation would need
    // to be adapted to your specific event system.

    Err(eyre!(
        "wait_for_event needs to be connected to actual event stream"
    ))
}

/// Measures bandwidth during an async operation
///
/// Executes the provided operation and measures how much data was transferred
/// and how long it took.
///
/// # Arguments
///
/// * `operation` - An async function that performs the data transfer
///
/// # Returns
///
/// A `BandwidthMeasurement` with transfer statistics
///
/// # Note
///
/// The actual byte counting needs to be done within the operation itself.
/// This is a framework for the measurement.
///
/// # Example
///
/// ```no_run
/// use common::test_utils::measure_bandwidth;
///
/// #[tokio::test]
/// async fn test_transfer_bandwidth() {
///     let measurement = measure_bandwidth(|| async {
///         // Perform data transfer
///     }).await;
///
///     println!("Transfer rate: {} Mbps", measurement.mbps());
/// }
/// ```
pub async fn measure_bandwidth<F, Fut>(operation: F) -> BandwidthMeasurement
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    let start = Instant::now();
    operation().await;
    let duration = start.elapsed();

    // Note: In practice, the operation would need to return the number of bytes transferred
    // This is a simplified version
    BandwidthMeasurement::new(0, duration)
}

/// Measures latency statistics for an operation over multiple iterations
///
/// Runs the operation multiple times and calculates latency percentiles.
///
/// # Arguments
///
/// * `operation` - The async operation to measure
/// * `iterations` - Number of times to run the operation
///
/// # Returns
///
/// A `LatencyStats` struct with percentile measurements
///
/// # Example
///
/// ```no_run
/// use common::test_utils::measure_latency;
///
/// #[tokio::test]
/// async fn test_connection_latency() {
///     let stats = measure_latency(
///         || async {
///             // Perform operation
///         },
///         100
///     ).await;
///
///     println!("P95 latency: {:?}", stats.p95);
///     println!("P99 latency: {:?}", stats.p99);
/// }
/// ```
pub async fn measure_latency<F, Fut>(operation: F, iterations: usize) -> LatencyStats
where
    F: Fn() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut measurements = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        let start = Instant::now();
        operation().await;
        measurements.push(start.elapsed());
    }

    // Sort measurements for percentile calculation
    measurements.sort();

    let min = *measurements.first().unwrap_or(&Duration::ZERO);
    let max = *measurements.last().unwrap_or(&Duration::ZERO);

    let sum: Duration = measurements.iter().sum();
    let mean = sum / iterations as u32;

    let median_idx = iterations / 2;
    let median = measurements.get(median_idx).copied().unwrap_or(Duration::ZERO);

    let p95_idx = (iterations as f64 * 0.95) as usize;
    let p95 = measurements.get(p95_idx).copied().unwrap_or(max);

    let p99_idx = (iterations as f64 * 0.99) as usize;
    let p99 = measurements.get(p99_idx).copied().unwrap_or(max);

    LatencyStats {
        min,
        max,
        mean,
        median,
        p95,
        p99,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bandwidth_measurement() {
        let measurement = BandwidthMeasurement::new(1_000_000, Duration::from_secs(1));

        assert_eq!(measurement.total_bytes, 1_000_000);
        assert_eq!(measurement.bytes_per_sec, 1_000_000.0);
        assert_eq!(measurement.mbps(), 8.0); // 1MB/s = 8Mbps
    }

    #[test]
    fn test_assert_bandwidth_within_success() {
        // Should not panic
        assert_bandwidth_within(95_000, 100_000, 10.0);
        assert_bandwidth_within(105_000, 100_000, 10.0);
    }

    #[test]
    #[should_panic]
    fn test_assert_bandwidth_within_failure() {
        // Should panic - outside 10% tolerance
        assert_bandwidth_within(85_000, 100_000, 10.0);
    }

    #[tokio::test]
    async fn test_measure_latency() {
        let stats = measure_latency(
            || async {
                tokio::time::sleep(Duration::from_millis(10)).await;
            },
            10,
        )
        .await;

        // Latencies should be around 10ms
        assert!(stats.min >= Duration::from_millis(9));
        assert!(stats.max <= Duration::from_millis(20));
        assert!(stats.mean >= Duration::from_millis(9));
        assert!(stats.median >= Duration::from_millis(9));
    }
}
