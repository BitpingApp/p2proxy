use std::time::Duration;

use color_eyre::eyre::{Result, bail};
use libp2p::identity::Keypair;
use libp2p::multiaddr::{self, Protocol};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm, identify, noise, relay, tcp, yamux};
use p2p_protocol::client::LibP2pClient;
use proxy_core::events::{ConnectionEvents, Events};
use tokio::sync::mpsc::Sender;
use tracing::{info, warn};

use super::behaviour::{Behaviour, BehaviourEvent};
use crate::utils::wait_ext::SwarmWaitExt;

const MAX_BOOTSTRAP_RETRIES: usize = 10;

/// The connected swarm and the handles the actors need, produced once at
/// startup before the network actor takes ownership of the swarm.
pub struct Bootstrapped {
    pub swarm: Swarm<Behaviour>,
    pub client: LibP2pClient,
    pub stream_control: libp2p_stream::Control,
    pub relay_peer_id: PeerId,
    pub relay_address: Multiaddr,
    pub bootstrap_address: Multiaddr,
    pub bootstrap_peer_id: PeerId,
}

pub async fn bootstrap(
    keypair: Keypair,
    listen_port: u16,
    bootstrap_address: Multiaddr,
    events: &Sender<Events>,
) -> Result<Bootstrapped> {
    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
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
        .with_behaviour(|k, relay_client| Behaviour::new(k.public(), relay_client))?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX)))
        .build();

    let _ = events
        .send(Events::LocalPeerId(*swarm.local_peer_id()))
        .await;

    for addr in listen_addresses(listen_port) {
        swarm.listen_on(addr.clone())?;
        swarm.add_external_address(addr);
    }

    let _ = events
        .send(Events::Connection(ConnectionEvents::Connecting))
        .await;

    info!(%bootstrap_address, "bootstrap hub multiaddr");
    let bootstrap_peer_id = dial_bootstrap(&mut swarm, &bootstrap_address).await?;

    info!("waiting for relay reservation");
    let relay_peer_id = swarm
        .wait_for(|_swarm, event| match event {
            SwarmEvent::Behaviour(BehaviourEvent::Relay(
                relay::client::Event::ReservationReqAccepted { relay_peer_id, .. },
            )) => Some(*relay_peer_id),
            _ => None,
        })
        .await;

    let _ = events
        .send(Events::Connection(ConnectionEvents::Connected(relay_peer_id)))
        .await;
    info!(%relay_peer_id, "reservation accepted");

    let Ok(relay_address) = bootstrap_address.clone().with_p2p(bootstrap_peer_id) else {
        bail!("could not construct relay multiaddr");
    };

    let stream_control = swarm.behaviour().stream.new_control();
    let client = LibP2pClient::new(swarm.behaviour().stream.new_control(), *swarm.local_peer_id());

    Ok(Bootstrapped {
        swarm,
        client,
        stream_control,
        relay_peer_id,
        relay_address,
        bootstrap_address,
        bootstrap_peer_id,
    })
}

fn listen_addresses(port: u16) -> [Multiaddr; 4] {
    [
        multiaddr::multiaddr!(Ip4([0, 0, 0, 0]), Tcp(port)),
        multiaddr::multiaddr!(Ip4([0, 0, 0, 0]), Udp(port), QuicV1),
        multiaddr::multiaddr!(Ip6([0, 0, 0, 0, 0, 0, 0, 0]), Tcp(port)),
        multiaddr::multiaddr!(Ip6([0, 0, 0, 0, 0, 0, 0, 0]), Udp(port), QuicV1),
    ]
}

async fn dial_bootstrap(swarm: &mut Swarm<Behaviour>, bootstrap: &Multiaddr) -> Result<PeerId> {
    let mut retries = 0;
    loop {
        if let Err(e) = swarm.dial(bootstrap.clone()) {
            warn!(?e, "failed to dial bootstrap server");
            retries += 1;
            if retries > MAX_BOOTSTRAP_RETRIES {
                bail!("failed to dial bootstrap server after {MAX_BOOTSTRAP_RETRIES} attempts");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        info!(attempt = retries + 1, "dialing bootstrap server");

        let reached = swarm
            .wait_for_with_timeout(
                |swarm, event| {
                    let SwarmEvent::Behaviour(BehaviourEvent::Identify(
                        identify::Event::Received { peer_id, info, .. },
                    )) = event
                    else {
                        return None;
                    };
                    for addr in &info.listen_addrs {
                        let Ok(with_relay) = addr.clone().with_p2p(*peer_id) else {
                            continue;
                        };
                        let Ok(circuit) = with_relay
                            .with(Protocol::P2pCircuit)
                            .with_p2p(*swarm.local_peer_id())
                        else {
                            continue;
                        };
                        let _ = swarm.listen_on(circuit);
                    }
                    Some(*peer_id)
                },
                Duration::from_secs(10),
            )
            .await;

        match reached {
            Ok(peer_id) => {
                info!("connected to bootstrap server");
                return Ok(peer_id);
            }
            Err(_) => {
                warn!("bootstrap connection timeout");
                retries += 1;
                if retries > MAX_BOOTSTRAP_RETRIES {
                    bail!("failed to connect to bootstrap server after {MAX_BOOTSTRAP_RETRIES} attempts");
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}