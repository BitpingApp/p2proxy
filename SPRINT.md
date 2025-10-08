# Sprint: P2Proxy Test Infrastructure

**Sprint Duration**: 2-3 weeks
**Sprint Goal**: Establish comprehensive test infrastructure and test coverage for P2Proxy's core functionality including connections, disconnections, throughput, jitter, and network stability.

## Sprint Overview

This sprint focuses on building a robust test harness from scratch to ensure P2Proxy's reliability and performance. Currently, the codebase has no existing test infrastructure. We will implement unit tests, integration tests, and performance benchmarks covering all critical aspects of the P2P proxy system.

## Sprint Goals

1. ✅ Create reusable test harness infrastructure
2. ✅ Implement connection lifecycle tests
3. ✅ Implement disconnection and failure scenario tests
4. ✅ Implement throughput and performance tests
5. ✅ Implement jitter and latency tests
6. ✅ Implement network stability and long-running tests

---

## Task Breakdown

### Phase 1: Test Harness Infrastructure

#### Task 1.1: Set Up Test Framework Foundation
**Estimate**: 2-3 days
**Priority**: P0 (Blocker)

Create the foundational test infrastructure:

- Add test dependencies to `Cargo.toml`:
  ```toml
  [dev-dependencies]
  tokio-test = "0.4"
  criterion = { version = "0.5", features = ["async_tokio"] }
  proptest = "1.4"
  mockall = "0.12"
  wiremock = "0.6"
  tempfile = "3.8"
  serial_test = "3.0"
  ```

- Create test directory structure:
  ```
  crates/p2proxy/tests/
  ├── common/
  │   ├── mod.rs
  │   ├── fixtures.rs
  │   ├── mock_swarm.rs
  │   └── test_utils.rs
  ├── integration/
  │   ├── connection_tests.rs
  │   ├── disconnection_tests.rs
  │   ├── throughput_tests.rs
  │   ├── jitter_tests.rs
  │   └── stability_tests.rs
  └── benches/
      ├── throughput_bench.rs
      └── latency_bench.rs
  ```

**Acceptance Criteria**:
- Test framework compiles without errors
- Can run `cargo test` successfully
- Benchmarks can run with `cargo bench`

---

#### Task 1.2: Create Mock P2P Network Components
**Estimate**: 3-4 days
**Priority**: P0 (Blocker)

Build mock components for testing P2P functionality in isolation:

**Files to create**:
- `tests/common/mock_swarm.rs`: Mock libp2p Swarm with configurable behavior
- `tests/common/mock_relay.rs`: Mock relay server for testing relay connections
- `tests/common/mock_peer.rs`: Mock peer nodes with controllable responses

**Mock Swarm Features**:
- Simulate connection establishment (successful/failed)
- Simulate peer discovery
- Inject network events (connected, disconnected, identify)
- Control timing and delays
- Simulate relay vs direct connections

**Example structure**:
```rust
pub struct MockSwarm {
    local_peer_id: PeerId,
    connected_peers: Vec<PeerId>,
    event_queue: VecDeque<SwarmEvent>,
    config: MockSwarmConfig,
}

impl MockSwarm {
    pub fn new(config: MockSwarmConfig) -> Self { ... }
    pub fn connect_to_peer(&mut self, peer: PeerId) -> Result<()> { ... }
    pub fn simulate_disconnect(&mut self, peer: PeerId) { ... }
    pub fn inject_event(&mut self, event: SwarmEvent) { ... }
}
```

**Acceptance Criteria**:
- Can create mock swarm with custom configuration
- Can simulate connection/disconnection events
- Can inject custom events for testing
- Mock components work with existing libp2p types

---

#### Task 1.3: Create Test Fixtures and Utilities
**Estimate**: 2 days
**Priority**: P0 (Blocker)

Build reusable test fixtures and helper functions:

**Files to create**:
- `tests/common/fixtures.rs`: Common test data and configurations
- `tests/common/test_utils.rs`: Helper functions for assertions and setup

**Fixtures needed**:
- Sample `Config.yaml` configurations for different test scenarios
- Mock authentication tokens
- Test keypairs (deterministic for reproducibility)
- Sample SOCKS5 requests
- Test target addresses (domains and IPs)

