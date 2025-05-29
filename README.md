# P2Proxy

A peer-to-peer proxy system built with Rust that enables secure and decentralized network communication over the Bitping Network.

## Features

- **Peer-to-peer networking** using libp2p
- **SOCKS5 proxy support** for seamless integration
- **Cross-platform compatibility** (Linux, macOS, Windows)
- **Multiple architectures** (x86_64, ARM64)
- **Terminal UI** for monitoring and management *(Alpha - subject to changes)*
- **Prometheus metrics** for observability
- **Secure authentication** and encrypted communication
- **Docker support** for easy deployment
- **Professional package distribution** via cargo-dist

## Architecture

P2Proxy consists of two main components:

- **`p2proxy`**: The core proxy daemon that handles P2P networking and proxy functionality
- **`ui`**: A terminal-based user interface for monitoring and managing the proxy *(Alpha version)*

## Installation

### Homebrew (macOS/Linux)

```bash
# Add the Bitping tap
brew tap BitpingApp/homebrew-tap

# Install p2proxy
brew install p2proxy

# Install the UI (optional)
brew install ui
```

### Shell Installer (Cross-platform)

```bash
# Install p2proxy
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/BitpingApp/p2proxy/releases/latest/download/p2proxy-installer.sh | sh

# Install UI
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/BitpingApp/p2proxy/releases/latest/download/ui-installer.sh | sh
```

### PowerShell (Windows)

```powershell
# Install p2proxy
powershell -ExecutionPolicy ByPass -c "irm https://github.com/BitpingApp/p2proxy/releases/latest/download/p2proxy-installer.ps1 | iex"

# Install UI
powershell -ExecutionPolicy ByPass -c "irm https://github.com/BitpingApp/p2proxy/releases/latest/download/ui-installer.ps1 | iex"
```

### Docker (Recommended)

#### Using Docker Compose

The easiest way to run P2Proxy is with Docker Compose:

```bash
# Clone the repository (or just download docker-compose.yml and Config.yaml)
git clone <repository-url>
cd p2proxy

# Start the service
docker-compose up -d

# View logs
docker-compose logs -f

# Stop the service
docker-compose down
```

#### Using Docker CLI

Pull and run the latest Docker image:

```bash
# Pull the image
docker pull bitping/p2proxy:latest

# Run with default configuration
docker run -d --name p2proxy \
  -p 1080:1080 \
  -p 45445:45445/udp \
  bitping/p2proxy:latest

# Run with custom configuration
docker run -d --name p2proxy \
  -p 1080:1080 \
  -p 45445:45445/udp \
  -v /path/to/your/Config.yaml:/app/Config.yaml \
  bitping/p2proxy:latest
```

### Pre-built Binaries

Download the latest release for your platform from the [Releases](https://github.com/BitpingApp/p2proxy/releases) page:

- **Linux x86_64**: `p2proxy-x86_64-unknown-linux-gnu.tar.xz`
- **Linux ARM64**: `p2proxy-aarch64-unknown-linux-gnu.tar.xz`
- **macOS x86_64**: `p2proxy-x86_64-apple-darwin.tar.xz`
- **macOS ARM64**: `p2proxy-aarch64-apple-darwin.tar.xz`
- **Windows x86_64**: `p2proxy-x86_64-pc-windows-msvc.zip`

Extract the archive and run the binaries:

```bash
# Linux/macOS
tar -xf p2proxy-x86_64-unknown-linux-gnu.tar.xz
cd p2proxy-x86_64-unknown-linux-gnu
./p2proxy

# Windows
# Extract p2proxy-x86_64-pc-windows-msvc.zip
p2proxy.exe
```

### Building from Source

#### Prerequisites

- [Rust](https://rustup.rs/) (latest stable version)
- Git with SSH access to the required repositories

#### Build Instructions

```bash
# Clone the repository
git clone https://github.com/BitpingApp/p2proxy
cd p2proxy

# Build all binaries
cargo build --release

# Build specific binary
cargo build --release --bin p2proxy
cargo build --release --bin ui
```

## Usage

### Configuration

P2Proxy uses a YAML configuration file (`Config.yaml`). The default configuration includes:

```yaml
port: 45445

# Logging configuration
log_level: info

servers:
  - protocol: Socks5
    port: 1080
    min_bandwidth: 70Mbps
```

**Important Notes:**
- The `port` specified at the root of `Config.yaml` (default: 45445) does **not** need to be port forwarded for basic operation
- However, if you're experiencing connectivity issues with other peers, port forwarding this port may help improve connectivity
- The logging level can be set to: `trace`, `debug`, `info`, `warn`, or `error`

### Running P2Proxy

#### Using Docker Compose

```bash
# Start the service
docker-compose up -d

# View logs
docker-compose logs -f p2proxy

# Stop the service
docker-compose down
```

#### Using Docker

```bash
# Start the container
docker run -d --name p2proxy \
  -p 1080:1080 \
  -p 45445:45445/udp \
  bitping/p2proxy:latest

# View logs
docker logs p2proxy

# Stop the container
docker stop p2proxy
```

#### Using Binaries

1. **Start the proxy daemon**:
   ```bash
   ./p2proxy
   ```

2. **Launch the UI** (in a separate terminal) *(Alpha version - subject to changes)*:
   ```bash
   ./ui
   ```

3. **Configure your applications** to use the SOCKS5 proxy (typically on port 1080)

### Command Line Options

```bash
# P2Proxy daemon
./p2proxy --help

# UI application
./ui --help
```

## Development

### Project Structure

```
p2proxy/
├── crates/
│   ├── models/          # Shared data models
│   ├── p2proxy/         # Main proxy daemon
│   └── ui/              # Terminal user interface
├── Config.yaml          # Configuration file
├── Cargo.toml          # Workspace configuration
├── dist-workspace.toml  # cargo-dist configuration
└── .github/
    └── workflows/
        └── release.yml  # CI/CD pipeline
```

### Dependencies

Key dependencies include:

- **libp2p**: Peer-to-peer networking
- **tokio**: Async runtime
- **socks5-impl**: SOCKS5 proxy implementation
- **ratatui**: Terminal UI framework
- **tracing**: Logging and observability
- **prometheus**: Metrics collection

### Running Tests

```bash
cargo test
```

### Development Setup

1. Clone the repository with SSH access
2. Install Rust and required tools
3. Build and run in development mode:
   ```bash
   cargo run --bin p2proxy
   cargo run --bin ui
   ```

### Releases

This project uses [cargo-dist](https://opensource.axo.dev/cargo-dist/) for automated releases. To create a new release:

1. Update version numbers in `Cargo.toml` files
2. Update `CHANGELOG.md`
3. Create and push a git tag:
   ```bash
   git tag v1.0.0
   git push origin v1.0.0
   ```

The CI will automatically:
- Build binaries for all platforms
- Create GitHub releases
- Publish to Homebrew
- Generate installers and checksums

## Monitoring

P2Proxy includes built-in Prometheus metrics for monitoring:

- Connection statistics
- Bandwidth usage
- Peer information
- Error rates

Access metrics at `http://localhost:9090/metrics` (configurable).

## Security

- All peer-to-peer communication is encrypted
- Authentication is required for proxy access
- Network traffic is routed through secure channels
- Private keys are stored securely

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Support

For issues and questions:

- Create an issue on GitHub
- Check the documentation
- Review existing issues for solutions

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for version history and changes.