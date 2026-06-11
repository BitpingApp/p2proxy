# p2proxy

A peer-to-peer SOCKS5 proxy daemon built on [libp2p](https://libp2p.io/). Routes outbound traffic through the [Bitping](https://bitping.com/) network of distributed peer nodes instead of through a single centralised proxy provider.

- **SOCKS5** on a local port — point any application (browser, curl, Playwright, an entire WireGuard tunnel) at it.
- **Multi-peer routing** — pick by minimum bandwidth, country, or specific peer ID.
- **Stable egress IPs** — pin an ordered list of peer IDs, or let sticky mode remember the discovered exit across restarts.
- **Connection pooling** — keeps warm streams open so per-request latency stays low.
- **Prometheus metrics** for observability.
- **TUI** for live status; `--no-ui` for headless / systemd / Docker.

## Install

### Homebrew (macOS & Linux)

```sh
brew install --cask BitpingApp/tap/p2proxy
```

### Debian / Ubuntu

```sh
# x86_64
curl -LO https://github.com/BitpingApp/p2proxy/releases/latest/download/p2proxy_<version>_amd64.deb
sudo dpkg -i p2proxy_<version>_amd64.deb

# arm64
curl -LO https://github.com/BitpingApp/p2proxy/releases/latest/download/p2proxy_<version>_arm64.deb
sudo dpkg -i p2proxy_<version>_arm64.deb
```

### Fedora / RHEL

```sh
sudo rpm -i https://github.com/BitpingApp/p2proxy/releases/latest/download/p2proxy-<version>-1.x86_64.rpm
```

### Alpine

```sh
curl -LO https://github.com/BitpingApp/p2proxy/releases/latest/download/p2proxy_<version>_x86_64.apk
sudo apk add --allow-untrusted p2proxy_<version>_x86_64.apk
```

### Tarball (any Linux/macOS)

Grab the right archive for your `<os>-<arch>` from the [releases page](https://github.com/BitpingApp/p2proxy/releases) and extract:

```sh
tar -xJf p2proxy-<version>-<os>-<arch>.tar.xz
cd p2proxy-<version>-<os>-<arch>
./p2proxy --help
```

### Build from source

```sh
git clone https://github.com/BitpingApp/p2proxy.git
cd p2proxy
cargo build --release -p p2proxy
./target/release/p2proxy --help
```

Requires a recent stable Rust toolchain (`rustup install stable`).

## Quick start

1. **Get a Bitping API key.** Sign up at <https://bitping.com> and copy your key from the dashboard.

2. **Export it** (or put it in your shell rc / systemd unit):

   ```sh
   export BITPING_API_KEY=<your-key>
   ```

3. **Drop a `Config.yaml`** somewhere — the bundled one is a good starting point:

   ```yaml
   port: 45445
   log_level: info
   servers:
     - protocol: Socks5
       port: 1080
       min_bandwidth: 50Mbps
   ```

4. **Run it**:

   ```sh
   p2proxy --config Config.yaml
   ```

   You'll see the TUI come up showing peer connections. Once it picks a peer (usually within a second or two), point your client at `socks5://localhost:1080`:

   ```sh
   curl --socks5-hostname localhost:1080 https://ifconfig.me
   ```

   The IP you get back should be the peer's, not yours.

   > **Use `--socks5-hostname`, not `--socks5`.** The plain `--socks5` flag tells curl to resolve the destination locally and hand the proxy a raw IP. That IP usually points at a CDN edge close to *you*, which often doesn't route well from the peer's side, and TLS handshakes fail with `SSL_ERROR_SYSCALL`. `--socks5-hostname` (aka SOCKS5h) lets the peer do the DNS, so it gets a CDN edge close to *it*. Same applies to other clients: use `socks5h://` URLs in Python requests / Playwright proxy config / etc.

## Configuration

`Config.yaml` is YAML. The minimal example is in the file shipped with the binary; the full reference is below.

### Top-level

| Key | Type | Default | Description |
|---|---|---|---|
| `port` | u16 | `45445` | UDP port libp2p listens on. Doesn't need port-forwarding for the proxy to work, but forwarding it improves direct (non-relayed) peer connectivity. |
| `log_level` | string | `info` | One of `trace`, `debug`, `info`, `warn`, `error`. |
| `servers` | list | — | One or more proxy endpoints (below). |

### Per-server

| Key | Type | Default | Description |
|---|---|---|---|
| `protocol` | enum | — | `Socks5` (currently the only supported protocol). |
| `port` | u16 | — | Local TCP port to listen on for incoming SOCKS5 connections. |
| `min_bandwidth` | string | `0Mbps` | Minimum advertised bandwidth a peer must have to be selected. Format: `<N>{Kbps,Mbps,Gbps}`. |
| `country` | string | — | Optional ISO-3166 alpha-2 country code (e.g. `US`, `DE`, `JP`). Omit to allow any country. |
| `destination_peers` | list | — | Ordered pinned-peer preference list for stable egress IPs. Each entry is a bare peer id (preferred) or a full multiaddr ending in `/p2p/<peer-id>`. See [Pinning & sticky peers](#pinning--sticky-peers). |
| `fallback_to_discovery` | bool | `false` | When every pinned peer is offline: `false` keeps retrying the list (hard pin); `true` falls back to country/bandwidth discovery. |
| `sticky` | bool | `true` | Remember the discovered exit peer and reconnect to it across restarts for a stable egress IP. Ignored when `destination_peers` is set. |
| `destination_peer` | string | — | **Deprecated** — use `destination_peers`. A single pinned multiaddr; treated as a one-entry preference list. |
| `pool` | object | — | Optional connection-pool tuning (below). |

### Pinning & sticky peers

Both features exist for **stable egress IPs** — the exit IP your traffic appears from
only changes when the underlying node actually becomes unreachable (or its own ISP
reassigns its address; peer identity is stable, IP stability is as good as that
node's connection).

**Pinned peers** (`destination_peers`) is an *ordered preference list*:

```yaml
servers:
  - protocol: Socks5
    port: 1080
    destination_peers:
      - 12D3KooWPrimaryPeerId...      # always tried first
      - 12D3KooWBackupPeerId...       # only used while the primary is unreachable
```

- Bare peer ids are resolved to the peer's *current* route through the hub on every
  (re)connect — pinning survives the peer moving between hubs. Full multiaddrs are
  also dialed verbatim.
- **Hard pin by default**: when every listed peer is offline, p2proxy keeps retrying
  the list and surfaces an error in the TUI/logs — it never silently routes through
  an arbitrary node. Set `fallback_to_discovery: true` to prefer availability.
- The NETWORK tab shows each rank with `active` / `ok` / `STALE` health, and the
  `p2proxy_pinned_peer_resolvable{port,rank}` metric exposes the same for alerting.
- In headless mode an all-offline pinned list fails the current discovery cycle and
  retries on the next SOCKS session (JIT), matching the existing discovery behavior.

**Sticky peers** (`sticky: true`, the default for unpinned servers) gives stable IPs
*without knowing any peer ids up front*: the first discovery that matches your
`country`/`min_bandwidth` filters is remembered in `sticky_peers.json` (written next
to `node_keypair.bin`) and re-used across restarts and reconnects. When the
remembered peer is gone, discovery picks a replacement and remembers that instead —
best-effort, never blocking. Changing the server's filters (or port) invalidates the
remembered peer automatically.

The log line `sticky exit for this server is now <peer-id>` tells you what was
learned — copy that id into `destination_peers` to make the pin explicit and
permanent.

Multiple `servers` entries in one config never conflict — the store keeps one
entry per listen port. Multiple p2proxy *instances* must run from different
working directories (each owns its own `sticky_peers.json`, same as
`node_keypair.bin` — instances sharing a CWD would already share identity and
would overwrite each other's sticky state).

> **Changed behavior:** restarts no longer rotate your exit IP by default. Set
> `sticky: false` to restore the old pick-a-fresh-peer-per-restart behavior.

### Per-server connection pool

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Toggle pooling. Disabling means every SOCKS5 connection dials a fresh libp2p stream. |
| `min_idle` | u32 | `5` | Always keep at least this many warm streams open. |
| `max_total` | u32 | `30` | Hard ceiling on concurrent streams. |
| `idle_timeout_secs` | u64 | `60` | Drop streams idle longer than this. |
| `open_timeout_secs` | u64 | `20` | Give up dialing a new stream after this long. |
| `max_retries` | u32 | `3` | Retries before giving up on a failed request. |
| `max_error_rate` | float | `0.15` | Failover threshold; rotate peer when its observed error rate exceeds this fraction. |

### Environment variables

| Variable | Description |
|---|---|
| `BITPING_API_KEY` | **Required.** Your Bitping API key. Authenticates the node to the Bitping network. |
| `NO_UI` | If `true`, disable the TUI (same as `--no-ui`). Defaults to `true` in Docker. |
| `RUST_LOG` | Standard Rust tracing filter. Overrides `log_level` in `Config.yaml` if set. |

## Running headless

For systemd, Docker, or any environment without a TTY, pass `--no-ui`:

```sh
p2proxy --no-ui --config /etc/p2proxy/Config.yaml
```

### systemd

```ini
# /etc/systemd/system/p2proxy.service
[Unit]
Description=Bitping p2proxy
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=BITPING_API_KEY=<your-key>
ExecStart=/usr/local/bin/p2proxy --no-ui --config /etc/p2proxy/Config.yaml
Restart=on-failure
RestartSec=5
DynamicUser=yes
StateDirectory=p2proxy

[Install]
WantedBy=multi-user.target
```

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now p2proxy
journalctl -u p2proxy -f
```

### Docker

```sh
docker run -d --name p2proxy \
  -p 1080:1080 \
  -p 45445:45445/udp \
  -e BITPING_API_KEY=<your-key> \
  -v "$PWD/Config.yaml:/app/Config.yaml:ro" \
  ghcr.io/bitpingapp/p2proxy:latest
```

Or with Docker Compose — see the bundled `docker-compose.yml`.

## Metrics

Prometheus metrics are exposed at `http://localhost:9091/metrics` by default. Useful series:

- `p2proxy_peer_connections{state=...}` — gauge of peer connections by state.
- `p2proxy_socks5_requests_total{outcome=...}` — counter, partitioned by success / error.
- `p2proxy_bandwidth_bytes_total{direction=...}` — total bytes routed, both directions.
- `p2proxy_stream_pool_size{server=..., state=...}` — pool size by state (idle / in-use).

Scrape config:

```yaml
scrape_configs:
  - job_name: p2proxy
    static_configs:
      - targets: ["localhost:9091"]
```

## Verifying downloads

Every release includes a `_SHA256SUMS` file:

```sh
curl -LO https://github.com/BitpingApp/p2proxy/releases/download/v<version>/p2proxy-<version>_SHA256SUMS
sha256sum -c p2proxy-<version>_SHA256SUMS
```

## License

[PolyForm Shield 1.0.0](LICENSE). You can run p2proxy freely; you can't repackage it as a competing product. See `LICENSE` for the precise terms.

## Support & security

- **Bug reports and feature requests**: [GitHub Issues](https://github.com/BitpingApp/p2proxy/issues).
- **Security vulnerabilities**: see [SECURITY.md](SECURITY.md) — please email `security@bitping.com` rather than filing a public issue.
- **General questions / discussion**: [bitping.com](https://bitping.com/).