**Utility functions**:
```rust
// Create test configuration with custom settings
pub fn test_config(servers: Vec<Server>) -> Config { ... }

// Create deterministic test keypair
pub fn test_keypair(seed: u64) -> Keypair { ... }

// Wait for connection with timeout
pub async fn wait_for_connection(
    swarm: &Swarm,
    timeout: Duration
) -> Result<PeerId> { ... }

// Assert bandwidth metrics
pub fn assert_bandwidth_reported(
    upload: u64,
    download: u64,
    tolerance: f64
) { ... }

// Create mock SOCKS5 connection
pub async fn mock_socks5_client(
    port: u16,
    target: Address
) -> Result<TcpStream> { ... }
```

**Acceptance Criteria**:
- All fixtures are deterministic and reproducible
- Utility functions are well-documented
- Helper functions reduce test boilerplate by >50%

---

### Phase 2: Connection Tests

#### Task 2.1: P2P Connection Establishment Tests
**Estimate**: 3 days
**Priority**: P0

Test successful connection scenarios:

**Test file**: `tests/integration/connection_tests.rs`

**Test scenarios**:
1. **Direct connection establishment**
   - Two peers connect directly without relay
   - Verify identify protocol exchange
   - Verify connection event emitted
   - Verify peer added to swarm

2. **Relay-mediated connection**
   - Connect through relay server
   - Verify relay reservation
   - Verify connection event emitted
   - Test DCUtR (hole punching) upgrade

3. **Multiple peer connections**
   - Connect to 5+ peers simultaneously
   - Verify all connections established
   - Verify correct peer tracking
   - Test connection limits if configured

4. **Reconnection after network change**
   - Establish connection
   - Simulate IP address change
   - Verify automatic reconnection
   - Verify session continuity

**Key assertions**:
```rust
#[tokio::test]
async fn test_direct_connection() {
    let swarm1 = create_test_swarm().await;
    let swarm2 = create_test_swarm().await;

    let peer2_id = swarm2.local_peer_id();
    swarm1.dial(peer2_addr).await.unwrap();

    let event = wait_for_event(&swarm1, Duration::from_secs(5)).await;
    assert_matches!(event, SwarmEvent::Connected(peer) if peer == peer2_id);
}
```

**Acceptance Criteria**:
- All connection scenarios pass
- Tests complete in <30 seconds
- No flaky tests (100% pass rate over 10 runs)

---

#### Task 2.2: SOCKS5 Proxy Connection Tests
**Estimate**: 2-3 days
**Priority**: P0

Test SOCKS5 proxy functionality:

**Test scenarios**:
1. **SOCKS5 handshake**
   - Send valid SOCKS5 greeting
   - Verify authentication method selection
   - Test unsupported auth methods

2. **Connection request handling**
   - IPv4 target address
   - IPv6 target address
   - Domain name target address
   - Invalid address format

3. **Proxy session lifecycle**
   - Establish proxy session
   - Send/receive data
   - Graceful close
   - Verify session events emitted

4. **Multiple concurrent sessions**
   - 100+ concurrent SOCKS5 connections
   - Verify session isolation
   - Verify correct bandwidth accounting
   - Test connection pooling

**Example test**:
```rust
#[tokio::test]
async fn test_socks5_handshake() {
    let proxy = start_test_proxy(1080).await;
    let mut client = TcpStream::connect("127.0.0.1:1080").await.unwrap();

    // Send SOCKS5 greeting
    client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();

    // Read response
    let mut buf = [0u8; 2];
    client.read_exact(&mut buf).await.unwrap();

    assert_eq!(buf[0], 0x05); // SOCKS version
    assert_eq!(buf[1], 0x00); // No authentication
}
```

**Acceptance Criteria**:
- All SOCKS5 protocol states tested
- Error conditions properly handled
- Concurrent sessions work without interference

---

#### Task 2.3: RPC Connection Tests
**Estimate**: 1-2 days
**Priority**: P1

Test RPC communication between daemon and UI:

**Test scenarios**:
1. **RPC connection establishment**
   - UI connects to daemon on port 9876
   - Verify remoc handshake
   - Test connection timeout

2. **Method invocation**
   - `get_server_states()` returns correct data
   - `get_connection_status()` reflects actual status
   - `get_stats()` returns accurate metrics
   - `watch_events()` streams events correctly

