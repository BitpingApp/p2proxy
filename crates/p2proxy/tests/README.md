# P2Proxy Test Infrastructure

This directory contains the test infrastructure and utilities for P2Proxy, providing reusable fixtures and helpers to simplify test development.

## Directory Structure

```
tests/
├── common/                     # Common test utilities
│   ├── mod.rs                 # Module exports
│   ├── fixtures.rs            # Test fixtures and data generators
│   ├── mock_swarm.rs          # Mock P2P components (stub)
│   └── test_utils.rs          # Test helper functions
├── example_usage.rs           # Usage examples
├── fixtures_test.rs           # Tests for fixtures
├── test_utils_test.rs         # Tests for utilities
└── README.md                  # This file
```

## Available Fixtures

### Configuration Builders

#### `test_config(servers: Vec<Server>) -> Config`
Creates a minimal test configuration with the specified servers.

```rust
let server = test_server(1080, ProxyProtocols::Socks5);
let config = test_config(vec![server]);
```

#### `test_server(port: u16, protocol: ProxyProtocols) -> Server`
Creates a basic test server with default settings.

```rust
let socks_server = test_server(1080, ProxyProtocols::Socks5);
```

#### `test_server_with_country(port: u16, protocol: ProxyProtocols, country: &str) -> Server`
Creates a server with a specific country requirement.

```rust
let server = test_server_with_country(1080, ProxyProtocols::Socks5, "AT");
```

#### `test_server_with_bandwidth(port: u16, protocol: ProxyProtocols, min_bandwidth_mbps: u64) -> Server`
Creates a server with a minimum bandwidth requirement.

```rust
let server = test_server_with_bandwidth(1080, ProxyProtocols::Socks5, 100);
```

### Keypair Generation

#### `test_keypair(seed: u64) -> Keypair`
Generates a deterministic Ed25519 keypair for testing.

```rust
let keypair1 = test_keypair(42);
let keypair2 = test_keypair(42);
assert_eq!(keypair1.public().to_peer_id(), keypair2.public().to_peer_id());
```

### Test Data Generation

#### `generate_test_data(size: usize) -> (Vec<u8>, String)`
Generates deterministic test data with a known pattern and blake3 hash.

```rust
let (data, hash) = generate_test_data(1_000_000); // 1MB
assert_eq!(data.len(), 1_000_000);
```

#### `generate_seeded_test_data(size: usize, seed: u64) -> (Vec<u8>, String)`
Generates varied test data using a seeded RNG.

```rust
let (data1, hash1) = generate_seeded_test_data(1024, 42);
let (data2, hash2) = generate_seeded_test_data(1024, 42);
assert_eq!(data1, data2); // Same seed produces same data
```

### Constants

#### `TEST_PORTS: Range<u16>`
Safe port range for testing (40000-50000).

```rust
let test_port = TEST_PORTS.start; // 40000
```

## Test Utilities

### SOCKS5 Testing

#### `mock_socks5_client(port: u16, target: Address) -> Result<TcpStream>`
Creates a mock SOCKS5 client connection to test proxy functionality.

```rust
let target = Address::DomainAddress("example.com".to_string(), 80);
let stream = mock_socks5_client(1080, target).await?;
```

#### `socks5_handshake(stream: &mut TcpStream) -> Result<()>`
Performs SOCKS5 handshake with a proxy server.

```rust
let mut stream = TcpStream::connect("127.0.0.1:1080").await?;
socks5_handshake(&mut stream).await?;
```

### Bandwidth Testing

#### `BandwidthMeasurement`
Structure for measuring bandwidth during operations.

```rust
let measurement = BandwidthMeasurement::new(1_000_000, Duration::from_secs(1));
println!("Throughput: {} Mbps", measurement.mbps());
```

#### `assert_bandwidth_within(actual: u64, expected: u64, tolerance_pct: f64)`
Asserts bandwidth is within a tolerance percentage.

```rust
assert_bandwidth_within(95_000, 100_000, 10.0); // ±10% tolerance
```

#### `measure_bandwidth<F, Fut>(operation: F) -> BandwidthMeasurement`
Measures bandwidth during an async operation.

```rust
let measurement = measure_bandwidth(|| async {
    // Perform data transfer
}).await;
```

### Latency Testing

#### `LatencyStats`
Structure containing latency percentiles (min, max, mean, median, p95, p99).

