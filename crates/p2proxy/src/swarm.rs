use std::sync::Arc;
use std::{sync::LazyLock, time::Duration};

use crate::proxy_protocols::socks_stream::{DataDirection, SocksStreamMessage};
use crate::stream_pool::{PoolConfig, StreamPool};
use crate::utils::wait_ext::SwarmWaitExt;
use crate::CONFIG;
use crate::{proxy_protocols, GRPC_CHANNEL};
use bitping_swarm::auth::Auth;
use color_eyre::eyre::{bail, eyre, Context, Result};
use color_eyre::owo_colors::OwoColorize;
use futures::StreamExt;
use libp2p::Multiaddr;
use libp2p::{
    dcutr, identify,
    identity::{Keypair, PublicKey},
    multiaddr::{self, Protocol},
    noise, ping, relay,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, PeerId, Swarm,
};
use libp2p_stream as stream;
use metrics::{counter, gauge};
use p2p_bandwidth_protocol::bandwidth_reporter::AuthedBandwidthReport;
use p2p_protocol::{client::LibP2pClient, P2pClient};
use models::config::Server;
use models::events::Events;
use models::{Counter, ServerContainer, ServerState};
use protocols::auth::v1::{
    authentication_service_client::AuthenticationServiceClient, FederatedApiTokenAuthRequest,
};
use sha2::Digest;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::RwLock;
use tonic::codec::CompressionEncoding;
use tracing::{debug, info, warn};

#[derive(NetworkBehaviour)]
pub(crate) struct Behaviour {
    stream: stream::Behaviour,
    dcutr: dcutr::Behaviour,
    relay: relay::client::Behaviour,
    identify: identify::Behaviour,
    /// libp2p liveness probe. Without this, dead peers stay in the
    /// rotation pool for hours because libp2p only notices via
    /// transport-level errors (TCP RST, QUIC idle ~30s) which don't
    /// fire on silent network drops. With ping at 15s/10s, a peer
    /// whose process is alive but unreachable gets booted within
    /// ~30 s of failure — fed into our normal `ConnectionClosed →
    /// PeerDisconnected → rediscovery` flow.
    ping: ping::Behaviour,
}

impl Behaviour {
    fn new(local_pubkey: PublicKey, relay: relay::client::Behaviour) -> Self {
        Self {
            stream: stream::Behaviour::new(),
            dcutr: dcutr::Behaviour::new(local_pubkey.to_peer_id()),
            relay,
            identify: identify::Behaviour::new(identify::Config::new(
                "bitping-federated/1.0.0".into(),
                local_pubkey,
            )),
            ping: ping::Behaviour::new(
                ping::Config::new()
                    .with_interval(Duration::from_secs(15))
                    .with_timeout(Duration::from_secs(10)),
            ),
        }
    }
}

pub static KEYPAIR: LazyLock<Keypair> = LazyLock::new(|| {
    // Try to read keypair from file
    let keypair_path = std::path::Path::new("node_keypair.bin");

    if keypair_path.exists() {
        // Load keypair from file
        match std::fs::read(keypair_path) {
            Ok(bytes) => match libp2p::identity::Keypair::from_protobuf_encoding(&bytes) {
                Ok(keypair) => {
                    debug!("Loaded existing keypair from disk");
                    return keypair;
                }
                Err(e) => {
                    warn!("Error deserializing keypair: {}, generating new one", e);
                }
            },
            Err(e) => {
                warn!("Error reading keypair file: {}, generating new one", e);
            }
        }
    }

    // Generate new keypair if we couldn't load one
    let keypair = libp2p::identity::Keypair::generate_ed25519();

    // Save the new keypair to disk
    match keypair.to_protobuf_encoding() {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(keypair_path, &bytes) {
                warn!("Failed to save keypair to disk: {}", e);
            } else {
                debug!("Generated and saved new keypair to disk");
            }
        }
        Err(e) => {
            info!("Failed to serialize keypair: {}", e);
        }
    }

    keypair
});

pub struct ProxyNetwork<T>(T);
pub struct AuthStep;
pub struct NetworkConnect {
    pub token: String,
}

pub struct Bootstrapped {
    token: String,
    swarm: Swarm<Behaviour>,
    event_send: Sender<Events>,
    relay_address: Multiaddr,
    relay_peer_id: PeerId,

    proxy_message_channel: (
        mpsc::Sender<SocksStreamMessage>,
        mpsc::Receiver<SocksStreamMessage>,
    ),

    // Stream pool for connection reuse
    stream_pool: Arc<StreamPool>,

    /// Typed outbound handle over the swarm's `libp2p_stream::Control` —
    /// FindNodes asks and bandwidth-report notifies to the hub ride this.
    client: LibP2pClient,

    // Bootstrap connection management
    bootstrap_address: Multiaddr,
    bootstrap_peer_id: Option<PeerId>,
    bootstrap_connected: bool,
    bootstrap_dialing: bool,

    /// When `true`, peer-discovery failures bail the whole process
    /// (no TUI to surface the error to). When `false`, the discovery
    /// loop emits `Events::Error` and keeps retrying so the operator
    /// can see the message in the TUI and edit `Config.yaml`.
    headless: bool,