3. **Multiple clients**
   - Multiple UIs connect simultaneously
   - Verify all receive events
   - Test client disconnection handling

**Acceptance Criteria**:
- All RPC methods tested
- Event streaming works correctly
- Client disconnections don't crash daemon

---

### Phase 3: Disconnection Tests

#### Task 3.1: Graceful Disconnection Tests
**Estimate**: 2 days
**Priority**: P0

Test clean shutdown scenarios:

**Test file**: `tests/integration/disconnection_tests.rs`

**Test scenarios**:
1. **Graceful peer disconnection**
   - Peer sends disconnect
   - Verify cleanup of sessions
   - Verify ConnectionEvents::Disconnected emitted
   - Verify metrics updated

2. **Graceful shutdown during active sessions**
   - Multiple active SOCKS5 sessions
   - Initiate shutdown
   - Verify sessions complete or gracefully terminate
   - Verify no data loss

3. **Client disconnect handling**
   - SOCKS5 client closes connection
   - Verify peer stream closed
   - Verify SessionEvents::End emitted
   - Verify bandwidth report generated

**Acceptance Criteria**:
- No panics during shutdown
- All resources properly cleaned up
- Events correctly emitted

---

#### Task 3.2: Network Failure Tests
**Estimate**: 2-3 days
**Priority**: P0

Test abrupt disconnection scenarios:

**Test scenarios**:
1. **Sudden peer unavailability**
   - Kill peer process abruptly
   - Verify timeout detection
   - Verify error handling
   - Test reconnection logic

2. **Network partition**
   - Simulate network partition between peers
   - Verify timeout and detection
   - Verify proper state cleanup
   - Test healing after partition resolves

3. **Relay failure**
   - Kill relay server during active connection
   - Verify fallback to direct connection (if possible)
   - Verify error reporting
   - Test recovery mechanisms

4. **Partial data transfer failure**
   - Disconnect during data transfer
   - Verify both sides detect failure
   - Verify no hung connections
   - Test cleanup of partial data

**Tools needed**:
- Network simulation (toxiproxy or similar)
- Process kill utilities
- Network namespace isolation for Linux tests

**Example test**:
```rust
#[tokio::test]
async fn test_peer_sudden_disconnect() {
    let (swarm1, swarm2) = connect_test_peers().await;
    let session = create_socks_session(&swarm1, &swarm2).await;

    // Kill peer2 abruptly
    drop(swarm2);

    // Wait for timeout detection
    let event = wait_for_event(&swarm1, Duration::from_secs(10)).await;
    assert_matches!(event, Events::Connection(ConnectionEvents::Disconnected));

    // Verify session cleaned up
    assert_eq!(swarm1.active_sessions(), 0);
}
```

**Acceptance Criteria**:
- All failure modes handled gracefully
- No memory leaks or hung connections
- Recovery mechanisms work correctly

---

#### Task 3.3: Authentication Failure Tests
**Estimate**: 1 day
**Priority**: P1

Test authentication-related disconnections:

**Test scenarios**:
1. **Invalid API key**
   - Start daemon with invalid Bitping API key
   - Verify connection failure
   - Verify error message clarity

2. **Expired token**
   - Simulate token expiration during session
   - Verify re-authentication attempt
   - Test session continuity (if supported)

3. **gRPC service unavailable**
   - Mock gRPC service returns errors
   - Verify retry logic
   - Test backoff behavior

**Acceptance Criteria**:
- Clear error messages for auth failures
- No infinite retry loops
- Proper backoff implemented

---

### Phase 4: Throughput Tests

#### Task 4.1: Bandwidth Measurement Tests
**Estimate**: 2-3 days
**Priority**: P0

Test accurate bandwidth reporting:

**Test file**: `tests/integration/throughput_tests.rs`

**Test scenarios**:
1. **Accurate byte counting**
   - Transfer known data size (1MB, 10MB, 100MB)
   - Verify upload bytes match
   - Verify download bytes match
   - Tolerance: ±1%

2. **Bandwidth metrics accuracy**
   - Transfer data at controlled rate
   - Verify Prometheus metrics match actual
   - Verify BandwidthEvents accuracy
   - Test metrics aggregation

3. **Hash verification**
   - Transfer data with known hash
   - Verify incoming_hash and outgoing_hash correct
   - Test hash mismatch detection

