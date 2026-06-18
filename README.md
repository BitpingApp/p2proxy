# p2proxy

A peer-to-peer SOCKS5 proxy daemon built on [libp2p](https://libp2p.io/). It routes outbound traffic through the [Bitping](https://bitping.com/) network of distributed peer nodes instead of through a single centralised proxy provider.

- **SOCKS5** on a local port — point any application (browser, `curl`, Playwright, a whole WireGuard tunnel) at it.
- **Filtered peer selection** — pick exits by minimum bandwidth, country, or a specific peer ID.
- **Stable egress IPs** — pin an ordered list of peer IDs, or let *sticky* mode remember the discovered exit across restarts and reconnects.
- **Live TUI** for status (peers, sessions, bandwidth, the rotation pool); `--no-ui` for headless / systemd / Docker.
- **Prometheus metrics** for observability.

Architecture (ports & adapters): see [ARCHITECTURE.md](ARCHITECTURE.md).

## Install

Prebuilt archives are published on the [releases page](https://github.com/BitpingApp/p2proxy/releases) for macOS and Linux × x86_64 and aarch64.

### Tarball (macOS / Linux)

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

> Native packages (Homebrew, `deb`/`rpm`/`apk`) and a published Docker image are planned; for now use the tarball or build from source. A `Dockerfile` and `docker-compose.yml` are included for self-hosting your own image.

## Quick start

1. **Get a Bitping API key** at <https://bitping.com> (dashboard → API key).

2. **Provide it** — either in `Config.yaml` (`bitping_api_key: …`) or via the environment (the env var wins):

   ```sh
   export BITPING_API_KEY=<your-key>
   ```

3. **Write a `Config.yaml`** (the bundled one is a working starting point):

   ```yaml
   servers:
     - protocol: Socks5
       port: 1080
       min_bandwidth: 50Mbps
       country: NL          # optional; omit for any country
   ```

4. **Run it:**

   ```sh
   p2proxy --config Config.yaml
   ```

   The TUI comes up and picks a peer (usually within a second or two). Point your client at `socks5://localhost:1080`:

   ```sh
   curl --socks5-hostname localhost:1080 https://ifconfig.me
   ```

   The IP you get back should be the peer's, not yours.

   > **Use `--socks5-hostname`, not `--socks5`.** Plain `--socks5` makes curl resolve the destination locally and hand the proxy a raw IP — usually a CDN edge near *you*, which often doesn't route from the peer's side and fails TLS with `SSL_ERROR_SYSCALL`. `--socks5-hostname` (SOCKS5h) lets the peer do the DNS, so it reaches a CDN edge near *it*. Same idea elsewhere: use `socks5h://` URLs in Python `requests`, Playwright proxy config, etc.

   To eyeball the exit + check for leaks in a browser, the repo's `just p2proxy-browser` launches Firefox through the proxy with remote DNS and WebRTC disabled.

## Configuration

`Config.yaml` is YAML; every key except `servers` and `bitping_api_key` has a default. Environment variables override file values.

### Top-level

| Key | Type | Default | Description |
|---|---|---|---|
| `bitping_api_key` | string | — (**required**) | Your Bitping API key. May instead be supplied via the `BITPING_API_KEY` env var (which overrides this). |
| `servers` | list | — (**required**) | One or more proxy endpoints (below). |
| `listen_addrs` | list of `host:port` | `0.0.0.0:0` and `[::]:0` (any port) | Addresses the libp2p stack listens on (TCP + QUIC are bound per entry). Takes precedence over `port`. |
| `port` | u16 | — | Shorthand that fixes the libp2p port on the default listen addresses (e.g. `45445`). Ignored when `listen_addrs` is set. |
| `log_level` | string | `info` | Default log level when `RUST_LOG` is unset (`trace`/`debug`/`info`/`warn`/`error`, or a full directive). `RUST_LOG` overrides it. |
| `metrics_port` | u16 | `9091` | Port for the Prometheus endpoint (bound on `0.0.0.0`). |
| `bootstrap_address` | multiaddr | `/dnsaddr/boot2.bitping.com` | Bootstrap hub. Override for staging / a self-hosted hub. |
| `grpc_url` | url | `https://grpc.bitping.com` | Bitping auth service endpoint. |
| `keypair_path` | path | `node_keypair.bin` | Where the node's libp2p identity is persisted (CWD-relative). |

> Logging follows `RUST_LOG` when set (e.g. `RUST_LOG=p2proxy=debug`); otherwise the `log_level` config key above (default `info`).

### Per-server

| Key | Type | Default | Description |
|---|---|---|---|
| `protocol` | enum | — | `Socks5` (the only protocol today). |
| `port` | u16 | — | Local TCP port the SOCKS5 listener binds. |
| `min_bandwidth` | string | `50Mbps` | Minimum advertised peer bandwidth. Format `<N>{bps,Kbps,Mbps,Gbps}`. |
| `country` | string | — | Country filter. Accepts Alpha-2 (`AU`), Alpha-3 (`AUS`), or name (`Australia`) — normalised to Alpha-2. Omit for any country. |
| `destination_peers` | list | — | Ordered pinned-peer preference list (see below). |
| `fallback_to_discovery` | bool | `false` | When all pinned peers are offline: `false` keeps retrying the list; `true` falls back to country/bandwidth discovery. |
| `sticky` | bool | `true` | Remember the discovered exit in `sticky_peers.json` and reuse it across restarts. Ignored when `destination_peers` is set. |
| `sticky_reconnect` | enum | `with-backoff` | On exit-peer disconnect: `with-backoff` fights to reconnect the same peer (stored direct address, then a hub-resolved relay circuit) before rotating; `fail-fast` rotates immediately. |
| `pool` | object | — | Per-server stream tuning (below). |

### Pinning & sticky peers

Both give **stable egress IPs** — the exit your traffic appears from changes only when the node actually becomes unreachable.

**Pinned peers** (`destination_peers`) is an ordered preference list:

```yaml
servers:
  - protocol: Socks5
    port: 1080
    destination_peers:
      - 12D3KooWPrimaryPeerId...   # always tried first
      - 12D3KooWBackupPeerId...    # failover while the primary is unreachable
```

- Bare peer ids are resolved to the peer's *current* route through the hub on every (re)connect, so a pin survives the peer moving between hubs. A full multiaddr ending in `/p2p/<id>` is also dialed verbatim.
- **Hard pin by default:** when every listed peer is offline, p2proxy keeps retrying and surfaces an error rather than silently routing through an arbitrary node. Set `fallback_to_discovery: true` to prefer availability over identity.

**Sticky peers** (`sticky: true`, default for unpinned servers) gives stable IPs without naming peers up front: the first discovery matching your `country`/`min_bandwidth` is remembered in `sticky_peers.json` (next to `node_keypair.bin`) and reused across restarts and reconnects, with a small standby pool for fast failover. Changing a server's `country`/`min_bandwidth`/`port` invalidates the remembered set automatically.

> The old single `destination_peer` key was removed — use `destination_peers` (a list). Restarts no longer rotate the exit IP by default; set `sticky: false` for a fresh peer each run.

A given `Config.yaml`'s servers never conflict (the store keys by listen port), but two p2proxy *instances* must run from different working directories — each owns its own `node_keypair.bin` and `sticky_peers.json`.

### Per-server stream tuning

This is **not** a connection pool — streams are not kept warm; every SOCKS5 connection opens a fresh libp2p stream.

| Key | Type | Default | Description |
|---|---|---|---|
| `max_total` | usize | `30` | Max concurrent stream opens to a single peer; also bounds how many remembered exits the sticky pool keeps. |
| `open_timeout_secs` | u64 | `20` | Give up opening a stream after this long. |

### Flags & environment

| Flag / env | Default | Description |
|---|---|---|
| `--config <path>` / `-c`, `P2PROXY_CONFIG` | `Config.yaml` | Path to the config file. |
| `--no-ui`, `NO_UI` | off | Run headless without the TUI. |
| `BITPING_API_KEY` | — | API key (overrides `bitping_api_key` in the file). |
| `RUST_LOG` | `info` | Tracing filter. |

## Running headless

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
WorkingDirectory=/var/lib/p2proxy
ExecStart=/usr/local/bin/p2proxy --no-ui --config /etc/p2proxy/Config.yaml
Restart=on-failure
RestartSec=5
DynamicUser=yes
StateDirectory=p2proxy

[Install]
WantedBy=multi-user.target
```

`WorkingDirectory`/`StateDirectory` matter: `node_keypair.bin` and `sticky_peers.json` are written relative to the CWD.

## Metrics

Prometheus metrics are exposed on `0.0.0.0:<metrics_port>` (default `:9091`). Selected series:

- `p2proxy_peers_connected` — gauge of connected libp2p peers.
- `p2proxy_sessions_active` — gauge of in-flight SOCKS sessions.
- `p2proxy_socks_connections_total`, `p2proxy_sessions_initialized_total` — accepted / fully-established sessions.
- `p2proxy_upload_bytes_total`, `p2proxy_download_bytes_total` — bytes relayed each direction.
- `p2proxy_session_errors_total{stage}` — session failures by stage (`handshake` / `request` / `peer-connection` / `data-transfer`).
- `p2proxy_stream_opened_total`, `p2proxy_stream_acquire_failed_total`, `p2proxy_stream_acquire_duration_seconds` — peer-stream opens.
- `p2proxy_ping_rtt_seconds`, `p2proxy_ping_failures_total` — peer liveness.
- `p2proxy_socks_jit_discovery_total`, `p2proxy_socks_rejected_no_peer_total` — discovery pressure.

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

[PolyForm Shield 1.0.0](LICENSE) — run p2proxy freely; don't repackage it as a competing product. See `LICENSE` for the exact terms.

## Support & security

- **Bugs / features:** [GitHub Issues](https://github.com/BitpingApp/p2proxy/issues).
- **Security:** see [SECURITY.md](SECURITY.md) — email `security@bitping.com` rather than filing a public issue.
- **Questions:** [bitping.com](https://bitping.com/).
