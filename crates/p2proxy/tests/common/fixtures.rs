//! Test fixtures for P2Proxy
//!
//! This module provides reusable test data and configurations including:
//! - Sample Config.yaml configurations for different test scenarios
//! - Mock authentication tokens
//! - Test keypairs (deterministic for reproducibility)
//! - Sample SOCKS5 requests
//! - Test target addresses (domains and IPs)

use human_bandwidth::re::bandwidth::Bandwidth;
use libp2p::identity::Keypair;
use models::config::{Config, ProxyProtocols, Server, ServerPeerOptions};
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::borrow::Cow;
use std::ops::Range;

/// Range of ports safe for testing (40000-50000)
///
/// This range is chosen to avoid conflicts with common services and system ports.
/// Tests can use any port within this range without fear of collision with production services.
///
/// # Example
///
/// ```no_run
/// use common::fixtures::TEST_PORTS;
///
/// let test_port = TEST_PORTS.start;
/// ```
pub const TEST_PORTS: Range<u16> = 40000..50000;

/// Creates a test configuration with the specified servers
///
/// This helper creates a minimal valid `Config` instance suitable for testing.
/// The configuration uses:
/// - Port 45445 for P2P communication
/// - A test API key "test_api_key"
///
/// # Arguments
///
/// * `servers` - Vector of `Server` instances to include in the configuration
///
/// # Returns
///
/// A `Config` instance ready for use in tests
///
/// # Example
///
/// ```no_run
/// use common::fixtures::{test_config, test_server};
/// use models::config::ProxyProtocols;
///
/// let socks_server = test_server(1080, ProxyProtocols::Socks5);
/// let config = test_config(vec![socks_server]);
///
/// assert_eq!(config.servers.len(), 1);
/// assert_eq!(config.port, 45445);
/// ```
pub fn test_config(servers: Vec<Server>) -> Config {
    Config {
        servers,
        port: 45445,
        bitping_api_key: Cow::Borrowed("test_api_key"),
    }
}

/// Creates a test server with the specified port and protocol
///
/// This helper creates a `Server` instance with sensible defaults for testing:
/// - No specific destination peer
/// - No country requirement
/// - Minimum bandwidth of 50 Mbps (default)
///
/// # Arguments
///
/// * `port` - The port number the server should listen on
/// * `protocol` - The proxy protocol (e.g., `ProxyProtocols::Socks5`)
///
/// # Returns
///
/// A `Server` instance configured for testing
///
/// # Example
///
/// ```no_run
/// use common::fixtures::test_server;
/// use models::config::ProxyProtocols;
///
/// let socks_server = test_server(1080, ProxyProtocols::Socks5);
/// assert_eq!(socks_server.port, 1080);
/// ```
pub fn test_server(port: u16, protocol: ProxyProtocols) -> Server {
    Server {
        protocol,
        port,
        peer_options: ServerPeerOptions {
            destination_peer: None,
            country: None,
            min_bandwidth: Bandwidth::from_mbps(50),
        },
    }
}

/// Creates a test server with a specific country requirement
///
/// # Arguments
///
/// * `port` - The port number the server should listen on
/// * `protocol` - The proxy protocol
/// * `country` - Two-letter country code (e.g., "AT", "US")
///
/// # Example
///
/// ```no_run
/// use common::fixtures::test_server_with_country;
/// use models::config::ProxyProtocols;
///
/// let server = test_server_with_country(1080, ProxyProtocols::Socks5, "AT");
/// assert_eq!(server.peer_options.country, Some("AT".to_string()));
/// ```
pub fn test_server_with_country(port: u16, protocol: ProxyProtocols, country: &str) -> Server {
    Server {
        protocol,
        port,
        peer_options: ServerPeerOptions {
            destination_peer: None,
            country: Some(country.to_string()),
            min_bandwidth: Bandwidth::from_mbps(50),
        },
    }
}

/// Creates a test server with a specific minimum bandwidth requirement
///
/// # Arguments
///
/// * `port` - The port number the server should listen on
/// * `protocol` - The proxy protocol
/// * `min_bandwidth_mbps` - Minimum bandwidth in Mbps
///
/// # Example
///
/// ```no_run
/// use common::fixtures::test_server_with_bandwidth;
/// use models::config::ProxyProtocols;
///
/// let server = test_server_with_bandwidth(1080, ProxyProtocols::Socks5, 100);
/// ```
pub fn test_server_with_bandwidth(port: u16, protocol: ProxyProtocols, min_bandwidth_mbps: u64) -> Server {
    Server {
        protocol,
        port,
        peer_options: ServerPeerOptions {
            destination_peer: None,
            country: None,
            min_bandwidth: Bandwidth::from_mbps(min_bandwidth_mbps),
        },
    }
}