**Test data generators**:
```rust
// Generate test data with known size and hash
pub fn generate_test_data(size: usize) -> (Vec<u8>, String) {
    let data = vec![0xAB; size];
    let hash = blake3::hash(&data).to_hex().to_string();
    (data, hash)
}
```

**Acceptance Criteria**:
- Byte counts accurate within 1%
- Hashes verified correctly
- Metrics match actual transfers

---

#### Task 4.2: Maximum Throughput Tests
**Estimate**: 2 days
**Priority**: P1

Test performance limits:

**Test scenarios**:
1. **Single session maximum throughput**
   - Transfer maximum data through single session
   - Measure MB/s achieved
   - Compare against baseline (direct TCP)
   - Target: >80% of direct TCP throughput

2. **Concurrent session throughput**
   - 10, 50, 100 concurrent sessions
   - Measure aggregate throughput
   - Verify fair bandwidth distribution
   - Test queue saturation handling

3. **Large file transfer**
   - Transfer 1GB+ file
   - Measure sustained throughput
   - Verify no degradation over time
   - Check memory usage stability

**Benchmark integration**:
```rust
fn benchmark_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");

    group.bench_function("single_session_1mb", |b| {
        b.to_async(Runtime::new().unwrap()).iter(|| async {
            let (data, _) = generate_test_data(1_000_000);
            transfer_via_proxy(data).await
        });
    });
}
```

**Acceptance Criteria**:
- Achieves >80% of baseline TCP throughput
- Handles 100+ concurrent sessions
- Memory usage remains stable

---

#### Task 4.3: Bandwidth Limit Compliance Tests
**Estimate**: 1-2 days
**Priority**: P1

Test min_bandwidth configuration enforcement:

**Test scenarios**:
1. **Peer selection based on bandwidth**
   - Configure min_bandwidth: 70Mbps
   - Mock peers with varying bandwidth capabilities
   - Verify only peers meeting requirement selected
   - Test fallback if no peers available

2. **Bandwidth requirement updates**
   - Change min_bandwidth during runtime
   - Verify new peer selection criteria
   - Test migration of existing sessions

**Acceptance Criteria**:
- Only compliant peers selected
- Configuration changes applied correctly

---

### Phase 5: Jitter Tests

#### Task 5.1: Latency Measurement Tests
**Estimate**: 2-3 days
**Priority**: P1

Test latency characteristics:

**Test file**: `tests/integration/jitter_tests.rs`

**Test scenarios**:
1. **Round-trip time measurement**
   - Measure RTT for P2P connections
   - Compare direct vs relay connections
   - Establish baseline metrics
   - Target: <100ms for direct, <250ms for relay

2. **Request-response latency**
   - Measure SOCKS5 handshake latency
   - Measure connection establishment latency
   - Measure first-byte latency
   - Create latency distribution histogram

**Measurement infrastructure**:
```rust
pub struct LatencyMeasurement {
    pub min: Duration,
    pub max: Duration,
    pub avg: Duration,
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
}

pub async fn measure_latency<F, Fut>(
    iterations: usize,
    operation: F
) -> LatencyMeasurement
where
    F: Fn() -> Fut,
    Fut: Future<Output = ()>,
{
    // Measure operation latency multiple times
    // Calculate percentiles
}
```

**Acceptance Criteria**:
- Latency measurements accurate within 5ms
- Percentiles calculated correctly
- Baseline metrics documented

---

#### Task 5.2: Jitter Analysis Tests
**Estimate**: 2 days
**Priority**: P1

Test timing consistency:

**Test scenarios**:
1. **Packet timing variance**
   - Send packets at regular intervals (10ms, 50ms, 100ms)
   - Measure arrival time variance
   - Calculate jitter (variance from expected timing)
   - Target: <10ms jitter for stable connections

2. **Jitter under load**
   - Measure jitter with varying background traffic
   - Test 0%, 50%, 100% capacity load
   - Verify jitter increases predictably
   - Identify jitter thresholds

3. **Clock synchronization effects**
   - Test on systems with clock skew
   - Verify monotonic time usage
   - Test across time zone boundaries

**Jitter calculation**:
```rust
pub fn calculate_jitter(
    send_times: &[Instant],
    recv_times: &[Instant]
) -> Duration {
    // Calculate variance in packet inter-arrival time
    // RFC 3550 jitter calculation
}
```

