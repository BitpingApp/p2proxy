# P2Proxy Test Suite

This directory contains comprehensive tests for P2Proxy covering connection handling, disconnection scenarios, throughput measurement, latency/jitter analysis, and long-term stability.

## Table of Contents

- [Test Structure](#test-structure)
- [Running Tests](#running-tests)
  - [Quick Start](#quick-start)
  - [Running Specific Test Categories](#running-specific-test-categories)
  - [Running Individual Tests](#running-individual-tests)
  - [Running Benchmarks](#running-benchmarks)
  - [Running Long-Running Tests](#running-long-running-tests)
- [Test Categories](#test-categories)
- [Expected Durations](#expected-durations)
- [Troubleshooting](#troubleshooting)

## Test Structure

```
tests/
├── common/                      # Shared test utilities and mocks
│   ├── mod.rs                   # Common module exports
│   ├── platform.rs              # Platform-specific test helpers
│   ├── fixtures.rs              # Test fixtures and configurations
│   ├── test_utils.rs            # Helper functions and utilities
│   ├── mock_swarm.rs            # Mock libp2p Swarm for testing
│   ├── mock_peer.rs             # Mock peer node implementation
│   └── mock_relay.rs            # Mock relay server
├── connection_tests.rs          # Connection establishment tests
├── disconnection_tests.rs       # Disconnection and failure tests
├── throughput_tests.rs          # Bandwidth and performance tests
├── jitter_tests.rs              # Latency and jitter analysis
├── stability_tests.rs           # Long-running stability tests
├── fixtures_test.rs             # Tests for fixture utilities
├── test_utils_test.rs           # Tests for test utilities
└── example_usage.rs             # Example test patterns

benches/
├── throughput_bench.rs          # Throughput benchmarks
└── latency_bench.rs             # Latency benchmarks
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

#### Connection Tests
Test P2P connection establishment, SOCKS5 proxy functionality, and RPC communication:
```bash
cargo test --test connection_tests
```

With detailed output:
```bash
cargo test --test connection_tests -- --nocapture
```

#### Disconnection Tests
Test graceful disconnections, network failures, and error handling:
```bash
cargo test --test disconnection_tests
```

#### Throughput Tests
Test bandwidth measurement, byte counting accuracy, and performance:
```bash
cargo test --test throughput_tests
```

#### Jitter Tests
Test latency characteristics and timing consistency:
```bash
cargo test --test jitter_tests
```

#### Stability Tests
Test long-running connections and system stability (most tests are marked with `#[ignore]`):
```bash
# Run only quick stability tests
cargo test --test stability_tests

# Run long-running stability tests (can take hours)
cargo test --test stability_tests -- --ignored --nocapture
```

### Running Individual Tests

Run a specific test by name:
```bash
cargo test test_p2p_direct_connection
```

Run tests matching a pattern:
```bash
# Run all SOCKS5-related tests
cargo test socks5

# Run all bandwidth-related tests
cargo test bandwidth
```

Run with debug logging:
```bash
RUST_LOG=debug cargo test test_p2p_direct_connection -- --nocapture
```

Run with trace-level logging (very verbose):
```bash
RUST_LOG=trace cargo test test_p2p_direct_connection -- --nocapture
```

### Running Benchmarks

Benchmarks use the Criterion framework for statistical analysis.

Run all benchmarks:
```bash
cargo bench
```

Run specific benchmark suite:
```bash
# Throughput benchmarks
cargo bench --bench throughput_bench

# Latency benchmarks
cargo bench --bench latency_bench
```

Run benchmarks matching a pattern:
```bash
cargo bench throughput
```

Compile benchmarks without running (useful for CI):
```bash
cargo bench --no-run
```

#### Benchmark Output

Criterion generates detailed HTML reports in `target/criterion/`:
- Statistical analysis of performance
- Comparison with previous runs
- Performance regression detection
- Violin plots and histograms

View reports:
```bash
open target/criterion/report/index.html  # macOS
xdg-open target/criterion/report/index.html  # Linux
```

### Running Long-Running Tests

Some stability tests are marked with `#[ignore]` because they take hours to complete. These tests validate long-term behavior like 24-hour connection stability and memory leak detection.

Run all ignored (long-running) tests:
```bash
cargo test -- --ignored --nocapture
```

Run specific long-running test:
```bash
cargo test test_24hour_connection_stability -- --ignored --nocapture
```

**Note**: Long-running tests are designed to run overnight or in CI on a schedule. They are not part of the normal test suite.

## Test Categories

### Connection Tests (`connection_tests.rs`)

Tests covering connection establishment and basic functionality:

- **P2P Connection Tests**
  - Direct peer-to-peer connections
  - Relay-mediated connections
  - Multiple concurrent peer connections
  - Connection upgrade (relay → direct via DCUtR)

- **SOCKS5 Proxy Tests**
  - SOCKS5 handshake protocol
  - Connection request handling (IPv4, IPv6, domain names)
  - Multiple concurrent proxy sessions
  - Session lifecycle management

- **RPC Communication Tests**
  - Daemon-to-UI RPC connection
  - Method invocation (get_server_states, get_stats, etc.)
  - Event streaming
  - Multiple client handling

**Key Validations**:
- Connection events are properly emitted
- Peer tracking is accurate
- Protocol exchanges complete successfully
- Concurrent sessions don't interfere

### Disconnection Tests (`disconnection_tests.rs`)

Tests covering clean and abrupt disconnection scenarios:

- **Graceful Disconnection**
  - Peer disconnection with cleanup
  - Shutdown during active sessions
  - Client disconnect handling
  - Resource cleanup verification

- **Network Failures**
  - Sudden peer unavailability
  - Network partitions
  - Relay failures and fallback
  - Partial transfer failures

- **Authentication Failures**
  - Invalid API key handling
  - Token expiration
  - gRPC service errors
  - Retry logic with backoff

**Key Validations**:
- No panics during failure scenarios
- Resources are properly cleaned up
- Error events are correctly emitted
- Recovery mechanisms work as expected

### Throughput Tests (`throughput_tests.rs`)

Tests covering bandwidth measurement and performance:

- **Bandwidth Measurement**
  - Accurate byte counting (upload/download)
  - Bandwidth metrics accuracy
  - Hash verification for data integrity
  - Tolerance: ±1% accuracy

- **Performance Tests**
  - Single session maximum throughput
  - Concurrent session throughput
  - Large file transfers (1GB+)
  - Sustained performance over time

- **Configuration Compliance**
  - Minimum bandwidth requirements
  - Peer selection based on bandwidth
  - Dynamic configuration updates

**Key Validations**:
- Byte counts match transferred data within 1%
- Hash verification prevents data corruption
- Prometheus metrics match actual transfers
- Performance meets baseline targets (>80% of direct TCP)

### Jitter Tests (`jitter_tests.rs`)

Tests covering latency and timing characteristics:

- **Latency Measurement**
  - Round-trip time (RTT) measurement
  - Request-response latency
  - First-byte latency
  - Percentile calculations (p50, p95, p99)

- **Jitter Analysis**
  - Packet timing variance
  - Jitter under different load conditions
  - Clock synchronization verification
  - RFC 3550 jitter calculation

- **Performance Targets**
  - Direct connections: <100ms RTT
  - Relay connections: <250ms RTT
  - Jitter: <10ms for stable connections

**Key Validations**:
- Latency measurements accurate within 5ms
- Jitter calculations match RFC standards
- Performance degrades predictably under load
- Timing is consistent across test runs

### Stability Tests (`stability_tests.rs`)

Tests covering long-term stability and reliability (many marked with `#[ignore]`):

- **Long-Running Sessions**
  - 24-hour connection stability
  - 6+ hour data transfers
  - Idle connection maintenance
  - Memory leak detection

- **Reconnection Logic**
  - Exponential backoff verification
  - Session restoration after disconnect
  - Peer rotation and failover
  - Configuration-based failover

- **Stress Tests**
  - High connection churn
  - Rapid session turnover
  - Resource exhaustion handling
  - Network chaos testing

**Key Validations**:
- Connections remain stable for 24+ hours
- Memory growth <10% over 24 hours
- CPU usage <5% when idle
- Automatic recovery from failures

## Expected Durations

Test suite execution times (on modern hardware):

| Test Suite | Duration | Notes |
|------------|----------|-------|
| All tests (excluding ignored) | 2-5 minutes | Standard test run |
| Connection tests | 30-60 seconds | Fast, mostly unit tests |
| Disconnection tests | 30-60 seconds | Includes timeout tests |
| Throughput tests | 1-2 minutes | Data transfer tests |
| Jitter tests | 1-2 minutes | Timing measurements |
| Stability tests (non-ignored) | 30 seconds | Quick stability checks |
| Long-running tests (--ignored) | 6-24 hours | Overnight/weekend tests |
| All benchmarks | 5-10 minutes | Statistical analysis |

**CI Performance**:
- Full test suite: 3-8 minutes (with caching)
- Benchmark compilation: 2-5 minutes
- Long-running tests: Run weekly on schedule

## CI Integration

Tests are automatically run on every push and pull request via both GitHub Actions and GitLab CI.

### GitHub Actions (`.github/workflows/test.yml`)

- **Matrix Testing**: Tests run on both Ubuntu and macOS
- **Dependency Caching**: Cargo dependencies are cached for faster builds (3-8 minute test runs)
- **Test Execution**: All standard tests run with verbose output and backtraces enabled
- **Benchmark Validation**: Benchmarks are compiled but not executed in CI
- **Long-Running Tests**: Optional weekly schedule for long-running stability tests

### GitLab CI (`.gitlab-ci.yml`)

- **Parallel Execution**: Test categories run in parallel for faster feedback
- **Cargo Caching**: Dependencies cached for faster builds
- **Multi-Platform**: Supports Linux and macOS runners (if available)
- **Long-Running Tests**: Manual trigger or scheduled with 48-hour timeout
- **Lint Checks**: Includes cargo fmt and clippy checks
- **Release Artifacts**: Generates release binaries on master/main branch

### Triggering Long-Running Tests in CI

**GitHub Actions:**
1. Include `[run-long-tests]` in your commit message
2. Manually trigger the workflow with the long-running tests option
3. Wait for the weekly scheduled run

**GitLab CI:**
1. Include `[run-long-tests]` in your commit message
2. Manually trigger the `test:long-running` job from the pipeline UI
3. Wait for the scheduled pipeline run

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

#### Mock Initialization Failures

If mock components fail to initialize:

```bash
# Check for detailed error messages
RUST_BACKTRACE=full cargo test -- --nocapture

# Run specific test in isolation
cargo test test_name -- --exact
```

#### Benchmark Variability

Benchmarks can be affected by system load:

```bash
# Run benchmarks with more samples for accuracy
cargo bench -- --sample-size 100

# Disable CPU frequency scaling (Linux)
sudo cpupower frequency-set --governor performance
```

#### Memory Leak Detection

Long-running tests check for memory leaks:

```bash
# Run with verbose memory tracking
RUST_LOG=trace cargo test test_24hour_connection_stability -- --ignored --nocapture

# Use external tools for detailed analysis
valgrind --leak-check=full cargo test test_name
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

### Debugging Test Failures

Enable detailed test output:
```bash
# Full stack traces
RUST_BACKTRACE=full cargo test -- --nocapture

# Test-specific logging
RUST_LOG=p2proxy=trace cargo test test_name -- --nocapture

# Run a single test in verbose mode
cargo test test_name -- --exact --nocapture
```

## Contributing Tests

When adding new tests:

1. **Use the common utilities**: Leverage `common/` modules for consistency
2. **Add documentation**: Include doc comments explaining what's tested
3. **Set appropriate timeouts**: Use `tokio::time::timeout` for async operations
4. **Use deterministic seeds**: Set RNG seeds for reproducible tests
5. **Mark long tests**: Use `#[ignore]` for tests taking >1 minute
6. **Add to categories**: Place tests in the appropriate file
7. **Update this README**: Document new test categories or requirements

### Test Naming Conventions

- Prefix with `test_` for test functions
- Use descriptive names: `test_p2p_direct_connection` not `test_1`
- Group related tests: `test_socks5_handshake`, `test_socks5_ipv4`, etc.
- Mark benchmark functions with `bench_` prefix

### Test Organization

- **Unit tests**: Test individual functions/modules
- **Integration tests**: Test component interactions
- **End-to-end tests**: Test full workflows
- **Benchmarks**: Performance measurements
- **Stability tests**: Long-running validation

## Additional Resources

- [Rust Testing Guide](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [Criterion Benchmarking](https://bheisler.github.io/criterion.rs/book/)
- [Tokio Testing](https://tokio.rs/tokio/topics/testing)
- [SPRINT.md](../../../SPRINT.md) - Original test planning document
