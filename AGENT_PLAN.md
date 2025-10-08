# Agent Execution Plan: P2Proxy Test Infrastructure

This document provides a detailed plan for implementing the P2Proxy test infrastructure using Claude agents. The work is organized into waves with clear dependencies and parallel execution opportunities.

## Table of Contents
- [Overview](#overview)
- [Wave Structure](#wave-structure)
- [Agent Specifications](#agent-specifications)
- [Agent Prompts](#agent-prompts)
- [Execution Strategy](#execution-strategy)

---

## Overview

**Total Agents**: 11
**Total Estimated Time**: 2-3 weeks
**Parallelization Factor**: Up to 5 agents running simultaneously

The sprint implementation is divided into 4 waves:
1. **Foundation** (Agents 1-3): Core infrastructure - Run in parallel
2. **Core Tests** (Agents 4-8): Primary test suites - Run in parallel after Wave 1
3. **Advanced Tests** (Agents 9-10): Long-running and stress tests - Run after Wave 2
4. **Integration** (Agent 11): CI/CD and polish - Run after Wave 3

---

## Wave Structure

### Wave 1: Foundation (Days 1-3)
**Can Start**: Immediately
**Parallel Execution**: All 3 agents run simultaneously
**Blocking**: Wave 2 depends on completion

```
Agent 1 (Framework) ────┐
Agent 2 (Mocks)     ────┼──→ Wave 2
Agent 3 (Utilities) ────┘
```

### Wave 2: Core Test Implementation (Days 4-10)
**Can Start**: After Wave 1 completes
**Parallel Execution**: All 5 agents run simultaneously
**Blocking**: Wave 3 depends on completion

```
Agent 4 (Connections)    ────┐
Agent 5 (Disconnections) ────┤
Agent 6 (Throughput)     ────┼──→ Wave 3
Agent 7 (Jitter)         ────┤
Agent 8 (Stability Core) ────┘
```

### Wave 3: Advanced Testing (Days 11-16)
**Can Start**: After Wave 2 completes
**Parallel Execution**: Both agents run simultaneously
**Blocking**: Wave 4 depends on completion

```
Agent 9 (Long-running)  ────┬──→ Wave 4
Agent 10 (Chaos)        ────┘
```

### Wave 4: Integration (Days 17-18)
**Can Start**: After Wave 3 completes
**Parallel Execution**: Single agent
**Blocking**: None - final wave

```
Agent 11 (CI/Docs) ────→ Sprint Complete
```

---

## Agent Specifications

### Agent 1: Test Framework Foundation
**Wave**: 1
**Priority**: P0 (Blocker)
**Estimated Time**: 4-6 hours
**Dependencies**: None

**Scope**:
- Add test dependencies to workspace `Cargo.toml`
- Create test directory structure
- Create initial skeleton files
- Verify compilation

**Files to Read**:
- `/Users/firaenix/Projects/bitping/p2proxy/Cargo.toml`
- `/Users/firaenix/Projects/bitping/p2proxy/crates/p2proxy/Cargo.toml`
- `/Users/firaenix/Projects/bitping/p2proxy/SPRINT.md` (Task 1.1)

**Files to Create**:
- `crates/p2proxy/tests/common/mod.rs`
- `crates/p2proxy/tests/common/fixtures.rs`
- `crates/p2proxy/tests/common/mock_swarm.rs`
- `crates/p2proxy/tests/common/test_utils.rs`
- `crates/p2proxy/benches/throughput_bench.rs`
- `crates/p2proxy/benches/latency_bench.rs`

**Files to Modify**:
- `crates/p2proxy/Cargo.toml` - Add dev-dependencies

**Key Deliverables**:
```toml
# Add to crates/p2proxy/Cargo.toml
[dev-dependencies]
tokio-test = "0.4"
criterion = { version = "0.5", features = ["async_tokio"] }
proptest = "1.4"
mockall = "0.12"
serial_test = "3.0"
tempfile = "3.8"
assert_matches = "1.5"
```

**Acceptance Criteria**:
- `cargo test` compiles successfully (even with empty tests)
- `cargo bench --no-run` compiles successfully
- All skeleton files exist with proper module structure

---

### Agent 2: Mock P2P Components
**Wave**: 1
**Priority**: P0 (Blocker)
**Estimated Time**: 8-10 hours
**Dependencies**: None (can run parallel with Agent 1)

**Scope**:
- Implement `MockSwarm` with configurable behavior
- Implement `MockRelay` for relay testing
- Implement `MockPeer` for peer simulation
- Create helper functions for mock creation

**Files to Read**:
- `/Users/firaenix/Projects/bitping/p2proxy/crates/p2proxy/src/swarm.rs`
- `/Users/firaenix/Projects/bitping/p2proxy/crates/models/src/events.rs`
- `/Users/firaenix/Projects/bitping/p2proxy/crates/models/src/lib.rs`
- `/Users/firaenix/Projects/bitping/p2proxy/SPRINT.md` (Task 1.2)

**Files to Create/Implement**:
- `crates/p2proxy/tests/common/mock_swarm.rs` - Complete implementation
- `crates/p2proxy/tests/common/mock_relay.rs` - New file
- `crates/p2proxy/tests/common/mock_peer.rs` - New file

**Key Components**:
```rust
// MockSwarm capabilities
pub struct MockSwarm {
    local_peer_id: PeerId,
    connected_peers: HashMap<PeerId, MockPeer>,
    event_queue: VecDeque<SwarmEvent>,
    config: MockSwarmConfig,
}

// MockRelay capabilities
pub struct MockRelay {
    peer_id: PeerId,
    listening_addr: Multiaddr,
    connected_peers: HashSet<PeerId>,
}

// MockPeer capabilities
pub struct MockPeer {
    peer_id: PeerId,
    bandwidth: Bandwidth,
    latency: Duration,
    failure_rate: f64, // 0.0 to 1.0
}
```

**Acceptance Criteria**:
- Can create mock swarm with custom peer IDs
- Can simulate connection/disconnection events
- Can inject custom events into queue
- Mock relay can forward connections
- Mock peer can simulate various network conditions

---

### Agent 3: Test Fixtures and Utilities
**Wave**: 1
**Priority**: P0 (Blocker)
**Estimated Time**: 4-6 hours
**Dependencies**: None (can run parallel with Agents 1-2)

**Scope**:
- Create reusable test configurations
- Implement test keypair generation
- Create SOCKS5 test client helpers
- Implement assertion utilities
- Create bandwidth/latency measurement helpers

**Files to Read**:
- `/Users/firaenix/Projects/bitping/p2proxy/Config.yaml`
- `/Users/firaenix/Projects/bitping/p2proxy/crates/models/src/config.rs`
- `/Users/firaenix/Projects/bitping/p2proxy/crates/p2proxy/src/swarm.rs` (KEYPAIR)
- `/Users/firaenix/Projects/bitping/p2proxy/SPRINT.md` (Task 1.3)

**Files to Implement**:
- `crates/p2proxy/tests/common/fixtures.rs` - Complete implementation
- `crates/p2proxy/tests/common/test_utils.rs` - Complete implementation

**Key Functions**:
```rust
// Test configuration builders
pub fn test_config(servers: Vec<Server>) -> Config;
pub fn test_server(port: u16, protocol: ProxyProtocols) -> Server;

// Test keypairs (deterministic)
pub fn test_keypair(seed: u64) -> Keypair;

// SOCKS5 helpers
pub async fn mock_socks5_client(port: u16, target: Address) -> Result<TcpStream>;
pub async fn socks5_handshake(stream: &mut TcpStream) -> Result<()>;

// Assertion utilities
pub fn assert_bandwidth_within(actual: u64, expected: u64, tolerance_pct: f64);
pub async fn wait_for_connection(swarm: &Swarm, timeout: Duration) -> Result<PeerId>;
pub async fn wait_for_event<P>(predicate: P, timeout: Duration) -> Result<Events>
    where P: Fn(&Events) -> bool;

// Measurement utilities
pub struct BandwidthMeasurement {
    pub total_bytes: u64,
    pub duration: Duration,
    pub bytes_per_sec: f64,
}

pub async fn measure_bandwidth<F, Fut>(operation: F) -> BandwidthMeasurement;
```

**Acceptance Criteria**:
- Fixtures are deterministic and reproducible
- Utility functions compile and have basic tests
- Helpers reduce boilerplate in test code

---

### Agent 4: Connection Tests
**Wave**: 2
**Priority**: P0
**Estimated Time**: 8-10 hours
**Dependencies**: Wave 1 complete

**Scope**:
- Implement P2P connection establishment tests
- Implement SOCKS5 proxy connection tests
- Implement RPC connection tests
- Cover all scenarios from SPRINT.md Tasks 2.1, 2.2, 2.3

**Files to Read**:
- All Wave 1 outputs (mock components, fixtures, utilities)
- `/Users/firaenix/Projects/bitping/p2proxy/crates/p2proxy/src/swarm.rs`
- `/Users/firaenix/Projects/bitping/p2proxy/crates/p2proxy/src/proxy_protocols/socks_stream.rs`
- `/Users/firaenix/Projects/bitping/p2proxy/crates/models/src/lib.rs` (RPC traits)
- `/Users/firaenix/Projects/bitping/p2proxy/SPRINT.md` (Phase 2)

**Files to Create**:
- `crates/p2proxy/tests/integration/connection_tests.rs`

**Test Scenarios** (minimum):
1. Direct P2P connection
2. Relay-mediated connection
3. Multiple simultaneous peer connections
4. Reconnection after network change
5. SOCKS5 handshake (NoAuth)
6. SOCKS5 connection request (IPv4, IPv6, domain)
7. SOCKS5 session lifecycle
8. Concurrent SOCKS5 sessions
9. RPC connection establishment
10. RPC method invocation (all Counter methods)
11. RPC event streaming

**Example Test Structure**:
```rust
#[tokio::test]
async fn test_p2p_direct_connection() { ... }

#[tokio::test]
async fn test_p2p_relay_connection() { ... }

#[tokio::test]
async fn test_socks5_handshake() { ... }

#[tokio::test]
async fn test_rpc_get_server_states() { ... }
```

**Acceptance Criteria**:
- All 11+ test scenarios implemented
- All tests pass
- Test coverage for connection happy paths
- Tests run in <30 seconds total

---

### Agent 5: Disconnection Tests
**Wave**: 2
**Priority**: P0
**Estimated Time**: 6-8 hours
**Dependencies**: Wave 1 complete, Agent 4 can run in parallel

**Scope**:
- Implement graceful disconnection tests
- Implement network failure tests
- Implement authentication failure tests
- Cover all scenarios from SPRINT.md Tasks 3.1, 3.2, 3.3

**Files to Read**:
- All Wave 1 outputs
- `/Users/firaenix/Projects/bitping/p2proxy/crates/p2proxy/src/swarm.rs`
- `/Users/firaenix/Projects/bitping/p2proxy/crates/models/src/events.rs`
- `/Users/firaenix/Projects/bitping/p2proxy/SPRINT.md` (Phase 3)

**Files to Create**:
- `crates/p2proxy/tests/integration/disconnection_tests.rs`

**Test Scenarios** (minimum):
1. Graceful peer disconnect
2. Graceful shutdown during active sessions
3. Client disconnect handling
4. Sudden peer unavailability
5. Network partition
6. Relay failure
7. Partial data transfer failure
8. Invalid API key
9. gRPC service unavailable

**Key Testing Patterns**:
```rust
#[tokio::test]
async fn test_graceful_peer_disconnect() {
    let (swarm1, swarm2) = connect_test_peers().await;

    // Disconnect gracefully
    swarm2.disconnect(&swarm1.local_peer_id()).await;

    // Verify event emitted
    let event = wait_for_event(|e| matches!(e, Events::Connection(ConnectionEvents::Disconnected)), Duration::from_secs(5)).await.unwrap();

    // Verify cleanup
    assert_eq!(swarm1.connected_peers().len(), 0);
}

#[tokio::test]
async fn test_sudden_peer_failure() {
    let (swarm1, swarm2) = connect_test_peers().await;

    // Kill peer2 abruptly
    drop(swarm2);

    // Verify timeout detection
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert!(!swarm1.is_connected_to(&peer2_id));
}
```

**Acceptance Criteria**:
- All 9+ test scenarios implemented
- All tests pass
- Proper cleanup verified
- No resource leaks detected

---

### Agent 6: Throughput Tests
**Wave**: 2
**Priority**: P0
**Estimated Time**: 6-8 hours
**Dependencies**: Wave 1 complete, Agent 4-5 can run in parallel

**Scope**:
- Implement bandwidth measurement tests
- Implement maximum throughput tests
- Implement bandwidth limit compliance tests
- Create throughput benchmarks
- Cover all scenarios from SPRINT.md Tasks 4.1, 4.2, 4.3

**Files to Read**:
- All Wave 1 outputs
- `/Users/firaenix/Projects/bitping/p2proxy/crates/p2proxy/src/proxy_protocols/socks_stream.rs`
- `/Users/firaenix/Projects/bitping/p2proxy/SPRINT.md` (Phase 4)

**Files to Create**:
- `crates/p2proxy/tests/integration/throughput_tests.rs`
- `crates/p2proxy/benches/throughput_bench.rs` (implement benchmarks)

**Test Scenarios** (minimum):
1. Accurate byte counting (1MB, 10MB, 100MB)
2. Bandwidth metrics accuracy
3. Hash verification (blake3)
4. Single session max throughput
5. Concurrent session throughput (10, 50, 100 sessions)
6. Large file transfer (1GB+)
7. Peer selection by min_bandwidth
8. Bandwidth requirement updates

**Benchmark Implementation**:
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};

fn throughput_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");

    for size in [1_000, 10_000, 100_000, 1_000_000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.to_async(Runtime::new().unwrap()).iter(|| async {
                let data = vec![0u8; size];
                transfer_data(black_box(&data)).await
            });
        });
    }

    group.finish();
}

criterion_group!(benches, throughput_benchmarks);
criterion_main!(benches);
```

**Acceptance Criteria**:
- Byte counting accurate within 1%
- Achieves >80% of baseline TCP throughput
- Handles 100+ concurrent sessions
- Benchmarks compile and run

---

### Agent 7: Jitter and Latency Tests
**Wave**: 2
**Priority**: P1
**Estimated Time**: 6-8 hours
**Dependencies**: Wave 1 complete, Agents 4-6 can run in parallel

**Scope**:
- Implement latency measurement tests
- Implement jitter analysis tests
- Create latency benchmarks
- Cover all scenarios from SPRINT.md Tasks 5.1, 5.2, 5.3

**Files to Read**:
- All Wave 1 outputs
- `/Users/firaenix/Projects/bitping/p2proxy/crates/p2proxy/src/swarm.rs`
- `/Users/firaenix/Projects/bitping/p2proxy/SPRINT.md` (Phase 5)

**Files to Create**:
- `crates/p2proxy/tests/integration/jitter_tests.rs`
- `crates/p2proxy/benches/latency_bench.rs` (implement benchmarks)

**Test Scenarios** (minimum):
1. Round-trip time measurement
2. Request-response latency
3. SOCKS5 handshake latency
4. Connection establishment latency
5. First-byte latency
6. Packet timing variance
7. Jitter under load
8. Latency percentiles (p50, p95, p99)

**Key Measurement Structure**:
```rust
#[derive(Debug, Clone)]
pub struct LatencyStats {
    pub min: Duration,
    pub max: Duration,
    pub mean: Duration,
    pub median: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub std_dev: Duration,
}

pub async fn measure_latency_distribution<F, Fut>(
    iterations: usize,
    operation: F
) -> LatencyStats
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

    calculate_stats(&measurements)
}

