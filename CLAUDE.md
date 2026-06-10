# CLAUDE.md

Guidance for [Claude Code](https://claude.com/claude-code) (and other AI coding agents) working in this repository.

## Project Overview

p2proxy is a peer-to-peer SOCKS5 proxy daemon built on libp2p, written in Rust. It routes outbound traffic through the Bitping network of peer nodes rather than through a centralised proxy provider.

## Workspace Structure

Cargo workspace with two crates:

- **`crates/p2proxy`** — single binary; proxy daemon + embedded ratatui TUI. The TUI runs in-process under `src/tui/`. `--no-ui` (or `NO_UI=true` env var) runs headless, which is the default in Docker.
- **`crates/models`** — shared data models, config types, event types used by `p2proxy`.

## Common Commands

### Building
```bash
# Build the single binary
cargo build --release -p p2proxy
```

### Running in Development
```bash
# Run with the TUI attached (default)
cargo run -p p2proxy

# Headless (e.g. under systemd / Docker)
cargo run -p p2proxy -- --no-ui

# Point at a Config.yaml outside the CWD
cargo run -p p2proxy -- --config ~/.config/p2proxy/Config.yaml
```

### Testing

Run all tests:
```bash
cargo test --all --verbose
```

Run specific test categories:
```bash
# Connection tests (P2P, SOCKS5, RPC) - 14 tests
cargo test --test connection_tests

# Disconnection tests (failures, cleanup) - 11 tests
cargo test --test disconnection_tests

# Throughput tests (basic bandwidth validation) - 3 tests
cargo test --test throughput_tests

# Stability tests (reconnection, failover) - 11 tests
cargo test --test stability_tests
```

Run tests with logging:
```bash
RUST_LOG=debug cargo test test_name -- --nocapture
RUST_LOG=trace cargo test -- --nocapture
```

### Test Suite Focus

The test suite is simplified and focused on three critical areas:

**✅ Connectivity** - Can nodes connect? (P2P, SOCKS5, RPC)
**✅ Recoverability** - Do connections recover from failures? (exponential backoff, network partitions)
**✅ Failover** - Does another connection kick in when one fails? (peer rotation)

Total: **39 tests** (14 connection + 11 disconnection + 3 throughput + 11 stability)

**Removed** overly complex tests for jitter/latency percentiles, chaos engineering, and long-running tests, since connection quality varies significantly by peer.

### Benchmarks

Run performance benchmarks:
```bash
# All benchmarks
cargo bench

# Specific benchmarks
cargo bench --bench throughput_bench
cargo bench --bench latency_bench

# Compile only (for CI)
cargo bench --no-run
```

View benchmark reports:
```bash
open target/criterion/report/index.html  # macOS
xdg-open target/criterion/report/index.html  # Linux
```

### Docker
```bash
# Using Docker Compose
docker-compose up -d
docker-compose logs -f
docker-compose down

# Build and run locally
docker build -t p2proxy .
docker run -p 1080:1080 -p 45445:45445/udp p2proxy
```

## Architecture

### P2P Networking (`p2proxy/src/swarm.rs`)

The core P2P functionality is built on libp2p with the following behaviors:
- **libp2p-stream**: Stream multiplexing for SOCKS5 connections, plus the typed
  request/notify protocols (FindNodes queries, bandwidth reports) that ride a
  `LibP2pClient` over the stream `Control`
- **dcutr**: Direct connection upgrade through relay
- **relay client**: Connection relay when direct connection is not possible
- **identify**: Peer identification protocol

The swarm authenticates with the Bitping gRPC service (`grpc.bitping.com`) before establishing P2P connections.

### Proxy Protocols (`p2proxy/src/proxy_protocols/`)

Currently implements SOCKS5 proxy protocol with two implementations:
- `socks.rs`: Standard SOCKS5 server
- `socks_stream.rs`: Stream-based SOCKS5 for P2P connections

### Configuration (`Config.yaml`)

The application reads configuration from `Config.yaml` with environment variable overrides. Key configuration includes:
- `port`: UDP port for libp2p (default: 45445)
- `log_level`: Logging verbosity (trace, debug, info, warn, error)
- `servers`: Array of proxy server configurations with protocol, port, country filtering, and minimum bandwidth requirements
- `bitping_api_key`: Authentication key for Bitping service (can be set via environment variable)

### Daemon ↔ TUI communication

The TUI runs **in-process** with the proxy daemon. Two `tokio::sync::mpsc` channels carry swarm events:

1. `swarm_tx` (producer: `ProxyNetwork::drive_network`) → `handle_swarm_events` mutates the shared `ServerContainer`.
2. `tui_tx` (re-emitted by `handle_swarm_events`) → `tui::Ui::run_ui` renders.

When `--no-ui` is set, the TUI receiver is dropped immediately and `try_send` becomes a cheap no-op.

### Key Dependencies

- **libp2p 0.56**: P2P networking foundation
- **tokio**: Async runtime
- **socks5-impl**: SOCKS5 protocol implementation
- **ratatui**: In-process terminal UI framework (skipped when `--no-ui`)
- **tonic**: gRPC client for Bitping authentication
- **prometheus**: Metrics exposed on port 9091

### Metrics

Prometheus metrics are exposed at `http://localhost:9091/metrics` and include connection statistics, bandwidth usage, and error rates.

### Node Identity

The application generates and persists a libp2p keypair in `node_keypair.bin` for consistent peer identity across restarts.

## Testing Infrastructure

P2Proxy has a comprehensive test suite with the following structure:

### Test Categories

1. **Connection Tests** (`crates/p2proxy/tests/connection_tests.rs`)
   - P2P connection establishment (direct and relay-mediated)
   - SOCKS5 proxy functionality
   - RPC communication between daemon and UI
   - Multiple concurrent peer connections

2. **Disconnection Tests** (`crates/p2proxy/tests/disconnection_tests.rs`)
   - Graceful disconnection and cleanup
   - Network failure handling
   - Authentication failures
   - Resource cleanup verification

3. **Throughput Tests** (`crates/p2proxy/tests/throughput_tests.rs`)
   - Bandwidth measurement accuracy (±1%)
   - Single and concurrent session performance
   - Hash verification for data integrity
   - Performance targets (>80% of direct TCP)

4. **Jitter Tests** (`crates/p2proxy/tests/jitter_tests.rs`)
   - Round-trip time (RTT) measurement
   - Latency percentiles (p50, p95, p99)
   - Jitter analysis (RFC 3550)
   - Performance targets (<100ms direct, <250ms relay)

5. **Stability Tests** (`crates/p2proxy/tests/stability_tests.rs`)
   - Long-running connection tests (24 hours, marked with `#[ignore]`)
   - Memory leak detection
   - Reconnection logic and exponential backoff
   - Stress testing and resource exhaustion

### Test Utilities

The test suite includes reusable infrastructure in `crates/p2proxy/tests/common/`:
- **fixtures.rs**: Test data generators, configurations, and keypairs
- **test_utils.rs**: Helper functions for bandwidth/latency measurement
- **mock_swarm.rs**: Mock libp2p Swarm for testing
- **mock_peer.rs**: Mock peer nodes
- **mock_relay.rs**: Mock relay servers

### Benchmarks

Performance benchmarks in `crates/p2proxy/benches/`:
- **throughput_bench.rs**: Throughput measurements
- **latency_bench.rs**: Latency measurements

All benchmarks use the Criterion framework with statistical analysis.

### CI Integration

Tests run automatically on push and pull requests:

**GitHub Actions** (`.github/workflows/test.yml`):
- Matrix testing on Ubuntu and macOS
- Cargo dependency caching for faster builds
- Standard tests run in 3-8 minutes
- Benchmark compilation (no execution in CI)
- Optional weekly schedule for long-running tests

**GitLab CI** (`.gitlab-ci.yml`):
- Parallel test execution per category
- Cargo caching for faster builds
- Optional macOS runner support
- Manual/scheduled long-running tests (48-hour timeout)
- Lint and format checks
- Release artifact generation

### Documentation

Comprehensive test documentation available at:
- **crates/p2proxy/tests/README.md**: Complete testing guide
- Includes troubleshooting, platform-specific issues, and contribution guidelines

## Release Process

The project uses `cargo-dist` for automated releases configured in `dist-workspace.toml`. The GitHub Actions workflow (`.github/workflows/release.yml`) automatically builds binaries for all platforms when a git tag is pushed:

```bash
git tag v1.0.0
git push origin v1.0.0
```

This triggers builds for:
- Linux (x86_64, ARM64)
- macOS (x86_64, ARM64)
- Windows (x86_64)
- Docker images published to Docker Hub
- Homebrew formula updates