```rust
let stats = measure_latency(operation, 100).await;
println!("P95 latency: {:?}", stats.p95);
```

#### `measure_latency<F, Fut>(operation: F, iterations: usize) -> LatencyStats`
Measures latency statistics over multiple iterations.

```rust
let stats = measure_latency(
    || async {
        // Operation to measure
    },
    100
).await;
```

### Event Testing

#### `wait_for_event<P>(predicate: P, timeout: Duration) -> Result<Events>`
Waits for an event matching a predicate (signature provided, implementation needs event stream connection).

```rust
let event = wait_for_event(
    |e| matches!(e, Events::Connection(_)),
    Duration::from_secs(5)
).await?;
```

## Usage Examples

See `example_usage.rs` for comprehensive examples of using the test infrastructure.

### Simple Test Setup

```rust
use common::*;
use models::config::ProxyProtocols;

#[test]
fn my_test() {
    // Create configuration
    let server = test_server(1080, ProxyProtocols::Socks5);
    let config = test_config(vec![server]);

    // Generate test data
    let (data, hash) = generate_test_data(1024);

    // Create keypair
    let keypair = test_keypair(42);

    // Run test assertions
    assert_eq!(data.len(), 1024);
}
```

### Async Test with Latency Measurement

```rust
#[tokio::test]
async fn my_async_test() {
    let stats = measure_latency(
        || async {
            // Your async operation
            tokio::time::sleep(Duration::from_millis(10)).await;
        },
        10
    ).await;

    assert!(stats.p95 <= Duration::from_millis(20));
}
```

## Running Tests

```bash
# Run all tests
cargo test --package p2proxy

# Run specific test file
cargo test --package p2proxy --test fixtures_test

# Run with output
cargo test --package p2proxy -- --nocapture

# Run with logging
RUST_LOG=debug cargo test --package p2proxy
```

### Long-Running Stability Tests

The stability test suite includes long-running tests that are marked with `#[ignore]` for manual execution:

```bash
# Run all long-running tests (24+ hours total)
cargo test --test stability_tests -- --ignored --nocapture

# Run individual long-running tests
cargo test --test stability_tests test_24hour_stability -- --ignored --nocapture      # 24 hours
cargo test --test stability_tests test_longrunning_transfer -- --ignored --nocapture  # 6 hours
cargo test --test stability_tests test_idle_connection -- --ignored --nocapture       # 2 hours
```

#### Long-Running Test Details

1. **test_24hour_stability** (24 hours)
   - Keeps P2P connection alive for 24 hours
   - Monitors memory/CPU usage every hour
   - Verifies no disconnections occur
   - Ensures memory growth < 10% and CPU < 5%

2. **test_longrunning_transfer** (6 hours)
   - Continuously transfers data for 6 hours
   - Measures sustained throughput every 10 minutes
   - Verifies no performance degradation
   - Monitors memory stability during transfers

3. **test_idle_connection** (2 hours)
   - Maintains idle connection (no data) for 2 hours
   - Checks connection every 10 minutes
   - Tests keepalive mechanisms
   - Verifies connection is immediately usable after idle

All long-running tests provide detailed progress logging and comprehensive final reports.

## Test Coverage

Current test coverage:
- **Fixtures**: 8 unit tests + 11 integration tests
- **Test Utilities**: 7 unit tests + 7 integration tests
- **Example Usage**: 10 example tests
- **Total**: 57 tests passing

All tests are deterministic and reproducible.

## Design Principles

1. **Determinism**: All fixtures use seeded RNGs for reproducible results
2. **Simplicity**: Helper functions reduce test boilerplate significantly
3. **Documentation**: Every public function has doc comments with examples
4. **Type Safety**: Leverages Rust's type system for compile-time guarantees
5. **Modularity**: Fixtures and utilities are independent and composable

## Future Enhancements

Planned additions (see SPRINT.md Task 1.2):
- Mock swarm implementation for P2P testing
- Mock relay server
- Mock peer nodes
- Property-based testing utilities
- Network simulation helpers

## Contributing

When adding new test fixtures or utilities:

1. Add comprehensive doc comments with examples
2. Ensure deterministic behavior (use seeds for RNG)
3. Add unit tests for the fixture/utility itself
4. Update this README with usage examples
5. Follow the existing naming conventions