pub fn calculate_jitter(timings: &[Duration]) -> Duration {
    // RFC 3550 jitter calculation
}
```

**Acceptance Criteria**:
- Latency measurements accurate within 5ms
- Jitter calculation follows RFC 3550
- Percentiles calculated correctly
- Targets: <100ms direct, <250ms relay
- Benchmarks compile and run

---

### Agent 8: Core Stability Tests
**Wave**: 2
**Priority**: P0
**Estimated Time**: 4-6 hours
**Dependencies**: Wave 1 complete, Agents 4-7 can run in parallel

**Scope**:
- Implement reconnection logic tests
- Implement basic stress tests
- Cover SPRINT.md Task 6.2 (not long-running tests)

**Files to Read**:
- All Wave 1 outputs
- `/Users/firaenix/Projects/bitping/p2proxy/crates/p2proxy/src/swarm.rs`
- `/Users/firaenix/Projects/bitping/p2proxy/SPRINT.md` (Phase 6 - Task 6.2 only)

**Files to Create**:
- `crates/p2proxy/tests/integration/stability_tests.rs` (partial)

**Test Scenarios** (minimum):
1. Exponential backoff verification
2. Session restoration after reconnect
3. Peer rotation/failover
4. Connection churn (rapid connect/disconnect)
5. High session turnover
6. Resource exhaustion handling

**Example Exponential Backoff Test**:
```rust
#[tokio::test]
async fn test_exponential_backoff() {
    let config = test_config_with_unreachable_peer();
    let proxy = ProxyNetwork::with_config(config).await.unwrap();

    let mut retry_intervals = Vec::new();

    for _ in 0..6 {
        let start = Instant::now();
        let _ = proxy.connect().await; // Will fail
        retry_intervals.push(start.elapsed());
    }

    // Verify exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s (capped)
    assert_approx_eq!(retry_intervals[0].as_secs_f64(), 1.0, 0.2);
    assert_approx_eq!(retry_intervals[1].as_secs_f64(), 2.0, 0.3);
    assert_approx_eq!(retry_intervals[2].as_secs_f64(), 4.0, 0.5);
    assert_approx_eq!(retry_intervals[3].as_secs_f64(), 8.0, 0.8);
    assert_approx_eq!(retry_intervals[4].as_secs_f64(), 16.0, 1.5);
    assert!(retry_intervals[5].as_secs_f64() <= 30.5); // Capped at 30s
}
```

**Acceptance Criteria**:
- Reconnection logic verified
- Backoff intervals correct
- Stress tests complete in <2 minutes
- No resource leaks under stress

---

### Agent 9: Long-Running Stability Tests
**Wave**: 3
**Priority**: P1
**Estimated Time**: 4-6 hours (implementation, not execution)
**Dependencies**: Wave 2 complete

**Scope**:
- Implement 24-hour stability test
- Implement long-running data transfer test
- Implement idle connection test
- Cover SPRINT.md Task 6.1

**Files to Read**:
- All Wave 1-2 outputs
- `/Users/firaenix/Projects/bitping/p2proxy/SPRINT.md` (Task 6.1)

**Files to Modify**:
- `crates/p2proxy/tests/integration/stability_tests.rs` (add long-running tests)

**Test Scenarios**:
1. 24-hour connection stability
2. 6-hour continuous data transfer
3. 2-hour idle connection
4. Memory leak detection
5. CPU usage monitoring

**Implementation Pattern**:
```rust
#[tokio::test]
#[ignore] // Long-running test, run manually
async fn test_24hour_stability() {
    let start_mem = measure_memory_usage();
    let start_time = Instant::now();

    let (swarm1, swarm2) = connect_test_peers().await;

    for hour in 0..24 {
        tokio::time::sleep(Duration::from_secs(3600)).await;

        // Verify still connected
        assert!(swarm1.is_connected(&swarm2.local_peer_id()));

        // Check memory growth
        let current_mem = measure_memory_usage();
        let growth = (current_mem as f64 - start_mem as f64) / start_mem as f64;
        assert!(growth < 0.10, "Memory grew by {:.1}%", growth * 100.0);

        // Check CPU usage
        let cpu = measure_cpu_usage();
        assert!(cpu < 5.0, "CPU usage too high: {:.1}%", cpu);

        tracing::info!("Hour {}/24: Stable (mem: {:.1}%, cpu: {:.1}%)",
                       hour + 1, growth * 100.0, cpu);
    }
}
```

**Acceptance Criteria**:
- Tests compile and are marked with `#[ignore]`
- Memory measurement implemented
- CPU measurement implemented
- Clear instructions for running manually
- Tests can be run with `cargo test -- --ignored`