    /// Per-server "current destination peer" handle, keyed by listen port.
    /// The SOCKS accept loop holds a clone of the
    /// `Arc<ArcSwap<Option<PeerId>>>` and reads it at session-open time;
    /// the swarm event loop writes a fresh PeerId here after the initial
    /// discovery completes (or on re-discovery after a disconnect) so
    /// subsequent sessions auto-pick the replacement.
    ///
    /// `Option<PeerId>` so the listener can come up immediately at
    /// startup (with `None`) before discovery has finished. SOCKS
    /// connections that arrive before the first peer is found get a
    /// SOCKS-level failure with a clear "no destination peer yet" log,
    /// rather than the whole proxy hanging until the first server's
    /// retry loop finishes.
    ///
    /// `ArcSwap` (rather than `tokio::RwLock`) because reads are lock-
    /// free — no need to await a read lock on every SOCKS connect.
    destination_peers:
        std::collections::HashMap<u16, Arc<arc_swap::ArcSwap<Option<PeerId>>>>,

    /// Last time we ran an eager re-discovery for each port. Used to
    /// throttle the `PeerDisconnected` handler: if the same server
    /// just rediscovered (within `REDISCOVERY_COOLDOWN`), we skip
    /// running another full discover-and-connect cycle and only
    /// clear the ArcSwap. Killed the rediscovery storm in the
    /// post-fix world (was 148 cycles/12h, expect <10 with this).
    last_rediscovery: std::collections::HashMap<u16, std::time::Instant>,

    /// Whether the hub has answered a `ResolvePeers` query yet (BIT-597).
    /// Gates the warn-once "hub doesn't support resolution" log.
    resolve_supported: Option<bool>,

    /// Last known per-(port, pinned-peer) resolvability — stale-peer log
    /// lines fire on transitions, not every retry pass.
    pinned_resolvable: std::collections::HashMap<(u16, PeerId), bool>,
}

/// Minimum wall-clock gap between successive eager re-discoveries for
/// the same server, beyond which we go lazy and just clear the
/// ArcSwap (JIT discovery on the next SOCKS session). Set to 30 s —
/// the rationale:
///
/// - libp2p's default `peer_idle_connection_timeout` is around 10–30s
///   depending on transport; circuits routinely tear down and re-open
///   in that window, which is what produced the 148-cycle storm.
/// - 30 s is long enough that idle circuit churn doesn't trigger
///   work, but short enough that an *actively* used port stays warm
///   between consecutive curls in a benchmarking loop.
/// - If a session arrives during the cooldown window and finds the
///   destination already cleared, JIT discovery still runs — so
///   "lazy" remains the safety net.
const REDISCOVERY_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);

pub struct ProxyForwarding {
    token: String,
    swarm: Swarm<Behaviour>,
    event_send: Sender<Events>,
    destination_peer: PeerId,
}

impl ProxyNetwork<AuthStep> {
    pub async fn with_authentication() -> Result<ProxyNetwork<NetworkConnect>> {
        let mut auth_client = AuthenticationServiceClient::new(GRPC_CHANNEL.clone())
            .send_compressed(CompressionEncoding::Gzip)
            .accept_compressed(CompressionEncoding::Gzip);

        let kp = KEYPAIR.clone().try_into_ed25519().unwrap();

        let signed_msg = sha2::Sha256::digest(CONFIG.bitping_api_key.to_string());
        let signature = kp.sign(signed_msg.as_slice());
        let local_pubkey_b58 = base58_monero::encode_check(&kp.public().to_bytes())?;
        let response = auth_client
            .federated_api_token_authenticate(tonic::Request::new(FederatedApiTokenAuthRequest {
                api_token: CONFIG.bitping_api_key.to_string(),
                signature: base58_monero::encode_check(&signature)?,
                public_key: local_pubkey_b58.clone(),
            }))
            .await?;

        let token = response.into_inner().token;
        sanity_check_federated_token(&token, &local_pubkey_b58)?;

        Ok(ProxyNetwork(NetworkConnect { token }))
    }
}

/// Shape-only sanity check on the federated PASETO the auth service returned.
/// Fail-fast on the cheap obvious misconfigurations:
///
/// - Empty token → auth service returned nothing usable.
/// - Token doesn't start with the `v4.public.` header → auth service didn't
///   return a v4.public PASETO (wrong version, or returned some other shape
///   like an opaque session token).
/// - Token has fewer than three `.`-separated segments → not a well-formed
///   PASETO (header.payload.footer is the minimal shape; v4 typically has
///   four with an explicit footer).
///
/// We intentionally **do not** crack the claims JSON or unseal the
/// `FederatedFooter` here. The trusted validation is the hub's job at
/// report ingestion (`paseto_validator.validate_federated_token`).
/// This client-side check is just defense-in-depth so a malfunctioning auth
/// service doesn't silently generate hours of unattributable bandwidth reports
/// before someone notices the hub-side rejection metric.
fn sanity_check_federated_token(token: &str, _expected_pubkey_b58: &str) -> Result<()> {
    use color_eyre::eyre::bail;

    if token.is_empty() {
        bail!("auth service returned an empty federated token");
    }
    const V4_PUBLIC_PREFIX: &str = "v4.public.";
    if !token.starts_with(V4_PUBLIC_PREFIX) {
        bail!(
            "federated token did not start with `{V4_PUBLIC_PREFIX}`; got `{}...` — wrong PASETO version?",
            token.chars().take(16).collect::<String>()
        );
    }
    // v4 tokens with a footer have shape `v4.public.<payload>.<footer>` — three dots.
    // Tokens without a footer have two. We accept both for resilience, but the hub
    // will reject tokens missing the FederatedFooter at validation time anyway.
    let dot_count = token.bytes().filter(|&b| b == b'.').count();
    if dot_count < 2 {
        bail!(
            "federated token has only {dot_count} `.` separator(s); expected at least 2 for PASETO v4.public"
        );
    }

    info!(
        token_len = token.len(),
        token_prefix = %&token[..token.len().min(20)],
        has_footer = dot_count >= 3,
        "authenticated against auth service; federated PASETO acquired"
    );

    Ok(())
}