**Acceptance Criteria**:
- Jitter <10ms for stable connections
- Jitter calculation matches RFC 3550
- Load impact quantified

---

#### Task 5.3: Latency Benchmarks
**Estimate**: 1 day
**Priority**: P2

Create performance benchmarks:

**Benchmark file**: `tests/benches/latency_bench.rs`

**Benchmarks**:
1. Connection establishment latency
2. SOCKS5 handshake latency
3. First-byte latency
4. Small message round-trip (<1KB)
5. Large message round-trip (1MB+)

**Acceptance Criteria**:
- Benchmarks run in CI
- Results tracked over time
- Regression detection configured

---

### Phase 6: Network Stability Tests

#### Task 6.1: Long-Running Session Tests
**Estimate**: 2-3 days
**Priority**: P0

Test stability over extended periods:

**Test file**: `tests/integration/stability_tests.rs`

**Test scenarios**:
1. **24-hour connection test**
   - Establish P2P connection
   - Keep alive for 24 hours
   - Verify no disconnections
   - Check memory usage remains stable
   - Monitor CPU usage (should be <5%)

2. **Long-running data transfer**
   - Transfer data continuously for 6+ hours
   - Measure throughput stability
   - Verify no degradation
   - Check for memory leaks

3. **Idle connection stability**
   - Establish connection
   - No data transfer for 2+ hours
   - Verify connection maintained
   - Test keepalive mechanisms

**Test infrastructure**:
```rust
#[tokio::test]
#[ignore] // Long-running test, run separately
async fn test_24hour_stability() {
    let start_mem = get_memory_usage();
    let (swarm1, swarm2) = connect_test_peers().await;

    for hour in 0..24 {
        tokio::time::sleep(Duration::from_secs(3600)).await;

        // Verify still connected
        assert!(swarm1.is_connected(&swarm2.local_peer_id()));

        // Check memory hasn't grown >10%
        let current_mem = get_memory_usage();
        assert!(current_mem < start_mem * 1.1);

        tracing::info!("Hour {}: Still stable", hour + 1);
    }
}
```

**Acceptance Criteria**:
- 24-hour test passes without disconnection
- Memory growth <10% over 24 hours
- CPU usage <5% when idle

---

#### Task 6.2: Reconnection Logic Tests
**Estimate**: 2 days
**Priority**: P0

Test automatic recovery:

**Test scenarios**:
1. **Exponential backoff**
   - Simulate connection failure
   - Verify retry intervals: 1s, 2s, 4s, 8s, 16s, 30s (max)
   - Test max retry limit (if configured)

2. **Session restoration**
   - Disconnect during active session
   - Reconnect to same peer
   - Test session continuity (if supported)
   - Verify state consistency

3. **Peer rotation**
   - Primary peer becomes unavailable
   - Verify automatic failover to alternative peer
   - Test with country/bandwidth requirements
   - Verify minimal disruption

**Example test**:
```rust
#[tokio::test]
async fn test_exponential_backoff() {
    let proxy = create_test_proxy().await;
    let mut retry_times = Vec::new();

    // Simulate 5 connection failures
    for i in 0..5 {
        let start = Instant::now();
        proxy.attempt_connection().await; // This will fail
        retry_times.push(start.elapsed());
    }

    // Verify exponential backoff
    assert_approx_eq!(retry_times[0].as_secs(), 1, 0.5);
    assert_approx_eq!(retry_times[1].as_secs(), 2, 0.5);
    assert_approx_eq!(retry_times[2].as_secs(), 4, 0.5);
    assert_approx_eq!(retry_times[3].as_secs(), 8, 0.5);
    assert_approx_eq!(retry_times[4].as_secs(), 16, 0.5);
}
```

**Acceptance Criteria**:
- Backoff intervals correct
- Reconnection succeeds after transient failure
- No infinite retry loops

---

#### Task 6.3: Stress and Chaos Tests
**Estimate**: 2-3 days
**Priority**: P1

Test system under extreme conditions:

**Test scenarios**:
1. **Connection churn**
   - Rapidly connect/disconnect 100+ peers
   - Measure impact on stability
   - Verify no resource exhaustion
   - Test recovery time