---

### Agent 10: Stress and Chaos Tests
**Wave**: 3
**Priority**: P1
**Estimated Time**: 6-8 hours
**Dependencies**: Wave 2 complete, can run parallel with Agent 9

**Scope**:
- Implement chaos testing scenarios
- Implement network simulation tests
- Cover SPRINT.md Task 6.3

**Files to Read**:
- All Wave 1-2 outputs
- `/Users/firaenix/Projects/bitping/p2proxy/SPRINT.md` (Task 6.3)

**Files to Modify**:
- `crates/p2proxy/tests/integration/stability_tests.rs` (add chaos tests)

**Test Scenarios**:
1. Random packet loss simulation
2. Random latency injection
3. Random bandwidth throttling
4. Network partition healing
5. Chaos under load

**Implementation Approach**:
```rust
// Use tokio's delay/drop to simulate network issues

#[tokio::test]
async fn test_packet_loss_resilience() {
    let mut mock_swarm = MockSwarm::new(MockSwarmConfig {
        packet_loss_rate: 0.05, // 5% packet loss
        ..Default::default()
    });

    // Transfer data with packet loss
    let data = generate_test_data(10_000_000); // 10MB
    let result = transfer_with_mock(&mut mock_swarm, data).await;

    // Should still succeed despite packet loss
    assert!(result.is_ok());

    // But may take longer
    assert!(result.unwrap().duration > expected_duration * 1.1);
}

#[tokio::test]
async fn test_latency_variance_handling() {
    let mut mock_swarm = MockSwarm::new(MockSwarmConfig {
        min_latency: Duration::from_millis(10),
        max_latency: Duration::from_millis(500),
        ..Default::default()
    });

    // Measure jitter under variable latency
    let jitter = measure_jitter_with_mock(&mut mock_swarm, 100).await;

    // System should adapt
    assert!(jitter < Duration::from_millis(50));
}
```

