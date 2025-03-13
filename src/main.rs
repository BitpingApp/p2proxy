use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use bitping_tcp_proxy::bandwidth_reporter::{BandwidthReporterCodec, BandwidthReporterProtocol};
use color_eyre::eyre::{bail, Context, Result};
use config::Config;
use futures::StreamExt;
use libp2p::{
    dcutr, identify,
    identity::{Keypair, PublicKey},
    multiaddr::{self, Protocol},
    noise, relay,
    request_response::{self, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId,
};
use libp2p_stream as stream;
use metrics::gauge;
use metrics_exporter_prometheus::PrometheusBuilder;
use protocols::auth::v1::{
    authentication_service_client::AuthenticationServiceClient, FederatedApiTokenAuthRequest,
};
use rand::Rng;
use ratatui::{style::Color, widgets::ListState};
use sha2::Digest;
use state::{ConnectionStatus, APP_STATE};
use tonic::{
    codec::CompressionEncoding,
    transport::{Channel, ClientTlsConfig},
};
use tracing::{debug, info, level_filters::LevelFilter, warn};
use tracing_subscriber::EnvFilter;

use utils::wait_ext::SwarmWaitExt;

mod config;
mod proxy_protocols;
mod state;
mod ui;
mod utils;

#[derive(NetworkBehaviour)]
struct Behaviour {
    stream: stream::Behaviour,
    dcutr: dcutr::Behaviour,
    relay: relay::client::Behaviour,
    identify: identify::Behaviour,
    bandwidth_reporter: request_response::Behaviour<BandwidthReporterCodec>,
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
            bandwidth_reporter: request_response::Behaviour::new(
                [(BandwidthReporterProtocol, ProtocolSupport::Outbound)],
                request_response::Config::default().with_max_concurrent_streams(1000),
            ),
        }
    }
}

static KEYPAIR: LazyLock<Keypair> = LazyLock::new(libp2p::identity::Keypair::generate_ed25519);

static GRPC_CHANNEL: LazyLock<Channel> = LazyLock::new(|| {
    get_grpc_channel("https://grpc.bitping.com".into(), "grpc.bitping.com".into())
        .expect("Failed to resolve GRPC Channel")
});

pub fn get_grpc_channel(grpc_hub_url: String, grpc_hub_domain: String) -> Result<Channel> {
    let channel_config = if grpc_hub_url.starts_with("https://") {
        let tls = ClientTlsConfig::new().domain_name(grpc_hub_domain);
        Channel::builder(grpc_hub_url.try_into()?)
            .tls_config(tls)
            .context("Error configuring TLS for GRPC")?
    } else {
        Channel::builder(grpc_hub_url.try_into()?)
    };

    Ok(channel_config.connect_lazy())
}