impl ProxyNetwork<NetworkConnect> {
    pub async fn with_swarm(
        self,
        event_send: Sender<Events>,
        headless: bool,
    ) -> Result<ProxyNetwork<Bootstrapped>> {
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(KEYPAIR.clone())
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_quic_config(|mut config| {
                config.max_idle_timeout = u32::MAX;
                config.handshake_timeout = Duration::from_secs(120);
                config
            })
            .with_dns()?
            .with_relay_client(noise::Config::new, yamux::Config::default)?
            .with_behaviour(|k, rc| Behaviour::new(k.public(), rc))?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX))
            })
            .build();
        let _ = event_send
            .send(Events::LocalPeerId(*swarm.local_peer_id()))
            .await;

        let tcp_ip4_addr = multiaddr::multiaddr!(Ip4([0, 0, 0, 0]), Tcp(CONFIG.port));
        let quic_ip4_addr = multiaddr::multiaddr!(Ip4([0, 0, 0, 0]), Udp(CONFIG.port), QuicV1);
        let tcp_ip6_addr = multiaddr::multiaddr!(Ip6([0, 0, 0, 0, 0, 0, 0, 0]), Tcp(CONFIG.port));
        let quic_ip6_addr =
            multiaddr::multiaddr!(Ip6([0, 0, 0, 0, 0, 0, 0, 0]), Udp(CONFIG.port), QuicV1);

        swarm.listen_on(tcp_ip4_addr.clone())?;
        swarm.listen_on(quic_ip4_addr.clone())?;
        swarm.listen_on(tcp_ip6_addr.clone())?;
        swarm.listen_on(quic_ip6_addr.clone())?;

        swarm.add_external_address(tcp_ip4_addr);
        swarm.add_external_address(quic_ip4_addr);
        swarm.add_external_address(tcp_ip6_addr);
        swarm.add_external_address(quic_ip6_addr);

        let _ = event_send
            .send(Events::Connection(
                models::events::ConnectionEvents::Connecting,
            ))
            .await;

        // Bootstrap hub multiaddr is configurable via Config.yaml's
        // `bootstrap:` field; defaults to `/dnsaddr/boot2.bitping.com`
        // (Bitping production) when omitted.
        let bootstrap = CONFIG.bootstrap.clone();
        info!(%bootstrap, "Bootstrap hub multiaddr");

        // Retry bootstrap connection until successful
        let mut bootstrap_retry_count = 0;
        const MAX_BOOTSTRAP_RETRIES: usize = 10;

        let bootstrap_peer_id = loop {
            match swarm.dial(bootstrap.clone()) {
                Ok(_) => {
                    info!(
                        "Attempting to connect to bootstrap server (attempt {}/{})",
                        bootstrap_retry_count + 1,
                        MAX_BOOTSTRAP_RETRIES
                    );
                }
                Err(e) => {
                    warn!(?e, "Failed to dial bootstrap server");
                    if bootstrap_retry_count >= MAX_BOOTSTRAP_RETRIES {
                        bail!(
                            "Failed to dial bootstrap server after {} attempts",
                            MAX_BOOTSTRAP_RETRIES
                        );
                    }
                    bootstrap_retry_count += 1;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            }

            // Wait for identify event with timeout
            match swarm
                .wait_for_with_timeout(
                    |swarm, event| {
                        if let SwarmEvent::Behaviour(BehaviourEvent::Identify(
                            identify::Event::Received {
                                connection_id: _,
                                peer_id,
                                info,
                            },
                        )) = event
                        {
                            for addr in &info.listen_addrs {
                                let circuit_addr = addr
                                    .clone()
                                    .with_p2p(*peer_id)
                                    .ok()?
                                    .with(Protocol::P2pCircuit)
                                    .with_p2p(*swarm.local_peer_id())
                                    .ok()?;

                                let _ = swarm.listen_on(circuit_addr);
                            }
                            Some(*peer_id)
                        } else {
                            None
                        }
                    },
                    Duration::from_secs(10),
                )
                .await
            {
                Ok(peer_id) => {
                    info!("Successfully connected to bootstrap server");
                    break peer_id;
                }
                Err(_) => {
                    warn!("Bootstrap connection timeout");
                    if bootstrap_retry_count >= MAX_BOOTSTRAP_RETRIES {
                        bail!(
                            "Failed to connect to bootstrap server after {} attempts",
                            MAX_BOOTSTRAP_RETRIES
                        );
                    }
                    bootstrap_retry_count += 1;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            }
        };

        info!("Waiting for Relay reservation.");

        // Wait for the relay reservation event
        let (relay_peer_id, renewal, limit) = swarm
            .wait_for(|_swarm, event| {
                if let SwarmEvent::Behaviour(BehaviourEvent::Relay(
                    relay::client::Event::ReservationReqAccepted {
                        relay_peer_id,
                        renewal,
                        limit,
                    },
                )) = event
                {
                    Some((*relay_peer_id, *renewal, *limit))
                } else {
                    None
                }
            })
            .await;

        // Store relay peer ID in app state
        let _ = event_send
            .send(Events::Connection(
                models::events::ConnectionEvents::Connected(relay_peer_id),
            ))
            .await;

        info!(%relay_peer_id, %renewal, ?limit, "Reservation accepted, time to connect to peer.");

        let Ok(relay_address) = bootstrap.clone().with_p2p(bootstrap_peer_id) else {
            bail!("Could not construct relay multiaddr")
        };

        // Create stream pool with default config (will be overridden per server)
        let pool_config = PoolConfig::default();
        let stream_control = swarm.behaviour().stream.new_control();
        let stream_pool = StreamPool::new(stream_control, pool_config);

        let client = LibP2pClient::new(
            swarm.behaviour().stream.new_control(),
            *swarm.local_peer_id(),
        );

        Ok(ProxyNetwork(Bootstrapped {
            swarm,
            token: self.0.token,
            event_send,
            relay_address,
            relay_peer_id,
            proxy_message_channel: mpsc::channel(1000),
            stream_pool,
            client,
            bootstrap_address: bootstrap,
            bootstrap_peer_id: Some(bootstrap_peer_id),
            bootstrap_connected: true,
            bootstrap_dialing: false,
            headless,
            destination_peers: std::collections::HashMap::new(),
            last_rediscovery: std::collections::HashMap::new(),
            resolve_supported: None,
            pinned_resolvable: std::collections::HashMap::new(),
        }))
    }
}