**Acceptance Criteria**:
- Chaos scenarios implemented
- Network simulation works
- Tests verify resilience
- Clear documentation of test behavior

---

### Agent 11: CI Integration and Documentation
**Wave**: 4
**Priority**: P1
**Estimated Time**: 3-4 hours
**Dependencies**: Wave 3 complete

**Scope**:
- Create CI workflow for tests
- Add test documentation
- Create test README
- Add code coverage reporting (optional)

**Files to Read**:
- `.github/workflows/release.yml` (for reference)
- All test implementations from Waves 1-3
- `/Users/firaenix/Projects/bitping/p2proxy/SPRINT.md`

**Files to Create**:
- `.github/workflows/test.yml`
- `crates/p2proxy/tests/README.md`

**Files to Modify**:
- `README.md` - Add testing section
- `CLAUDE.md` - Add test execution commands

**CI Workflow Structure**:
```yaml
name: Tests
on:
  push:
    branches: [master]
  pull_request:

jobs:
  test:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
        rust: [stable]

    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: ${{ matrix.rust }}
          override: true

      - name: Run tests
        run: cargo test --all --verbose

      - name: Run benchmarks (compile only)
        run: cargo bench --no-run

  long-running:
    runs-on: ubuntu-latest
    # Only run on manual trigger or weekly schedule
    if: github.event_name == 'workflow_dispatch' || github.event.schedule
    steps:
      - uses: actions/checkout@v3
      - name: Run long tests
        run: cargo test --all -- --ignored --nocapture
```