static CONFIG: LazyLock<Config> =
    LazyLock::new(|| Config::new().expect("Cannot initialise config"));

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::ERROR.into())
                .from_env()?,
        )
        .pretty()
        .init();

    color_eyre::install()?;

    ui::thread::run_thread();

    let builder = PrometheusBuilder::new();
    builder.install()?;

    // Update connection status
    {
        APP_STATE.update(|state| {
            state.connection_status = ConnectionStatus::Connecting;
        });
    }

    let destination_address = CONFIG.destination_address.clone();

    let mut auth_client = AuthenticationServiceClient::new(GRPC_CHANNEL.clone())
        .send_compressed(CompressionEncoding::Gzip)
        .accept_compressed(CompressionEncoding::Gzip);

    let kp = KEYPAIR.clone().try_into_ed25519().unwrap();

    let signed_msg = sha2::Sha256::digest(std::env::var("BITPING_API_KEY").unwrap());
    let signature = kp.sign(signed_msg.as_slice());
    let response = auth_client
        .federated_api_token_authenticate(tonic::Request::new(FederatedApiTokenAuthRequest {
            api_token: std::env::var("BITPING_API_KEY").unwrap(),
            signature: base58_monero::encode_check(&signature)?,
            public_key: base58_monero::encode_check(&kp.public().to_bytes())?,
        }))
        .await?;

    let token = response.get_ref().token.clone();
    let token = Cow::from(token);

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(KEYPAIR.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic_config(|c| {
            let mut config = c.clone();
            config.max_idle_timeout = u32::MAX;
            config.handshake_timeout = Duration::from_secs(120);
            config
        })
        .with_dns()?
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(|k, rc| Behaviour::new(k.public(), rc))?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX)))
        .build();

    // Store local peer ID in app state
    {
        let mut state = APP_STATE.lock().unwrap();
        state.local_peer_id = Some(*swarm.local_peer_id());
    }

    swarm.listen_on("/ip4/0.0.0.0/tcp/45445".parse()?)?;
    swarm.listen_on("/ip4/0.0.0.0/udp/45445/quic-v1".parse()?)?;

    let bootstrap = multiaddr::multiaddr!(Dnsaddr("boot1.bitping.com"));
    swarm.dial(bootstrap)?;

    // Wait for the listen address event
    let listen_address = swarm
        .wait_for(|swarm, event| {
            if let SwarmEvent::NewListenAddr { address, .. } = event {
                let listen_address = address.clone().with_p2p(*swarm.local_peer_id()).unwrap();
                tracing::info!(%listen_address);

                let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
                println!(
                    "To Connect:  RUST_LOG={} cargo run --package stream-example -- {}",
                    rust_log, listen_address
                );

                Some(listen_address.clone())
            } else {
                None
            }
        })
        .await;

    swarm
        .wait_for(|swarm, event| {
            if let SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received {
                connection_id: _,
                peer_id,
                info,
            })) = event
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
                Some(())
            } else {
                None
            }
        })
        .await;

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
                // Store relay peer ID in app state
                {
                    let mut state = APP_STATE.lock().unwrap();
                    state.relay_peer_id = Some(*relay_peer_id);
                }

                Some((*relay_peer_id, *renewal, *limit))
            } else {
                None
            }
        })
        .await;

    info!(%relay_peer_id, %renewal, ?limit, "Reservation accepted, time to connect to peer.");

    let Some(Protocol::P2p(peer_id)) = destination_address.iter().last() else {
        bail!("Provided address does not end in `/p2p`");
    };

    swarm.dial(destination_address)?;

    let span = tracing::debug_span!("dcutr_upgrade");
    let _enter = span.enter();
    // Wait for successful DCUtR connection
    swarm
        .wait_for(|_swarm, event| match event {
            SwarmEvent::Behaviour(BehaviourEvent::Dcutr(dcutr::Event {
                remote_peer_id,
                result,
            })) => {
                warn!(?result, "DCUTR result");

                if *remote_peer_id == peer_id {
                    Some(())
                } else {
                    None
                }
            }
            SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                num_established,
                concurrent_dial_errors,
                established_in,
            } => Some(()),
            e => {
                debug!(?e, "Got other message while waiting for dcutr");

                None
            }
        })
        .await;

    info!("Direct connection established with peer {}", peer_id);

    // Update connection status
    {
        let mut state = APP_STATE.lock().unwrap();
        state.connection_status = ConnectionStatus::Connected;
    }

    // Now that we've got a solid connection to the Peer, we can start the proxy
    info!("Starting SOCKS5 proxy for peer {}", peer_id);

    // Use our wrapped version that tracks sessions
    let socks_handle = tokio::spawn(proxy_protocols::socks::run_socks_proxy(
        &KEYPAIR,
        token,
        peer_id,
        swarm.behaviour().stream.new_control(),
    ));

    // Main event loop
    loop {
        if let Some(event) = swarm.next().await {
            match event {
                SwarmEvent::ConnectionEstablished {
                    peer_id, endpoint, ..
                } => {
                    let mut state = APP_STATE.lock().unwrap();
                    let address = endpoint.get_remote_address().clone();
                    let is_relay = state.relay_peer_id.map_or(false, |id| peer_id == id);
                    state.peers.insert(
                        peer_id,
                        PeerInfo {
                            address,
                            connected_at: Instant::now(),
                            is_relay,
                        },
                    );
                    state.connection_status = ConnectionStatus::Connected;
                    info!("Connection established with peer: {}", peer_id);

                    // Update metrics
                    gauge!("p2proxy_peers_connected").set(state.peers.len() as f64);
                }
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    let mut state = APP_STATE.lock().unwrap();
                    state.peers.remove(&peer_id);

                    // Update connection status if all peers are gone
                    if state.peers.is_empty() {
                        state.connection_status = ConnectionStatus::Disconnected;
                    }

                    info!("Connection closed with peer: {}", peer_id);

                    // Update metrics
                    gauge!("p2proxy_peers_connected").set(state.peers.len() as f64);
                }
                SwarmEvent::NewListenAddr { address, .. } => {
                    info!("Listening on: {}", address);
                }
                SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received {
                    peer_id,
                    info,
                    ..
                })) => {
                    info!("Identified peer: {}", peer_id);
                    for addr in &info.listen_addrs {
                        info!("  Address: {}", addr);
                    }
                }
                SwarmEvent::Behaviour(BehaviourEvent::Relay(
                    relay::client::Event::ReservationReqAccepted {
                        relay_peer_id,
                        renewal,
                        limit,
                    },
                )) => {
                    info!("Relay reservation accepted: {}", relay_peer_id);
                }
                event => {
                    debug!(?event, "Other event received");
                }
            }
        }
    }
}
