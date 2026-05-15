// Integration tests for test fixtures
mod common;

#[cfg(test)]
mod tests {
    use crate::common::fixtures::*;
    use models::config::ProxyProtocols;

    #[test]
    fn test_config_creation() {
        let server = test_server(1080, ProxyProtocols::Socks5);
        let config = test_config(vec![server]);

        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.port, 45445);
        assert_eq!(config.bitping_api_key, "test_api_key");
    }

    #[test]
    fn test_server_creation() {
        let server = test_server(1080, ProxyProtocols::Socks5);

        assert_eq!(server.port, 1080);
        assert_eq!(server.protocol, ProxyProtocols::Socks5);
        assert_eq!(server.peer_options.destination_peer, None);
        assert_eq!(server.peer_options.country, None);
    }

    #[test]
    fn test_keypair_deterministic() {
        let kp1 = test_keypair(42);
        let kp2 = test_keypair(42);
        let kp3 = test_keypair(99);

        // Same seed produces same peer ID
        assert_eq!(kp1.public().to_peer_id(), kp2.public().to_peer_id());

        // Different seed produces different peer ID
        assert_ne!(kp1.public().to_peer_id(), kp3.public().to_peer_id());
    }

    #[test]
    fn test_generate_test_data() {
        let (data, hash) = generate_test_data(1024);

        assert_eq!(data.len(), 1024);
        assert_eq!(hash.len(), 64); // blake3 hash is 32 bytes = 64 hex chars

        // Verify all bytes are 0xAB
        assert!(data.iter().all(|&b| b == 0xAB));

        // Verify hash is correct
        let computed_hash = blake3::hash(&data).to_hex().to_string();
        assert_eq!(hash, computed_hash);
    }

    #[test]
    fn test_generate_seeded_test_data_deterministic() {
        let (data1, hash1) = generate_seeded_test_data(1024, 42);
        let (data2, hash2) = generate_seeded_test_data(1024, 42);

        assert_eq!(data1, data2);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_generate_seeded_test_data_different_seeds() {
        let (data1, hash1) = generate_seeded_test_data(1024, 42);
        let (data2, hash2) = generate_seeded_test_data(1024, 99);

        assert_ne!(data1, data2);
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn fixture_server_with_country() {
        let server = test_server_with_country(1080, ProxyProtocols::Socks5, "AT");

        assert_eq!(server.peer_options.country, Some("AT".to_string()));
    }

    #[test]
    fn fixture_server_with_bandwidth() {
        let server = test_server_with_bandwidth(1080, ProxyProtocols::Socks5, 100);

        assert_eq!(server.peer_options.min_bandwidth.as_mbps(), 100);
    }

    #[test]
    fn test_test_ports_range() {
        assert_eq!(TEST_PORTS.start, 40000);
        assert_eq!(TEST_PORTS.end, 50000);
        assert!(TEST_PORTS.contains(&45000));
    }
}