**Test README Contents**:
- Overview of test structure
- How to run different test categories
- How to run specific tests
- How to run benchmarks
- Interpreting test results
- Troubleshooting common issues

**Acceptance Criteria**:
- CI workflow runs on push/PR
- All tests pass in CI
- Documentation is clear and complete
- Test commands added to CLAUDE.md

---

## Agent Prompts

### Agent 1 Prompt
```
You are implementing the test framework foundation for P2Proxy, a Rust P2P proxy system.

Context:
- Read SPRINT.md Task 1.1 for requirements
- Read Cargo.toml files to understand project structure
- The project has no existing tests

Your tasks:
1. Add dev-dependencies to crates/p2proxy/Cargo.toml:
   - tokio-test = "0.4"
   - criterion = { version = "0.5", features = ["async_tokio"] }
   - proptest = "1.4"
   - mockall = "0.12"
   - serial_test = "3.0"
   - tempfile = "3.8"
   - assert_matches = "1.5"

2. Create test directory structure:
   - crates/p2proxy/tests/common/mod.rs (with pub mod declarations)
   - crates/p2proxy/tests/common/fixtures.rs (empty skeleton)
   - crates/p2proxy/tests/common/mock_swarm.rs (empty skeleton)
   - crates/p2proxy/tests/common/test_utils.rs (empty skeleton)
   - crates/p2proxy/benches/throughput_bench.rs (minimal benchmark skeleton)
   - crates/p2proxy/benches/latency_bench.rs (minimal benchmark skeleton)

3. Verify compilation:
   - Run `cargo test --no-run` to verify compilation
   - Run `cargo bench --no-run` to verify benchmarks compile

Success criteria:
- All files created
- cargo test compiles
- cargo bench compiles
- No compilation errors

Do not implement test logic yet - just create the structure.
```