impl ProxyNetwork<Bootstrapped> {
    pub async fn configure_server(
        &mut self,
        server: &'static Server,
        _shutdown: &tokio_util::sync::CancellationToken,
    ) -> Result<()> {
        // BIND FIRST. Previously this function ran the full
        // `discover_and_connect_to_peer` retry loop *before* binding the
        // SOCKS listener — so if server 1's filters matched no peers, its
        // listener never came up *and* every later server in
        // CONFIG.servers was blocked behind it because the outer
        // `for server in ...` loop awaited each call serially. The fix:
        // open the TCP listener immediately with an empty
        // `ArcSwap<Option<PeerId>>`, register it in the swarm's
        // destination_peers map, then post a `DiscoverPeerForServer`
        // message on the proxy channel so the swarm task runs discovery
        // in the background. The accept loop reports SOCKS GeneralFailure
        // to any client that connects before a peer is found.
        let peer_handle: Arc<arc_swap::ArcSwap<Option<PeerId>>> =
            Arc::new(arc_swap::ArcSwap::from_pointee(None));
        self.0
            .destination_peers
            .insert(server.port, peer_handle.clone());

        proxy_protocols::socks_stream::create_socks_proxy_stream(
            server,
            &KEYPAIR,
            self.0.token.to_string(),
            peer_handle,
            self.0.stream_pool.clone(),
            self.0.proxy_message_channel.0.clone(),
        )
        .await?;

        // Kick off discovery in the background via the existing message
        // channel. The handler runs in `handle_proxy_events`, which has
        // exclusive `&mut self` access to the swarm — same machinery as
        // the PeerDisconnected re-discovery path.
        if let Err(e) = self
            .0
            .proxy_message_channel
            .0
            .send(SocksStreamMessage::DiscoverPeerForServer { server_config: server })
            .await
        {
            warn!(?e, port = server.port, "Failed to enqueue initial DiscoverPeerForServer");
        }

        Ok(())
    }

