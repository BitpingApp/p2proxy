use std::{
    borrow::{Borrow, Cow},
    sync::{Arc, LazyLock},
    time::Duration,
};

use bitping_tcp_proxy::{
    bandwidth_reporter::{BandwidthReporterCodec, BandwidthReporterProtocol},
    tcp_forwarder::node_forward,
    TCP_PROXY_PROTOCOL,
};
use color_eyre::eyre::{self, bail, Context, Result};
use config::Config;
use futures::StreamExt;
use libp2p::{
    dcutr, identify,
    identity::{Keypair, PublicKey},
    multiaddr::{self, Protocol},
    noise, relay,
    request_response::{self, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, tls, yamux, Multiaddr, PeerId, StreamProtocol,
};
use libp2p_stream as stream;
use protocols::auth::v1::{
    authentication_service_client::AuthenticationServiceClient, FederatedApiTokenAuthRequest,
    FederatedAuthenticateRequest,
};
use ratatui::{
    crossterm::event::{self, Event},
    DefaultTerminal, Frame,
};
use sha2::Digest;
use socks_intermediary::run_socks_proxy;
use tokio::{
    select,
    task::{futures::TaskLocalFuture, JoinSet, LocalSet},
};
use tonic::{
    codec::CompressionEncoding,
    transport::{Channel, ClientTlsConfig},
};
use tracing::{debug, error, info, level_filters::LevelFilter, warn};
// use tracing::{error, info, level_filters::LevelFilter};
use crate::wait_ext::SwarmWaitExt;
use tracing_subscriber::EnvFilter;

mod config;
mod socks_intermediary;
mod wait_ext;

struct PeerState {
    connected_peer: Option<PeerId>,
}

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

fn run(mut terminal: DefaultTerminal) -> Result<()> {
    loop {
        terminal.draw(render)?;
        if matches!(event::read()?, Event::Key(_)) {
            break Ok(());
        }
    }
}

fn render(frame: &mut Frame) {
    frame.render_widget("hello world", frame.area());
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env()?,
        )
        .pretty()
        .init();

    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = run(terminal);
    ratatui::restore();

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

    // Now that we've got a solid connection to the Peer, we can start the proxy

    info!("Starting SOCKS5 proxy for peer {}", peer_id);
    tokio::spawn(run_socks_proxy(
        &KEYPAIR,
        token,
        peer_id,
        swarm.behaviour().stream.new_control(),
    ));

    loop {
        // select! {
        if let Some(event) = swarm.next().await {
            match event {
                // libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {}
                // libp2p::swarm::SwarmEvent::NewExternalAddrOfPeer { peer_id, address } => {
                //     // swarm.dial(address)?
                // }
                event => tracing::info!(?event),
            }
        }
        // }
    }
}