### Agent 2 Prompt
```
You are implementing mock P2P network components for testing P2Proxy.

Context:
- Read crates/p2proxy/src/swarm.rs to understand the Behaviour struct and libp2p usage
- Read crates/models/src/events.rs to understand event types
- Read SPRINT.md Task 1.2 for requirements

Your tasks:
1. Implement MockSwarm in crates/p2proxy/tests/common/mock_swarm.rs:
   - Struct with local_peer_id, connected_peers, event_queue, config
   - Methods: new(), connect_to_peer(), simulate_disconnect(), inject_event()
   - Configurable behavior (success rate, latency, etc.)

2. Implement MockRelay in crates/p2proxy/tests/common/mock_relay.rs:
   - Simulates a relay server
   - Can accept reservations
   - Can forward connections

3. Implement MockPeer in crates/p2proxy/tests/common/mock_peer.rs:
   - Represents a remote peer
   - Configurable bandwidth, latency, failure rate
   - Can respond to queries

Use real libp2p types (PeerId, Multiaddr, etc.) but mock the behavior.
All mocks should be deterministic when given a seed.

Success criteria:
- All mock components compile
- Can create instances with custom config
- Can simulate various network conditions
- Well-documented with examples
```

### Agent 3 Prompt
```
You are implementing test fixtures and utilities for P2Proxy tests.

Context:
- Read Config.yaml and crates/models/src/config.rs for configuration structure
- Read crates/p2proxy/src/swarm.rs for keypair handling
- Read SPRINT.md Task 1.3 for requirements

Your tasks:
1. Implement fixtures in crates/p2proxy/tests/common/fixtures.rs:
   - test_config(servers: Vec<Server>) -> Config
   - test_server(port: u16, protocol: ProxyProtocols) -> Server
   - test_keypair(seed: u64) -> Keypair (deterministic)
   - TEST_PORTS constant range for tests
   - Sample test data generators

2. Implement utilities in crates/p2proxy/tests/common/test_utils.rs:
   - mock_socks5_client(port: u16, target: Address) -> Result<TcpStream>
   - socks5_handshake(stream: &mut TcpStream) -> Result<()>
   - assert_bandwidth_within(actual: u64, expected: u64, tolerance: f64)
   - wait_for_event<P>(predicate: P, timeout: Duration) -> Result<Events>
   - measure_bandwidth<F>(operation: F) -> BandwidthMeasurement
   - measure_latency<F>(operation: F, iterations: usize) -> LatencyStats

3. Update crates/p2proxy/tests/common/mod.rs to export all modules

All helpers should be well-documented with usage examples.
Use deterministic values where possible (seeded RNG).

Success criteria:
- All fixtures and utilities compile
- Deterministic and reproducible
- Well-documented with examples
- Reduce test boilerplate significantly
```

### Agent 4 Prompt
```
You are implementing connection tests for P2Proxy.

Context:
- All Wave 1 work is complete (mocks, fixtures, utilities available)
- Read crates/p2proxy/src/swarm.rs for P2P connection logic
- Read crates/p2proxy/src/proxy_protocols/socks_stream.rs for SOCKS5 logic
- Read crates/models/src/lib.rs for RPC traits
- Read SPRINT.md Phase 2 (Tasks 2.1, 2.2, 2.3) for requirements

Your tasks:
Create crates/p2proxy/tests/integration/connection_tests.rs with:

1. P2P Connection Tests:
   - test_p2p_direct_connection()
   - test_p2p_relay_connection()
   - test_p2p_multiple_peers()
   - test_p2p_reconnection()

2. SOCKS5 Tests:
   - test_socks5_handshake_noauth()
   - test_socks5_connect_ipv4()
   - test_socks5_connect_ipv6()
   - test_socks5_connect_domain()
   - test_socks5_session_lifecycle()
   - test_socks5_concurrent_sessions()

3. RPC Tests:
   - test_rpc_connection()
   - test_rpc_get_server_states()
   - test_rpc_get_stats()
   - test_rpc_watch_events()

Use the mock components and test utilities from Wave 1.
All tests should have timeouts and proper cleanup.

Success criteria:
- All 15+ tests implemented
- All tests pass
- Tests complete in <30 seconds
- No flaky tests
```

### Agent 5 Prompt
```
You are implementing disconnection tests for P2Proxy.

Context:
- All Wave 1 work is complete
- Read crates/p2proxy/src/swarm.rs for cleanup logic
- Read crates/models/src/events.rs for disconnection events
- Read SPRINT.md Phase 3 (Tasks 3.1, 3.2, 3.3) for requirements

Your tasks:
Create crates/p2proxy/tests/integration/disconnection_tests.rs with:

1. Graceful Disconnection Tests:
   - test_graceful_peer_disconnect()
   - test_shutdown_during_active_sessions()
   - test_client_disconnect_handling()

2. Network Failure Tests:
   - test_sudden_peer_unavailability()
   - test_network_partition()
   - test_relay_failure()
   - test_partial_transfer_failure()

3. Auth Failure Tests:
   - test_invalid_api_key()
   - test_grpc_unavailable()

Use mock components to simulate failures.
Verify proper cleanup (no resource leaks).
Test timeout detection and error handling.

Success criteria:
- All 9+ tests implemented
- All tests pass
- Cleanup verified in all scenarios
- No resource leaks
```

