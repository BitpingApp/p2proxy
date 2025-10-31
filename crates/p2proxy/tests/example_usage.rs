// Example usage of test fixtures and utilities
//
// This file demonstrates how to use the test infrastructure in real tests.

mod common;

#[cfg(test)]
mod examples {
    use crate::common::*;
    use models::config::ProxyProtocols;
    use std::time::Duration;

    /// Example: Creating a simple test configuration
    #[test]
    fn example_simple_config() {
        // Create a SOCKS5 server on port 1080
        let socks_server = test_server(1080, ProxyProtocols::Socks5);

        // Create a configuration with this server
        let config = test_config(vec![socks_server]);

        // Verify the configuration
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].port, 1080);
        assert_eq!(config.port, 45445); // P2P port
    }

    /// Example: Creating a configuration with country requirements
    #[test]
    fn example_config_with_country() {
        // Create servers with different country requirements
        let austria_server = test_server_with_country(1080, ProxyProtocols::Socks5, "AT");
        let us_server = test_server_with_country(1081, ProxyProtocols::Socks5, "US");

        let config = test_config(vec![austria_server, us_server]);

        assert_eq!(config.servers.len(), 2);
        assert_eq!(
            config.servers[0].peer_options.country,
            Some("AT".to_string())
        );
        assert_eq!(
            config.servers[1].peer_options.country,
            Some("US".to_string())
        );
    }

    /// Example: Creating a configuration with bandwidth requirements
    #[test]
    fn example_config_with_bandwidth() {
        // Create a server requiring 100 Mbps minimum bandwidth
        let high_bandwidth_server =
            test_server_with_bandwidth(1080, ProxyProtocols::Socks5, 100);

        let config = test_config(vec![high_bandwidth_server]);

        assert_eq!(config.servers[0].peer_options.min_bandwidth.as_mbps(), 100);
    }

    /// Example: Using deterministic keypairs
    #[test]
    fn example_deterministic_keypairs() {
        // Create two keypairs with the same seed
        let keypair1 = test_keypair(42);
        let keypair2 = test_keypair(42);

        // They should have the same peer ID
        assert_eq!(
            keypair1.public().to_peer_id(),
            keypair2.public().to_peer_id()
        );

        // Different seed produces different keypair
        let keypair3 = test_keypair(99);
        assert_ne!(
            keypair1.public().to_peer_id(),
            keypair3.public().to_peer_id()
        );
    }

    /// Example: Generating test data for transfers
    #[test]
    fn example_test_data_generation() {
        // Generate 1MB of test data
        let (data, hash) = generate_test_data(1_000_000);

        assert_eq!(data.len(), 1_000_000);

        // The hash can be used to verify data integrity after transfer
        let verified_hash = blake3::hash(&data).to_hex().to_string();
        assert_eq!(hash, verified_hash);
    }

    /// Example: Generating seeded test data
    #[test]
    fn example_seeded_test_data() {
        // Generate different test data with different seeds
        let (data1, _) = generate_seeded_test_data(1024, 1);
        let (data2, _) = generate_seeded_test_data(1024, 2);

        // Different seeds produce different data
        assert_ne!(data1, data2);

        // Same seed always produces the same data
        let (data3, _) = generate_seeded_test_data(1024, 1);
        assert_eq!(data1, data3);
    }

    /// Example: Using the TEST_PORTS range
    #[test]
    fn example_test_ports() {
        // Pick a port from the safe test range
        let test_port = TEST_PORTS.start + 42; // 40042

        assert!(TEST_PORTS.contains(&test_port));

        // Create a server with this port
        let server = test_server(test_port, ProxyProtocols::Socks5);
        assert_eq!(server.port, test_port);
    }

    /// Example: Bandwidth measurement usage
    #[test]
    fn example_bandwidth_measurement() {
        // Create a bandwidth measurement
        let measurement = BandwidthMeasurement::new(10_000_000, Duration::from_secs(1));

        // Check the transfer rate
        assert_eq!(measurement.total_bytes, 10_000_000);
        assert_eq!(measurement.bytes_per_sec, 10_000_000.0);
        assert_eq!(measurement.mbps(), 80.0); // 10MB/s = 80Mbps
    }

    /// Example: Bandwidth assertions with tolerance
    #[test]
    fn example_bandwidth_assertions() {
        // Assert that 95KB is within 10% of 100KB
        assert_bandwidth_within(95_000, 100_000, 10.0);

        // This is useful for real network tests where exact values are hard to achieve
        let measured_bytes = 980_000;
        let expected_bytes = 1_000_000;
        assert_bandwidth_within(measured_bytes, expected_bytes, 5.0);
    }

    /// Example: Measuring operation latency
    #[tokio::test]
    async fn example_latency_measurement() {
        // Measure latency of an operation over 10 iterations
        let stats = measure_latency(
            || async {
                // Simulate some async operation
                tokio::time::sleep(Duration::from_millis(10)).await;
            },
            10,
        )
        .await;

        // Check latency percentiles
        println!("Min latency: {:?}", stats.min);
        println!("Median latency: {:?}", stats.median);
        println!("P95 latency: {:?}", stats.p95);
        println!("P99 latency: {:?}", stats.p99);
        println!("Max latency: {:?}", stats.max);

        // Assert latencies are reasonable
        assert!(stats.p95 <= Duration::from_millis(20));
    }

    /// Example: Combining multiple fixtures
    #[test]
    fn example_complex_config() {
        // Create a complex test configuration with multiple servers
        let servers = vec![
            // Basic SOCKS5 server
            test_server(1080, ProxyProtocols::Socks5),
            // High-bandwidth server
            test_server_with_bandwidth(1081, ProxyProtocols::Socks5, 100),
            // Region-specific server
            test_server_with_country(1082, ProxyProtocols::Socks5, "DE"),
        ];

        let config = test_config(servers);

        // Use the configuration for testing
        assert_eq!(config.servers.len(), 3);

        // Generate test data
        let (data, _hash) = generate_test_data(1024);

        // Create deterministic keypair
        let _keypair = test_keypair(42);

        // This demonstrates a complete test setup
        assert!(data.len() > 0);
    }

    /// Example: Creating a WireGuard VPN configuration
    #[test]
    fn example_wireguard_config() {
        // Create a WireGuard server on port 51820 (standard WireGuard port)
        let wg_server = test_server(51820, ProxyProtocols::WireGuard);

        let config = test_config(vec![wg_server]);

        // Verify the configuration
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].port, 51820);
        assert_eq!(config.servers[0].protocol, ProxyProtocols::WireGuard);
    }

    /// Example: Mixed protocol configuration
    #[test]
    fn example_mixed_protocols() {
        let servers = vec![
            // SOCKS5 proxy on standard port
            test_server(1080, ProxyProtocols::Socks5),
            // WireGuard VPN on standard port
            test_server(51820, ProxyProtocols::WireGuard),
            // High-bandwidth WireGuard server
            test_server_with_bandwidth(51821, ProxyProtocols::WireGuard, 100),
        ];

        let config = test_config(servers);

        assert_eq!(config.servers.len(), 3);
        assert_eq!(config.servers[0].protocol, ProxyProtocols::Socks5);
        assert_eq!(config.servers[1].protocol, ProxyProtocols::WireGuard);
        assert_eq!(config.servers[2].protocol, ProxyProtocols::WireGuard);
    }
}