2. **High session turnover**
   - Create 1000+ SOCKS5 sessions/minute
   - Very short session duration (1-10 seconds)
   - Verify no session leaks
   - Test cleanup performance

3. **Resource exhaustion**
   - Max out file descriptor limit
   - Verify graceful degradation
   - Test error handling
   - Verify recovery when resources available

4. **Network chaos**
   - Random packet loss (5%, 10%, 20%)
   - Random latency injection (10ms-500ms)
   - Random bandwidth throttling
   - Verify protocol resilience

**Chaos testing tools**:
- toxiproxy for network simulation
- chaos-mesh for Kubernetes environments
- Custom packet loss injection

**Acceptance Criteria**:
- System remains stable under stress
- Graceful degradation when resources exhausted
- Recovery after chaos resolves

---

## Test Execution Strategy

### Unit Tests
```bash
# Run all unit tests
cargo test --lib

# Run with output
cargo test --lib -- --nocapture

# Run specific test
cargo test --lib test_name
```

### Integration Tests
```bash
# Run all integration tests
cargo test --test '*'

# Run specific test file
cargo test --test connection_tests

# Run with logging
RUST_LOG=debug cargo test --test connection_tests
```

### Long-Running Tests
```bash
# Run stability tests (marked with #[ignore])
cargo test --test stability_tests -- --ignored --nocapture
```

### Benchmarks
```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench throughput

# Generate flamegraph
cargo bench --bench throughput_bench -- --profile-time=5
```

### CI Integration
Add to `.github/workflows/test.yml`:
```yaml
name: Tests
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - name: Run tests
        run: cargo test --all
      - name: Run benchmarks
        run: cargo bench --no-run
```

---

## Success Criteria

### Overall Sprint Success
- [ ] All P0 tasks completed
- [ ] Test coverage >70% for core modules
- [ ] All tests pass consistently (no flakes)
- [ ] CI integration complete
- [ ] Documentation for running tests

### Connection Tests
- [ ] All connection scenarios tested
- [ ] Direct and relay connections work
- [ ] SOCKS5 proxy functionality verified
- [ ] RPC communication tested

### Disconnection Tests
- [ ] Graceful shutdown tested
- [ ] Network failures handled correctly
- [ ] No resource leaks
- [ ] Authentication failures handled

### Performance Tests
- [ ] Throughput >80% of TCP baseline
- [ ] 100+ concurrent sessions supported
- [ ] Bandwidth accounting accurate (±1%)
- [ ] Latency <100ms direct, <250ms relay

### Stability Tests
- [ ] 24-hour test passes
- [ ] Memory stable (<10% growth)
- [ ] Reconnection logic works
- [ ] Stress tests pass

---

## Dependencies and Tools

### Required Crates
```toml
[dev-dependencies]
tokio-test = "0.4"           # Testing utilities for async
criterion = "0.5"            # Benchmarking framework
proptest = "1.4"             # Property-based testing
mockall = "0.12"             # Mocking framework
serial_test = "3.0"          # Serialize test execution
tempfile = "3.8"             # Temporary files/dirs
wiremock = "0.6"             # HTTP mocking (for gRPC)
assert_matches = "1.5"       # Pattern matching assertions
```

### External Tools
- **toxiproxy**: Network condition simulation
- **iperf3**: Baseline throughput measurement
- **flamegraph**: Performance profiling
- **valgrind**: Memory leak detection (Linux)

### Test Infrastructure
- Mock Bitping gRPC service
- Mock relay servers
- Test keypair generation
- Network simulation tools

---

## Notes and Considerations

1. **Deterministic Testing**: Use seeded RNG for reproducibility
2. **Timeouts**: All async tests should have reasonable timeouts
3. **Cleanup**: Use RAII patterns and `Drop` for resource cleanup
4. **Logging**: Comprehensive logging in tests for debugging
5. **Isolation**: Tests should not interfere with each other
6. **Flakiness**: Run each test 10+ times to detect flakiness
7. **Performance**: Keep test suite runtime <5 minutes for fast feedback

---

## Future Improvements (Post-Sprint)

- Property-based testing for protocol correctness
- Fuzzing for security testing
- Load testing in production-like environment
- Multi-node distributed testing
- Automated performance regression detection
- Integration with external monitoring (Grafana/Prometheus)