### Agent 6 Prompt
```
You are implementing throughput tests and benchmarks for P2Proxy.

Context:
- All Wave 1 work is complete
- Read crates/p2proxy/src/proxy_protocols/socks_stream.rs for bandwidth tracking
- Read SPRINT.md Phase 4 (Tasks 4.1, 4.2, 4.3) for requirements

Your tasks:
1. Create crates/p2proxy/tests/integration/throughput_tests.rs with:
   - test_accurate_byte_counting() - 1MB, 10MB, 100MB transfers
   - test_bandwidth_metrics_accuracy()
   - test_hash_verification() - verify blake3 hashes
   - test_single_session_max_throughput()
   - test_concurrent_session_throughput() - 10, 50, 100 sessions
   - test_large_file_transfer() - 1GB+
   - test_min_bandwidth_enforcement()

2. Implement crates/p2proxy/benches/throughput_bench.rs:
   - Benchmark data transfer at various sizes (1KB, 1MB, 10MB, 100MB)
   - Use criterion framework
   - Measure bytes/sec

Generate test data with known sizes and hashes.
Measure and verify bandwidth accuracy (±1% tolerance).

Success criteria:
- All tests pass
- Byte counting accurate within 1%
- Benchmarks compile and run
- >80% of TCP baseline throughput
```

### Agent 7 Prompt
```
You are implementing latency and jitter tests for P2Proxy.

Context:
- All Wave 1 work is complete
- Read SPRINT.md Phase 5 (Tasks 5.1, 5.2, 5.3) for requirements

Your tasks:
1. Create crates/p2proxy/tests/integration/jitter_tests.rs with:
   - test_round_trip_time() - measure RTT
   - test_connection_establishment_latency()
   - test_socks5_handshake_latency()
   - test_first_byte_latency()
   - test_packet_timing_variance()
   - test_jitter_under_load()
   - test_latency_percentiles() - p50, p95, p99

2. Implement crates/p2proxy/benches/latency_bench.rs:
   - Benchmark connection setup
   - Benchmark SOCKS5 handshake
   - Benchmark small message round-trip

Create LatencyStats struct with min/max/mean/median/p95/p99.
Implement RFC 3550 jitter calculation.

Success criteria:
- All tests pass
- Latency measurements accurate within 5ms
- Jitter calculation correct
- Targets: <100ms direct, <250ms relay
- Benchmarks work
```

### Agent 8 Prompt
```
You are implementing core stability tests for P2Proxy (not long-running).

Context:
- All Wave 1 work is complete
- Read crates/p2proxy/src/swarm.rs for reconnection logic
- Read SPRINT.md Task 6.2 for requirements

Your tasks:
Create crates/p2proxy/tests/integration/stability_tests.rs with:

1. Reconnection Logic Tests:
   - test_exponential_backoff() - verify 1s, 2s, 4s, 8s, 16s, 30s intervals
   - test_session_restoration()
   - test_peer_rotation_failover()

2. Stress Tests:
   - test_connection_churn() - rapid connect/disconnect
   - test_high_session_turnover() - 1000+ sessions/min
   - test_resource_exhaustion_handling()

These tests should complete in <2 minutes.
Long-running tests will be added by Agent 9.

Success criteria:
- All tests pass
- Backoff intervals verified
- No resource leaks
- Tests complete quickly
```

### Agent 9 Prompt
```
You are implementing long-running stability tests for P2Proxy.

Context:
- Wave 2 is complete with core stability tests
- Read existing stability_tests.rs
- Read SPRINT.md Task 6.1 for requirements

Your tasks:
Add to crates/p2proxy/tests/integration/stability_tests.rs:

1. Long-running tests (all marked with #[ignore]):
   - test_24hour_stability() - keep connection alive 24 hours
   - test_longrunning_transfer() - transfer data for 6 hours
   - test_idle_connection() - no data for 2 hours

2. Monitoring utilities:
   - measure_memory_usage() -> usize
   - measure_cpu_usage() -> f64
   - Monitor memory growth (must be <10%)
   - Monitor CPU usage (must be <5% when idle)

These tests are for manual execution only.
Add documentation for running: cargo test -- --ignored

Success criteria:
- Tests compile and marked with #[ignore]
- Memory/CPU measurement works
- Clear instructions for manual execution
- Tests verify stability over time
```

### Agent 10 Prompt
```
You are implementing chaos and stress tests for P2Proxy.

Context:
- Wave 2 is complete
- Read existing stability_tests.rs
- Read SPRINT.md Task 6.3 for requirements

Your tasks:
Add to crates/p2proxy/tests/integration/stability_tests.rs:

1. Network Chaos Tests:
   - test_packet_loss_resilience() - 5%, 10%, 20% loss
   - test_latency_variance_handling() - 10ms-500ms random
   - test_bandwidth_throttling() - random throttling
   - test_network_partition_healing()

2. Stress Tests:
   - test_chaos_under_load() - multiple chaos conditions simultaneously

Use MockSwarm configuration to simulate network issues.
Tests should verify the system remains stable and recovers.

Success criteria:
- All chaos scenarios implemented
- System handles chaos gracefully
- Recovery verified
- Clear test documentation
```

