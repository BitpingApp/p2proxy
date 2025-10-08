# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

P2Proxy is a peer-to-peer proxy system built with Rust that enables secure and decentralized network communication over the Bitping Network. The system uses libp2p for P2P networking and provides SOCKS5 proxy functionality.

## Workspace Structure

This is a Cargo workspace with three main crates:

- **`crates/p2proxy`**: Core proxy daemon that handles P2P networking, authentication, and proxy functionality
- **`crates/ui`**: Terminal-based user interface for monitoring and managing the proxy (Alpha)
- **`crates/models`**: Shared data models and RPC traits used across both binaries

## Common Commands

### Building
```bash
# Build all binaries
cargo build --release

# Build specific binary
cargo build --release --bin p2proxy
cargo build --release --bin ui
```

### Running in Development
```bash
# Run the proxy daemon
cargo run --bin p2proxy

# Run the UI (in a separate terminal)
cargo run --bin ui
```

### Testing
```bash
cargo test
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
- **libp2p-stream**: Stream multiplexing for SOCKS5 connections
- **dcutr**: Direct connection upgrade through relay
- **relay client**: Connection relay when direct connection is not possible
- **identify**: Peer identification protocol
- **request-response**: For bandwidth reporting and peer queries

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

### RPC Communication

The `models` crate defines RPC traits using the `remoc` library for inter-process communication between the proxy daemon and UI:
- `Counter` trait: Provides methods to query server states, connection status, statistics, and event streams
- Communication happens over TCP on port 9876 (localhost)

### Key Dependencies

- **libp2p 0.55**: P2P networking foundation
- **tokio**: Async runtime
- **socks5-impl**: SOCKS5 protocol implementation
- **ratatui**: Terminal UI framework
- **remoc**: Remote trait invocation for RPC
- **tonic**: gRPC client for Bitping authentication
- **prometheus**: Metrics exposed on port 9091

### Metrics

Prometheus metrics are exposed at `http://localhost:9091/metrics` and include connection statistics, bandwidth usage, and error rates.

### Node Identity

The application generates and persists a libp2p keypair in `node_keypair.bin` for consistent peer identity across restarts.

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
