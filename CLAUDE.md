# CLAUDE.md

Guidance for [Claude Code](https://claude.com/claude-code) (and other AI coding agents) working in this repository.

## Project Overview

p2proxy is a peer-to-peer SOCKS5 proxy daemon built on libp2p, written in Rust. It routes outbound traffic through the Bitping network of peer nodes rather than through a centralised proxy provider. An in-process ratatui TUI shows live status; `--no-ui` (or `NO_UI=true`, the default in Docker) runs headless.

## Workspace Structure

Hexagonal (ports-and-adapters) with an actor runtime. Two crates:

- **`crates/core`** (package `p2proxy-core`, lib `proxy_core`) — the pure domain. Plain logic, value types, and the port traits. **No** `tokio` runtime, `libp2p-swarm`, `tonic`, `ratatui`, `metrics`, or globals — they aren't dependencies, so the core literally can't reach the outside. Holds `domain/` (`connect`, `selection`, `sticky`, `backoff`, `circuit`), `ports/` (the trait boundaries + each one's error type), `config`, `events`, and in-memory `testing` fakes.
- **`crates/p2proxy`** — the binary. The only place the `Swarm`, gRPC, files, ratatui, and the tokio runtime appear. Holds the port **adapters** (`adapters/`), the **actors + runtime** (`runtime/`), the TUI (`tui/`), and the composition root (`main.rs`).

See **[ARCHITECTURE.md](ARCHITECTURE.md)** for the ports/adapters diagram and the full mapping table.

## Code Style & Conventions

**Code documents itself; comments are the rare exception.** Prefer well-named
functions, types, variables, files, and folder paths over comments. A comment
exists *only* to capture a *why* that isn't apparent from reading the code — a
non-obvious constraint, a protocol/ordering subtlety, a deliberate trade-off.
Never restate *what* the code does, and never narrate *history* (`// was X`,
`// replaces old Y`, `// previously…`, `// no longer needed`) — the diff and git
history carry that story.

**Names and structure are the documentation.** Functions, variables, types,
files, and folder paths must be self-explanatory, so the module tree reads as the
design (`runtime/network/classify.rs`, `ConnectPlan::next_action`,
`send_session_error`). If something needs a comment to explain *what* it is,
rename it instead.

Other standing rules (shared with the monorepo-root CLAUDE.md):

- No file over 1000 lines (target well under 600); `mod.rs` is a table of
  contents, not a body — split by purpose into named files.
- Never `.unwrap()`/`.expect()` in production code; use `?`, typed errors,
  `ok_or`, `unwrap_or`, or pattern matching. Tests may panic.
- Prefer typed `thiserror` enums over `anyhow`/`eyre` in libraries and reusable
  code so callers can `matches!`; reserve `eyre` for the binary entry point.
- Keep each error type next to the trait that exposes it or the code that throws
  it — not in a central `errors` module.
- Prefer message passing / actor-owned state / `ArcSwap` over `Mutex`/`RwLock`.
- Prefer static dispatch (generics / associated types / RPITIT) over `Box<dyn>`;
  async traits via `impl Future`, not `async_trait`.
- A port earns its place only when there's something to *adapt* (a real backend
  plus a fake, or genuine swap value). Otherwise keep the concrete type — the
  crate boundary already gives the isolation.
- Always invert conditionals: check errors and guard conditions first and
  early-return on them explicitly (`?`, `return`, `continue`, `let ... else`).
  The happy path comes last, at the function's base indentation — never nested
  inside an `if`. Invert the condition instead of wrapping success in braces.

## Architecture

Dependencies point strictly inward. The binary depends on `proxy_core`;
`proxy_core` depends on nothing external. Decision logic is pure and generic over
the port traits; the binary supplies the real adapters, tests supply in-memory
fakes.

**Ports** (`core::ports`, RPITIT async + static dispatch): `PeerDirectory`,
`Dialer`, `StreamOpener`, `StickyStore`, `Authenticator`, `Identity`, `Clock`,
`EventSink`, plus the one `Actor` trait (`handle(&mut self, ctx, event)`).

**Adapters** (`p2proxy::adapters`): `SwarmGateway` (`PeerDirectory`+`Dialer`),
`PeerStreamManager` (`StreamOpener`), `FileStickyStore` (`StickyStore` →
`sticky_peers.json`), `GrpcAuth` (`Authenticator` → `grpc.bitping.com`),
`KeypairIdentity` (`Identity` → `node_keypair.bin`), `TokioClock`, `ChannelSink`
(`EventSink` → TUI / Prometheus).

**Actors** (`p2proxy::runtime`): exactly two implement `Actor`, because they're
the only single-owner-mutable-state holders —
- `NetworkActor` owns the libp2p `Swarm` and runs its own bespoke loop,
  `NetworkActor::run(self, …)`, polling the swarm stream + command inbox and
  feeding each into `handle`.
- `DiscoveryActor<S: StickyStore>` runs the pure `connect` flow, owns the sticky
  store + per-port destination `ArcSwap`s, and reacts to peer close / unusable.

`Runtime::spawn` wires them: the bespoke `NetworkActor::run` for the swarm owner,
the generic `drive` for any channel-driven actor (the discovery slot is generic
over the `Actor` trait). `SessionSupervisor` (one per listen port) is the SOCKS
accept loop that spawns a per-connection relay task reading the destination
`ArcSwap`; `PeerStreamManager` is a shared adapter — neither is an `Actor`, by
design (streaming / shared-resource shaped, not message-handler shaped).

## Configuration (`Config.yaml`)

YAML; environment variables override file values. Top-level: `bitping_api_key`
(or the `BITPING_API_KEY` env var), `servers`, `listen_addrs` (host:port list;
default `0.0.0.0:0` + `[::]:0`; the `port` shorthand fixes that default port),
`metrics_port` (default 9091), `log_level`, `bootstrap_address`, `grpc_url`,
`keypair_path`. Per-server: `protocol`
(`Socks5`), `port`, `min_bandwidth` (default 50Mbps), `country`,
`destination_peers` (ordered pinned list), `fallback_to_discovery`, `sticky`,
`sticky_reconnect` (`with-backoff` | `fail-fast`), `pool { max_total,
open_timeout_secs }`. Logging follows `RUST_LOG`, falling back to the `log_level`
config key. Full reference in [README.md](README.md).

`node_keypair.bin` (libp2p identity) and `sticky_peers.json` (remembered exit
pool, fingerprinted by `country`/`min_bandwidth`/port) are written relative to
the CWD; run separate instances from separate working directories.

## Common Commands

```bash
cargo build --release -p p2proxy            # build the binary
cargo run -p p2proxy                         # run with the TUI
cargo run -p p2proxy -- --no-ui              # headless
cargo run -p p2proxy -- --config path.yaml   # config outside CWD
cargo fmt --all && cargo clippy --all
```

## Testing

Real production code against in-memory fakes — no mock-to-mock.

```bash
cargo test -p p2proxy-core # fast pure-domain suite: connect, selection, sticky,
                           # backoff, circuit, per-port error conversions
cargo test -p p2proxy      # adapters + actors over an in-memory libp2p
                           # MemoryTransport swarm + a loopback SOCKS socket
```

The data-relay loop, bootstrap, and `PeerStreamManager::open` are validated by a
live smoke run (`curl --socks5-hostname localhost:1080 https://ifconfig.me`
returns a peer's egress IP), not unit-covered — `ProxySession` holds a concrete
libp2p `Stream` that can't be faked without a full stream pair.

## Metrics

Prometheus on `0.0.0.0:<metrics_port>` (default `:9091`). Useful series:
`p2proxy_peers_connected`, `p2proxy_sessions_active`,
`p2proxy_socks_connections_total`, `p2proxy_upload_bytes_total` /
`p2proxy_download_bytes_total`, `p2proxy_session_errors_total{stage}`,
`p2proxy_stream_opened_total`, `p2proxy_ping_rtt_seconds`. Full list in README.

## Release

p2proxy is built and released by the **monorepo GitLab CI**, not from this
directory. Trigger via GitLab UI → Run pipeline with
`P2PROXY_RELEASE_VERSION=X.Y.Z` (semver, no `v`; append `-rcN` for a prerelease).
The pipeline bumps `crates/p2proxy/Cargo.toml`, refreshes the lock, subtree-pushes
`customer/p2proxy/` → `github.com/BitpingApp/p2proxy`, cross-compiles macOS +
Linux × x86_64 + aarch64, and publishes a GitHub Release. Pipeline config lives at
`customer/.release/p2proxy.gitlab-ci.yml` + `customer/.shared-ci.yml`; full
details are in the monorepo-root CLAUDE.md under "Releasing a customer app". Do
**not** build releases from this directory — the workspace path deps need the
full monorepo.