/// Generates a deterministic keypair for testing
///
/// This function creates a keypair using a seeded random number generator,
/// ensuring that the same seed always produces the same keypair. This is
/// crucial for test reproducibility.
///
/// # Arguments
///
/// * `seed` - A seed value for the random number generator (0-u64::MAX)
///
/// # Returns
///
/// A deterministic libp2p `Keypair`
///
/// # Example
///
/// ```no_run
/// use common::fixtures::test_keypair;
///
/// // These will always generate the same keypair
/// let keypair1 = test_keypair(42);
/// let keypair2 = test_keypair(42);
///
/// assert_eq!(
///     keypair1.public().to_peer_id(),
///     keypair2.public().to_peer_id()
/// );
/// ```
pub fn test_keypair(seed: u64) -> Keypair {
    use rand::RngCore;

    let mut rng = StdRng::seed_from_u64(seed);

    // Generate deterministic Ed25519 keypair
    // Ed25519 private keys are 32 bytes
    let mut secret_bytes = [0u8; 32];
    rng.fill_bytes(&mut secret_bytes);

    // Create keypair from the deterministic bytes
    Keypair::ed25519_from_bytes(secret_bytes)
        .expect("Failed to create Ed25519 keypair from bytes")
}

/// Generates test data with a known size and blake3 hash
///
/// This function creates deterministic test data filled with a repeating pattern.
/// The data is hashed using blake3, and both the data and hash are returned.
///
/// # Arguments
///
/// * `size` - Size of the data to generate in bytes
///
/// # Returns
///
/// A tuple of `(Vec<u8>, String)` where:
/// - The `Vec<u8>` is the generated test data
/// - The `String` is the blake3 hash in hexadecimal format
///
/// # Example
///
/// ```no_run
/// use common::fixtures::generate_test_data;
///
/// let (data, hash) = generate_test_data(1024);
///
/// assert_eq!(data.len(), 1024);
/// assert_eq!(hash.len(), 64); // blake3 hash is 32 bytes = 64 hex chars
///
/// // Verify the hash matches
/// let computed_hash = blake3::hash(&data).to_hex().to_string();
/// assert_eq!(hash, computed_hash);
/// ```
pub fn generate_test_data(size: usize) -> (Vec<u8>, String) {
    // Generate deterministic data with a simple pattern
    // Using 0xAB as a recognizable test pattern
    let data = vec![0xAB; size];

    // Compute blake3 hash
    let hash = blake3::hash(&data);
    let hash_string = hash.to_hex().to_string();

    (data, hash_string)
}

/// Generates deterministic test data with varying content based on a seed
///
/// Unlike `generate_test_data`, this function creates data with varying content
/// using a seeded random number generator. This is useful for testing scenarios
/// where you need different data patterns.
///
/// # Arguments
///
/// * `size` - Size of the data to generate in bytes
/// * `seed` - Seed value for the random number generator
///
/// # Returns
///
/// A tuple of `(Vec<u8>, String)` where:
/// - The `Vec<u8>` is the generated test data
/// - The `String` is the blake3 hash in hexadecimal format
///
/// # Example
///
/// ```no_run
/// use common::fixtures::generate_seeded_test_data;
///
/// let (data1, hash1) = generate_seeded_test_data(1024, 42);
/// let (data2, hash2) = generate_seeded_test_data(1024, 42);
/// let (data3, hash3) = generate_seeded_test_data(1024, 99);
///
/// // Same seed produces same data
/// assert_eq!(data1, data2);
/// assert_eq!(hash1, hash2);
///
/// // Different seed produces different data
/// assert_ne!(data1, data3);
/// assert_ne!(hash1, hash3);
/// ```
pub fn generate_seeded_test_data(size: usize, seed: u64) -> (Vec<u8>, String) {
    use rand::RngCore;

    let mut rng = StdRng::seed_from_u64(seed);
    let mut data = vec![0u8; size];
    rng.fill_bytes(&mut data);

    let hash = blake3::hash(&data);
    let hash_string = hash.to_hex().to_string();

    (data, hash_string)
}

#[cfg(test)]
mod tests {
    use super::*;

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

}
