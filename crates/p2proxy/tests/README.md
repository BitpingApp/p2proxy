# P2Proxy Test Suite

This directory contains simplified, focused tests for P2Proxy covering connection handling, reconnection logic, failover scenarios, and basic throughput validation.

## Table of Contents

- [Test Structure](#test-structure)
- [Running Tests](#running-tests)
- [Test Categories](#test-categories)
- [Troubleshooting](#troubleshooting)

## Test Structure

```
tests/
├── common/                      # Shared test utilities and mocks
│   ├── mod.rs                   # Common module exports
│   ├── fixtures.rs              # Test fixtures and configurations
│   ├── test_utils.rs            # Helper functions and utilities
│   ├── mock_swarm.rs            # Mock libp2p Swarm for testing
│   ├── mock_peer.rs             # Mock peer node implementation
│   └── mock_relay.rs            # Mock relay server
├── connection_tests.rs          # Connection establishment tests (14 tests)
├── disconnection_tests.rs       # Disconnection and failure tests (9 tests)
├── throughput_tests.rs          # Basic bandwidth tests (3 tests)
└── stability_tests.rs           # Failover and reconnection tests (11 + 3 long-running)
```

## Running Tests

### Quick Start

Run all standard tests:
```bash
cargo test --all --verbose
```

Run tests for the p2proxy crate only:
```bash
cargo test -p p2proxy --verbose
```

### Running Specific Test Categories

#### Connection Tests (14 tests)
Test P2P connection establishment, SOCKS5 proxy functionality, and RPC communication:
```bash
cargo test --test connection_tests
```

#### Disconnection Tests (9 tests)
Test graceful disconnections, network failures, and error handling:
```bash
cargo test --test disconnection_tests
```

#### Throughput Tests (3 tests)
Test basic byte counting and concurrent sessions:
```bash
cargo test --test throughput_tests
```

#### Stability Tests (11 quick + 3 long-running)
Test reconnection logic, failover, and resource management:
```bash
# Run only quick stability tests
cargo test --test stability_tests

# Run long-running stability tests (can take hours)
cargo test --test stability_tests -- --ignored --nocapture
```

### Running Individual Tests

Run a specific test by name:
```bash
cargo test test_peer_rotation_failover
```

Run tests matching a pattern:
```bash
# Run all failover-related tests
cargo test failover

# Run all reconnection tests
cargo test reconnection
```

Run with debug logging:
```bash
RUST_LOG=debug cargo test test_peer_rotation_failover -- --nocapture
```

## Test Categories

### Connection Tests (`connection_tests.rs`) - 14 tests

Tests covering connection establishment and basic functionality:

- **P2P Connection Tests** (4 tests)
  - Direct peer-to-peer connections
  - Relay-mediated connections
  - Multiple concurrent peer connections
  - Disconnection and reconnection

- **SOCKS5 Proxy Tests** (6 tests)
  - SOCKS5 handshake protocol
  - Connection requests (IPv4, IPv6, domain names)
  - Session lifecycle management
  - Concurrent proxy sessions

- **RPC Communication Tests** (4 tests)
  - Daemon-to-UI RPC connection
  - Method invocation (get_server_states, get_stats, etc.)
  - Event streaming
  - Multiple client handling

**Key Validations**:
- Connection events are properly emitted
- Peer tracking is accurate
- Protocol exchanges complete successfully
- Concurrent sessions don't interfere

### Disconnection Tests (`disconnection_tests.rs`) - 9 tests

Tests covering clean and abrupt disconnection scenarios:

- **Graceful Disconnection** (3 tests)
  - Peer disconnection with cleanup
  - Shutdown during active sessions
  - Client disconnect handling

- **Network Failures** (4 tests)
  - Sudden peer unavailability
  - Network partitions
  - Relay failures
  - Partial transfer failures

- **Authentication Failures** (2 tests)
  - Invalid API key handling
  - gRPC service unavailability

**Key Validations**:
- No panics during failure scenarios
- Resources are properly cleaned up
- Error events are correctly emitted
- Recovery mechanisms work as expected

### Throughput Tests (`throughput_tests.rs`) - 3 tests

Simplified tests covering basic data transfer:

- **Basic byte counting accuracy** - Verifies byte counting within 1% tolerance
- **Concurrent session throughput** - Tests 10 and 50 concurrent sessions
- **Minimum bandwidth enforcement** - Validates configuration compliance

**Key Validations**:
- Byte counts match transferred data within 1%
- Concurrent sessions don't interfere
- Configuration is properly applied

**Note**: This suite focuses on ensuring data flows correctly rather than stringent performance measurements, as connection quality varies by peer.

### Stability Tests (`stability_tests.rs`) - 11 quick + 3 long-running

Tests covering reconnection logic and failover scenarios:

**Reconnection Logic Tests** (3 tests):
- **Exponential backoff** - Verifies retry intervals (1s, 2s, 4s, 8s, 16s, 30s)
- **Session restoration** - Tests reconnection after disconnect
- **Peer rotation and failover** ⭐ **CRITICAL** - Switches to alternative peer when primary fails

**Stability Tests** (8 tests):
- Connection churn (150 rapid connect/disconnect cycles)
- High session turnover (150 short-lived sessions)
- Resource exhaustion handling
- Concurrent connections (50 simultaneous)
- Multiple disconnect/reconnect cycles
- Concurrent disconnections
- Network partition healing ⭐ **CRITICAL** - Recovery after network outage

**Long-Running Tests** (3 tests, marked with `#[ignore]`):
- 24-hour connection stability
- 6-hour continuous data transfer
- 2-hour idle connection

**Key Validations**:
- Exponential backoff works correctly
- Connections recover from failures
- Failover switches to backup peers
- Network partitions are detected and recovered
- Resources are cleaned up properly

### Running Long-Running Tests

Long-running tests are marked with `#[ignore]` and must be run manually:

```bash
# Run all long-running tests
cargo test --test stability_tests -- --ignored --nocapture

# Run specific tests
cargo test test_24hour_stability -- --ignored --nocapture
cargo test test_longrunning_transfer -- --ignored --nocapture
cargo test test_idle_connection -- --ignored --nocapture
```

**Expected Durations**:
- `test_24hour_stability`: 24 hours
- `test_longrunning_transfer`: 6 hours
- `test_idle_connection`: 2 hours

## Expected Durations

Test suite execution times (on modern hardware):

| Test Suite | Tests | Duration | Notes |
|------------|-------|----------|-------|
| All tests (excluding ignored) | 37 | 2-4 minutes | Standard test run |
| Connection tests | 14 | 30-60 seconds | Fast, mostly unit tests |
| Disconnection tests | 9 | 30-60 seconds | Includes timeout tests |
| Throughput tests | 3 | 15-30 seconds | Basic data transfer |
| Stability tests (non-ignored) | 11 | 60-120 seconds | Reconnection and failover |
| Long-running tests (--ignored) | 3 | 6-24 hours | Overnight/weekend tests |

**CI Performance**:
- Full test suite: 3-8 minutes (with caching)
- Long-running tests: Run weekly on schedule

## What Changed?

This is a **simplified version** of the original test suite. We removed:

### Removed:
- ❌ **jitter_tests.rs** (7 tests) - Overly complex timing measurements
  - RFC 3550 jitter calculations
  - Latency percentile analysis (p50, p95, p99)
  - First-byte latency measurements

- ❌ **Complex throughput tests** (5 tests removed)
  - Hash verification complexity
  - Multiple transfer sizes (1MB, 10MB, 100MB)
  - Detailed bandwidth metrics

- ❌ **Complex chaos engineering tests** (5 tests removed)
  - Packet loss resilience (5%, 10%, 20%)
  - Latency variance handling
  - Bandwidth throttling
  - Mixed success/failure scenarios
  - Chaos under load

### Why?

Connection quality varies significantly by peer, so stringent performance measurements aren't meaningful. Instead, we focus on:

✅ **Connectivity** - Can nodes connect?
✅ **Recoverability** - Do connections recover from failures?
✅ **Failover** - Does another connection kick in when one fails?

This results in a **leaner, more focused test suite** (37 tests vs 54 tests, 31% reduction) that tests what actually matters for a P2P proxy system.

## Troubleshooting

### Common Issues

#### Tests Timing Out

Some tests involve network operations with timeouts. If tests are timing out:

```bash
# Increase timeout with retry
cargo test -- --test-threads=1

# Run with debug output to see where it hangs
RUST_LOG=debug cargo test -- --nocapture
```

#### Port Conflicts

Tests use ephemeral ports, but conflicts can occur:

```bash
# Run tests serially (slower but avoids conflicts)
cargo test -- --test-threads=1
```

### Platform-Specific Issues

#### macOS

- **File Descriptor Limits**: macOS has lower default limits
  ```bash
  ulimit -n 4096
  cargo test
  ```

#### Linux

- **Network Namespace Isolation**: Some tests may require privileges
  ```bash
  # Run with capabilities if needed
  sudo -E cargo test
  ```

#### Windows

- **Path Length Limits**: Some test files may have long paths
  ```bash
  # Enable long path support
  git config --system core.longpaths true
  ```

### Getting Help

If you encounter issues:

1. Check test output with `--nocapture` for detailed logs
2. Use `RUST_LOG=debug` or `RUST_LOG=trace` for verbose logging
3. Run tests in isolation with `--test-threads=1`
4. Review the test source code for specific test requirements
5. Check CI logs for platform-specific failures
6. Open an issue with full error output and system information

## Contributing Tests

When adding new tests:

1. **Use the common utilities**: Leverage `common/` modules for consistency
2. **Add documentation**: Include doc comments explaining what's tested
3. **Set appropriate timeouts**: Use `tokio::time::timeout` for async operations
4. **Use deterministic seeds**: Set RNG seeds for reproducible tests
5. **Mark long tests**: Use `#[ignore]` for tests taking >1 minute
6. **Add to categories**: Place tests in the appropriate file
7. **Update this README**: Document new test categories or requirements
8. **Focus on critical paths**: Test connectivity, recovery, and failover scenarios

### Test Naming Conventions

- Prefix with `test_` for test functions
- Use descriptive names: `test_peer_rotation_failover` not `test_1`
- Group related tests: `test_socks5_handshake`, `test_socks5_ipv4`, etc.

## Test Focus Areas

This simplified test suite focuses on three core areas:

### 1. Connectivity ✅
- Can P2P connections be established?
- Do SOCKS5 proxy connections work?
- Can RPC communication be established?
- Do concurrent connections work correctly?

### 2. Recoverability ✅
- Do connections recover after disconnection?
- Is exponential backoff implemented correctly?
- Do network partitions heal?
- Are resources cleaned up after failures?

### 3. Failover ✅
- When a peer fails, does the system switch to another?
- Is peer rotation working?
- Can the system handle connection churn?
- Does resource exhaustion trigger graceful degradation?

These are the **critical paths** for a P2P proxy system. Other metrics (like precise jitter measurements) vary too much by peer to be meaningful.