### Agent 11 Prompt
```
You are implementing CI integration and test documentation for P2Proxy.

Context:
- All tests are implemented (Waves 1-3 complete)
- Read .github/workflows/release.yml for CI patterns
- Read SPRINT.md for test overview

Your tasks:
1. Create .github/workflows/test.yml:
   - Run on push and PR
   - Test on ubuntu-latest and macos-latest
   - Run: cargo test --all
   - Run: cargo bench --no-run (compile benchmarks)
   - Optional: weekly schedule for long-running tests

2. Create crates/p2proxy/tests/README.md:
   - Test structure overview
   - How to run different test categories
   - How to run specific tests
   - How to run benchmarks
   - How to run long-running tests
   - Troubleshooting

3. Update README.md:
   - Add "Testing" section
   - Reference tests/README.md

4. Update CLAUDE.md:
   - Add test execution commands
   - Add benchmark commands

Make documentation clear and actionable.

Success criteria:
- CI workflow works
- All tests pass in CI
- Documentation complete
- Easy for developers to run tests
```

---

## Execution Strategy

### Sequential Execution (Simplest)

Execute waves one at a time:

```bash
# Wave 1 - Run sequentially or in parallel
# Agent 1
# Agent 2
# Agent 3

# Wait for Wave 1 to complete, then Wave 2
# Agent 4
# Agent 5
# Agent 6
# Agent 7
# Agent 8

# Wait for Wave 2, then Wave 3
# Agent 9
# Agent 10

# Wait for Wave 3, then Wave 4
# Agent 11
```

### Parallel Execution (Fastest)

Launch all agents in a wave simultaneously:

**Wave 1** (Launch in parallel):
```bash
# In separate terminals or using Task tool:
# Launch Agent 1, Agent 2, Agent 3 in parallel
```

**Wave 2** (After Wave 1 completes, launch in parallel):
```bash
# Launch Agents 4, 5, 6, 7, 8 in parallel
```

**Wave 3** (After Wave 2 completes):
```bash
# Launch Agents 9, 10 in parallel
```

**Wave 4** (After Wave 3 completes):
```bash
# Launch Agent 11
```

### Using Claude Code Task Tool

To launch agents using the Task tool:

```markdown
# Wave 1 - Launch in single message with 3 Task calls

Task 1:
subagent_type: general-purpose
description: Setup test framework foundation
prompt: [Agent 1 Prompt from above]

Task 2:
subagent_type: general-purpose
description: Create mock P2P components
prompt: [Agent 2 Prompt from above]

Task 3:
subagent_type: general-purpose
description: Create test fixtures and utilities
prompt: [Agent 3 Prompt from above]
```

---

## Monitoring Progress

### After Each Wave

1. **Verify compilation**: `cargo test --no-run`
2. **Run tests**: `cargo test`
3. **Check coverage**: Review which test scenarios are complete
4. **Fix issues**: Address any failing tests before next wave

### Success Metrics

- **Wave 1**: Structure created, compiles cleanly
- **Wave 2**: All core tests pass, good coverage
- **Wave 3**: Advanced tests implemented
- **Wave 4**: CI passing, documentation complete

---

## Risk Mitigation

### Common Issues

1. **Agent conflicts**: Ensure Wave dependencies are respected
2. **Compilation errors**: Run `cargo test --no-run` frequently
3. **Flaky tests**: Mark for review and stabilization
4. **Long execution**: Use `#[ignore]` for long tests

### Debugging Failed Tests

1. Run with logging: `RUST_LOG=debug cargo test test_name`
2. Run single test: `cargo test test_name`
3. Check test output: `cargo test -- --nocapture`
4. Review mock behavior and timing

---

## Post-Sprint Activities

After all agents complete:

1. **Test Review Session**:
   - Review all tests
   - Identify gaps in coverage
   - Improve flaky tests

2. **Performance Baseline**:
   - Run benchmarks
   - Document baseline metrics
   - Set regression thresholds

3. **Documentation Polish**:
   - Update CLAUDE.md with test insights
   - Add troubleshooting guide
   - Document performance targets

4. **CI Optimization**:
   - Tune CI timeouts
   - Configure test parallelization
   - Set up coverage reporting

---

## Summary

This plan organizes the test infrastructure implementation into **11 specialized agents** across **4 waves**, with clear dependencies and parallelization opportunities. Each agent has specific deliverables, acceptance criteria, and ready-to-use prompts.

**Total Timeline**: 2-3 weeks
**Peak Parallelization**: 5 agents running simultaneously (Wave 2)
**Final Deliverable**: Complete test infrastructure with CI integration

Follow the wave structure, use the prompts provided, and monitor progress after each wave. The sprint document (SPRINT.md) remains the source of truth for requirements, while this plan (AGENT_PLAN.md) provides the execution strategy.