    /// Borrow the discovery-facing slice of the bootstrapped state. The
    /// tuple field is module-private, so this constructor is the only way
    /// `discovery::connect` gets its hands on the swarm.
    fn discovery_engine(&mut self) -> crate::discovery::DiscoveryEngine<'_> {
        crate::discovery::DiscoveryEngine {
            swarm: &mut self.0.swarm,
            client: &self.0.client,
            relay_address: &self.0.relay_address,
            relay_peer_id: self.0.relay_peer_id,
            token: &self.0.token,
            event_send: &self.0.event_send,
            headless: self.0.headless,
            resolve_supported: &mut self.0.resolve_supported,
            pinned_resolvable: &mut self.0.pinned_resolvable,
        }
    }

    /// Attempt to dial the bootstrap server if not already connected or dialing
    fn try_dial_bootstrap(&mut self) {
        if !self.0.bootstrap_connected && !self.0.bootstrap_dialing {
            info!("Attempting to dial bootstrap server");
            match self.0.swarm.dial(self.0.bootstrap_address.clone()) {
                Ok(_) => {
                    self.0.bootstrap_dialing = true;
                    debug!("Bootstrap dial initiated");
                }
                Err(e) => {
                    warn!(?e, "Failed to dial bootstrap server");
                }
            }
        }
    }

    pub async fn drive_network(
        mut self,
        server_state: Arc<RwLock<ServerContainer>>,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Result<()> {
        // Initial bootstrap dial check
        self.try_dial_bootstrap();

        // Bootstrap reconnection timer
        let mut bootstrap_retry_timer = tokio::time::interval(Duration::from_secs(5));
        bootstrap_retry_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Main event loop
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("Shutdown requested — gracefully disconnecting from hub & peers");
                    self.graceful_shutdown().await;
                    return Ok(());
                }
                Some(message) = self.0.proxy_message_channel.1.recv() => {
                    if let Err(e) = self.handle_proxy_events(message, server_state.clone(), &shutdown).await {
                        warn!(?e, "Something went wrong handling proxy events");
                    }
                },
                Some(event) = self.0.swarm.next() => {
                    self.handle_swarm_events_with_bootstrap(event, server_state.clone());
                }
                _ = bootstrap_retry_timer.tick() => {
                    // Periodically check if we need to reconnect to bootstrap
                    self.try_dial_bootstrap();
                }
            };
        }
    }

    /// Cleanly close all libp2p connections so the hub sees a proper noise /
    /// yamux / QUIC close instead of a TCP RST. Without this, restarting
    /// p2proxy within the hub's liveness-timeout window leaves the previous
    /// session lingering on the hub side — FindNodes then returns peers whose
    /// reservations still point at the stale session, and the new dial
    /// stack-fails for ~20 retries until those reservations age out.
    ///
    /// Strategy: disconnect every connected peer, then poll the swarm for up
    /// to one second so libp2p can flush the close frames and emit the
    /// `ConnectionClosed` events.
    async fn graceful_shutdown(&mut self) {
        let peers: Vec<PeerId> = self.0.swarm.connected_peers().copied().collect();
        let peer_count = peers.len();
        for peer in peers {
            // `disconnect_peer_id` returns Err if the peer is already gone —
            // a benign race with a peer that disconnected first.
            let _ = self.0.swarm.disconnect_peer_id(peer);
        }
        info!(peer_count, "Disconnecting from peers");

        let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            if self.0.swarm.connected_peers().next().is_none() {
                break;
            }
            tokio::select! {
                _ = tokio::time::sleep_until(drain_deadline) => {
                    let stragglers = self.0.swarm.connected_peers().count();
                    if stragglers > 0 {
                        warn!(stragglers, "Drain deadline hit; forcing exit");
                    }
                    break;
                }
                _ = self.0.swarm.next() => {
                    // We don't care which event — we're just pumping the
                    // event loop so libp2p can finish its close handshakes.
                }
            }
        }
    }

    async fn handle_proxy_events(
        &mut self,
        message: SocksStreamMessage,
        server_state: Arc<RwLock<ServerContainer>>,
        shutdown: &tokio_util::sync::CancellationToken,
    ) -> Result<()> {
        match message {
            SocksStreamMessage::Initialized {
                session_id,
                target_addr,
                peer,
            } => {
                counter!("p2proxy_sessions_initialized_total").increment(1);
                metrics::gauge!("p2proxy_sessions_active").increment(1.0);
                debug!(
                    "New session: {} to {:?} via {}",
                    session_id, target_addr, peer
                );

                // Emit through `event_send`, not `server_state` directly.
                // `main.rs::handle_swarm_events` is the fan-out point that
                // mirrors to BOTH the TUI's `tui_tx` channel and the
                // shared `server_state`. Bypassing it (which the old
                // code did) made the TUI miss every Session/Bandwidth
                // event — so the Overview tab showed zeros even when
                // SOCKS sessions were actively transferring data.
                let _ = server_state; // explicit no-op for the old path
                let _ = self
                    .0
                    .event_send
                    .send(Events::Session(models::events::SessionEvents::New(
                        session_id,
                        target_addr,
                        peer,
                    )))
                    .await;
            }
            SocksStreamMessage::DataTransferred {
                session_id,
                direction,
                bytes,
            } => {
                let bw_event = match direction {
                    DataDirection::Incoming => {
                        counter!("p2proxy_download_bytes_total").increment(bytes as u64);
                        // Per-session label so Prometheus can break down
                        // throughput by SOCKS session — useful for
                        // spotting one runaway client among many.
                        counter!(
                            "p2proxy_session_download_bytes",
                            "session_id" => session_id.to_string()
                        )
                        .increment(bytes as u64);
                        models::events::BandwidthEvents::Download(session_id, bytes as u64)
                    }
                    DataDirection::Outgoing => {
                        counter!("p2proxy_upload_bytes_total").increment(bytes as u64);
                        counter!(
                            "p2proxy_session_upload_bytes",
                            "session_id" => session_id.to_string()
                        )
                        .increment(bytes as u64);
                        models::events::BandwidthEvents::Upload(session_id, bytes as u64)
                    }
                };
                // Same fix as Initialized — fan out via event_send so
                // the TUI's bandwidth graph + totals actually update.
                let _ = self
                    .0
                    .event_send
                    .send(Events::Bandwidth(bw_event))
                    .await;

                let dir_str = match direction {
                    DataDirection::Incoming => "incoming",
                    DataDirection::Outgoing => "outgoing",
                };
                debug!("Session {}: {} {} bytes", session_id, dir_str, bytes);
            }
            SocksStreamMessage::Error {
                session_id,
                error,
                stage,
            } => {
                counter!("p2proxy_session_errors_total").increment(1);
                warn!(
                    "Error in session {:?} during {:?}: {}",
                    session_id, stage, error
                );
            }
            SocksStreamMessage::Finished {
                session_id,
                incoming_hash,
                outgoing_hash,
                report,
            } => {
                counter!("p2proxy_sessions_completed_total").increment(1);
                metrics::gauge!("p2proxy_sessions_active").decrement(1.0);
                debug!(
                    ?session_id,
                    ?incoming_hash,
                    ?outgoing_hash,
                    ?report,
                    "Session finished.",
                );

                // Fan out via event_send so the TUI removes the session
                // row in lock-step with the ServerContainer.
                let _ = self
                    .0
                    .event_send
                    .send(Events::Session(models::events::SessionEvents::End(
                        session_id,
                    )))
                    .await;

                let token = self.0.token.clone();
                if let Ok(authed_report) = Auth::new(report, &KEYPAIR, token) {
                    counter!("p2proxy_bandwidth_reports_sent_total").increment(1);
                    // Fire-and-forget notify, spawned so the proxy-event
                    // handler returns immediately; the stream opens once the
                    // main loop resumes polling the swarm.
                    let client = self.0.client.clone();
                    let relay_peer = self.0.relay_peer_id;
                    tokio::spawn(async move {
                        if let Err(e) = client
                            .notify::<AuthedBandwidthReport>(
                                relay_peer,
                                AuthedBandwidthReport(authed_report),
                            )
                            .await
                        {
                            warn!(?e, "bandwidth report notify failed");
                        }
                    });
                }
            }
            SocksStreamMessage::RequestNewPeer {
                callback,
                server_config,
            } => {
                // JIT discovery from a SOCKS session that found its
                // destination peer cleared. Runs full
                // `discover_and_connect_to_peer`, stores the result in
                // the per-server `ArcSwap` (so the next session reuses
                // it without a round-trip), emits ActiveDestination so
                // the TUI updates, and finally sends the peer back to
                // the requesting session via the oneshot.
                counter!("p2proxy_peer_requests_total").increment(1);
                match crate::discovery::connect(self.discovery_engine(), server_config, shutdown).await {
                    Ok(destination) => {
                        counter!("p2proxy_peer_discoveries_successful_total").increment(1);
                        if let Some(handle) =
                            self.0.destination_peers.get(&server_config.port).cloned()
                        {
                            handle.store(Arc::new(Some(destination.peer)));
                        }
                        metrics::gauge!(
                            "p2proxy_server_active_destination_present",
                            "port" => server_config.port.to_string()
                        )
                        .set(1.0);
                        let _ = self
                            .0
                            .event_send
                            .send(Events::ActiveDestination {
                                port: server_config.port,
                                peer: Some(destination.peer),
                                source: Some(destination.source),
                            })
                            .await;
                        let _ = callback
                            .send(destination.peer)
                            .map_err(|p| eyre!("Failed to send new peer back to stream {p}"))?;
                    }
                    e => {
                        counter!("p2proxy_peer_discoveries_failed_total").increment(1);
                        let _ = e.wrap_err("Failed to discover peer after connection dropped")?;
                    }
                }
            }
            SocksStreamMessage::PeerDisconnected {
                server_config,
                old_peer,
            } => {
                // Throttled-eager rediscovery. The trade-off here is:
                //
                //   - Full lazy (clear ArcSwap, no discovery) keeps
                //     the swarm event loop quiet but makes the first
                //     SOCKS session after a disconnect pay the full
                //     discover-and-connect latency (up to 10s), which
                //     trips client-side curl timeouts on sparsely-
                //     populated countries (NZ has 2 candidates that
                //     might both be unreachable through the circuit
                //     on first dial).
                //
                //   - Full eager (run discovery on every
                //     ConnectionClosed) thrashes — saw 148 cycles in
                //     a 12-hour idle window because libp2p tears
                //     circuits down and reopens on its own schedule.
                //
                // Compromise: run eager rediscovery, but at most
                // once per `REDISCOVERY_COOLDOWN`. If a flurry of
                // closes arrives, only the first triggers discovery;
                // subsequent ones inside the window just clear the
                // ArcSwap (so the JIT lazy path can still recover
                // when needed). This keeps the destination warm for
                // active ports without burning cycles on idle ones.
                let Some(handle) = self.0.destination_peers.get(&server_config.port).cloned()
                else {
                    return Ok(());
                };
                if **handle.load() != Some(old_peer) {
                    return Ok(());
                }

                let now = std::time::Instant::now();
                let throttled = self
                    .0
                    .last_rediscovery
                    .get(&server_config.port)
                    .map(|last| now.duration_since(*last) < REDISCOVERY_COOLDOWN)
                    .unwrap_or(false);

                if throttled {
                    counter!(
                        "p2proxy_peer_lazy_cleared_total",
                        "port" => server_config.port.to_string()
                    )
                    .increment(1);
                    debug!(
                        ?old_peer,
                        port = server_config.port,
                        "destination peer cleared (rediscovery throttled — next session will JIT)"
                    );
                    handle.store(Arc::new(None));
                    metrics::gauge!(
                        "p2proxy_server_active_destination_present",
                        "port" => server_config.port.to_string()
                    )
                    .set(0.0);
                    let _ = self
                        .0
                        .event_send
                        .send(Events::ActiveDestination {
                            port: server_config.port,
                            peer: None,
                            source: None,
                        })
                        .await;
                    return Ok(());
                }

                // Within the cooldown window we'd skip; outside it,
                // do the full discover-and-connect now so the next
                // SOCKS client finds a warm destination. Stamp the
                // cooldown timer *before* the await so concurrent
                // closes (which arrive on this same handler) all see
                // a recent timestamp and short-circuit.
                self.0
                    .last_rediscovery
                    .insert(server_config.port, now);
                counter!(
                    "p2proxy_peer_proactive_rediscovery_total",
                    "port" => server_config.port.to_string()
                )
                .increment(1);
                info!(
                    ?old_peer,
                    port = server_config.port,
                    "destination peer disconnected — running eager rediscovery"
                );
                match crate::discovery::connect(self.discovery_engine(), server_config, shutdown)
                    .await
                {
                    Ok(destination) => {
                        info!(?old_peer, new_peer = ?destination.peer, port = server_config.port, "warm-rediscovered destination peer");
                        handle.store(Arc::new(Some(destination.peer)));
                        metrics::gauge!(
                            "p2proxy_server_active_destination_present",
                            "port" => server_config.port.to_string()
                        )
                        .set(1.0);
                        let _ = self
                            .0
                            .event_send
                            .send(Events::ActiveDestination {
                                port: server_config.port,
                                peer: Some(destination.peer),
                                source: Some(destination.source),
                            })
                            .await;
                    }
                    Err(e) => {
                        // Rediscovery failed — leave the ArcSwap
                        // cleared so the lazy JIT path kicks in on
                        // the next session attempt.
                        warn!(
                            ?e,
                            port = server_config.port,
                            "eager rediscovery failed; clearing destination, JIT will retry on next session"
                        );
                        handle.store(Arc::new(None));
                        metrics::gauge!(
                            "p2proxy_server_active_destination_present",
                            "port" => server_config.port.to_string()
                        )
                        .set(0.0);
                        let _ = self
                            .0
                            .event_send
                            .send(Events::ActiveDestination {
                                port: server_config.port,
                                peer: None,
                                source: None,
                            })
                            .await;
                    }
                }
            }
            SocksStreamMessage::DiscoverPeerForServer { server_config } => {
                // Initial discovery for a freshly-bound server (enqueued
                // by configure_server). Same shape as PeerDisconnected
                // except there's no old peer to validate against.
                let Some(handle) = self.0.destination_peers.get(&server_config.port).cloned()
                else {
                    debug!(port = server_config.port, "DiscoverPeerForServer for unknown server, ignoring");
                    return Ok(());
                };
                info!(port = server_config.port, "running initial peer discovery");
                match crate::discovery::connect(self.discovery_engine(), server_config, shutdown).await {
                    Ok(destination) => {
                        info!(peer = ?destination.peer, port = server_config.port, "destination peer discovered");
                        handle.store(Arc::new(Some(destination.peer)));
                        metrics::gauge!(
                            "p2proxy_server_active_destination_present",
                            "port" => server_config.port.to_string()
                        )
                        .set(1.0);
                        let _ = self
                            .0
                            .event_send
                            .send(Events::ActiveDestination {
                                port: server_config.port,
                                peer: Some(destination.peer),
                                source: Some(destination.source),
                            })
                            .await;
                    }
                    Err(e) => {
                        // In TUI mode discover loops forever, so reaching
                        // here means headless mode bailed or shutdown
                        // was triggered. Listener stays up but every
                        // SOCKS session will GeneralFailure until a peer
                        // appears (e.g. via a future SOCKS-triggered
                        // RequestNewPeer).
                        warn!(?e, port = server_config.port, "initial discovery failed");
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle swarm events with bootstrap connection management
    fn handle_swarm_events_with_bootstrap(
        &mut self,
        event: SwarmEvent<BehaviourEvent>,
        server_state: Arc<RwLock<ServerContainer>>,
    ) {
        // Handle bootstrap-specific events
        match &event {
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                if Some(*peer_id) == self.0.bootstrap_peer_id {
                    info!("Bootstrap connection established");
                    self.0.bootstrap_connected = true;
                    self.0.bootstrap_dialing = false;
                }
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                if Some(*peer_id) == self.0.bootstrap_peer_id {
                    warn!("Bootstrap connection lost");
                    self.0.bootstrap_connected = false;
                    self.0.bootstrap_dialing = false;
                }
                // Proactive destination-peer rediscovery: if the dropped
                // peer is the current destination for any of our servers,
                // tell ourselves (via proxy_message_channel) to find a
                // replacement now rather than waiting for a SOCKS client
                // to trip the failure. We need to find the matching server
                // by looking up the port whose ArcSwap holds this peer_id.
                let stale_servers: Vec<(u16, PeerId)> = self
                    .0
                    .destination_peers
                    .iter()
                    .filter_map(|(port, handle)| {
                        (**handle.load() == Some(*peer_id)).then_some((*port, *peer_id))
                    })
                    .collect();
                for (port, old_peer) in stale_servers {
                    // Find the Server struct by port. servers come from
                    // CONFIG.servers (&'static), so a single lookup is
                    // cheap and always succeeds — the entry only exists
                    // because configure_server put it there.
                    let Some(server) = CONFIG
                        .servers
                        .iter()
                        .find(|s| s.port == port)
                    else {
                        continue;
                    };
                    let sender = self.0.proxy_message_channel.0.clone();
                    tokio::spawn(async move {
                        if let Err(e) = sender
                            .send(SocksStreamMessage::PeerDisconnected {
                                server_config: server,
                                old_peer,
                            })
                            .await
                        {
                            debug!(?e, "PeerDisconnected enqueue failed (receiver gone)");
                        }
                    });
                }
            }
            SwarmEvent::OutgoingConnectionError { peer_id, .. } => {
                if peer_id.as_ref() == self.0.bootstrap_peer_id.as_ref() {
                    warn!("Bootstrap connection failed");
                    self.0.bootstrap_dialing = false;
                }
            }
            // Ping liveness — failures mean the peer's transport stack
            // stopped responding to our 15s pings. Force-close the
            // connection so libp2p emits the normal `ConnectionClosed`
            // event, which our `PeerDisconnected` flow then turns into
            // a rotation-pool removal + throttled rediscovery. Without
            // this, silent network drops (laptop sleep, NAT TTL,
            // peer's daemon hung) would leave the dead peer in the
            // pool until something else noticed.
            SwarmEvent::Behaviour(BehaviourEvent::Ping(ping::Event {
                peer,
                result: Err(failure),
                ..
            })) => {
                counter!(
                    "p2proxy_ping_failures_total",
                    "peer" => peer.to_string()
                )
                .increment(1);
                warn!(%peer, ?failure, "ping failure — disconnecting peer");
                let _ = self.0.swarm.disconnect_peer_id(*peer);
            }
            SwarmEvent::Behaviour(BehaviourEvent::Ping(ping::Event {
                peer,
                result: Ok(rtt),
                ..
            })) => {
                metrics::histogram!(
                    "p2proxy_ping_rtt_seconds",
                    "peer" => peer.to_string()
                )
                .record(rtt.as_secs_f64());
            }
            _ => {}
        }

        // Delegate to the original handler
        handle_swarm_events(event, server_state, &self.0.event_send);
    }
}

fn handle_swarm_events(
    event: SwarmEvent<BehaviourEvent>,
    _server_state: Arc<RwLock<ServerContainer>>,
    event_send: &Sender<Events>,
) {
    match event {
        SwarmEvent::ConnectionEstablished {
            peer_id, endpoint, ..
        } => {
            counter!("p2proxy_peer_connections_total").increment(1);
            gauge!("p2proxy_peers_connected").increment(1.0);
            info!("Connection established with peer: {}", peer_id);

            // Route via event_send so main.rs's fan-out reaches both
            // server_state AND the TUI. The previous direct-to-state
            // path was why the TUI's CONNECTED PEERS gauge sat at 1
            // (the bootstrap hub from with_swarm) regardless of how
            // many destination peers we actually had open.
            let tx = event_send.clone();
            tokio::spawn(async move {
                let _ = tx
                    .send(Events::Connection(
                        models::events::ConnectionEvents::Connected(peer_id),
                    ))
                    .await;
            });
        }
        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            gauge!("p2proxy_peers_connected").decrement(1.0);
            info!("Connection closed with peer: {}", peer_id);

            // Carry the peer_id so the TUI can drop just this one
            // peer from its sets and the rotation pools, rather than
            // clearing every peer on a single disconnect.
            let tx = event_send.clone();
            tokio::spawn(async move {
                let _ = tx
                    .send(Events::Connection(
                        models::events::ConnectionEvents::Disconnected(peer_id),
                    ))
                    .await;
            });
        }
        SwarmEvent::NewListenAddr { address, .. } => {
            counter!("p2proxy_listen_addresses_total").increment(1);
            debug!("Listening on: {}", address);
        }
        SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received {
            peer_id,
            info,
            ..
        })) => {
            counter!("p2proxy_peer_identified_total").increment(1);
            debug!("Identified peer: {}", peer_id);
            for addr in &info.listen_addrs {
                debug!("  Address: {}", addr);
            }
        }
        SwarmEvent::Behaviour(BehaviourEvent::Relay(
            relay::client::Event::ReservationReqAccepted {
                relay_peer_id,
                renewal,
                limit,
            },
        )) => {
            counter!("p2proxy_relay_reservations_total").increment(1);
            debug!("Relay reservation accepted: {}", relay_peer_id);
        }
        SwarmEvent::Behaviour(BehaviourEvent::Dcutr(e)) => {
            counter!("p2proxy_dcutr_events_total").increment(1);
            info!(?e, "Dcutr event");
        }
        event => {
            debug!(?event, "Other event received");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanity_check_rejects_empty_token() {
        let err = sanity_check_federated_token("", "fake-pubkey").unwrap_err();
        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn sanity_check_rejects_wrong_paseto_version() {
        let err = sanity_check_federated_token("v3.local.somecontent", "pubkey").unwrap_err();
        assert!(format!("{err}").contains("v4.public"));
    }

    #[test]
    fn sanity_check_rejects_garbage_token() {
        let err = sanity_check_federated_token("not-a-paseto-at-all", "pubkey").unwrap_err();
        assert!(format!("{err}").contains("v4.public"));
    }

    // A token with the right `v4.public.` prefix but no further `.` separator
    // would have only two dots (the prefix's), and `dot_count >= 2` accepts it.
    // The shape check is intentionally permissive — the hub is the trusted gate.

    #[test]
    fn sanity_check_accepts_well_formed_v4_public_with_footer() {
        // Three `.`-segments after the v4.public prefix → well-formed shape.
        // No crypto verification at this layer; the hub does the real check.
        sanity_check_federated_token(
            "v4.public.eyJzb21lIjoiY2xhaW1zIn0.eyJmb290ZXIiOiJoZXJlIn0",
            "pubkey",
        )
        .expect("well-formed token should pass shape check");
    }
}
