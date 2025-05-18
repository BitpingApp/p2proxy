use std::collections::HashSet;
use std::{sync::LazyLock, time::Duration};

use crate::config::Server;
use crate::proxy_protocols::socks_stream::{DataDirection, SocksStreamMessage};
use crate::utils::wait_ext::SwarmWaitExt;
use crate::CONFIG;
use crate::{proxy_protocols, GRPC_CHANNEL};
use bitping_swarm::auth::Auth;
use bitping_swarm::query::{QueryCodec, QueryProtocol, QueryRequest};
use bitping_tcp_proxy::bandwidth_reporter::{BandwidthReporterCodec, BandwidthReporterProtocol};
use color_eyre::eyre::{bail, ensure, eyre, Context, Result};
use color_eyre::owo_colors::OwoColorize;
use futures::StreamExt;
use libp2p::request_response::Message;
use libp2p::Multiaddr;
use libp2p::{
    dcutr, identify,
    identity::{Keypair, PublicKey},
    multiaddr::{self, Protocol},
    noise, relay,
    request_response::{self, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, PeerId, Swarm,
};
use libp2p_stream as stream;
use models::events::Events;
use protocols::auth::v1::{
    authentication_service_client::AuthenticationServiceClient, FederatedApiTokenAuthRequest,
};
use protocols::models::v1::Requirements;
use sha2::Digest;
use tokio::sync::mpsc::{self, Sender};
use tokio::time::Timeout;
use tonic::codec::CompressionEncoding;
use tracing::{debug, info, instrument, warn};

#[derive(NetworkBehaviour)]
struct Behaviour {
    stream: stream::Behaviour,
    dcutr: dcutr::Behaviour,
    relay: relay::client::Behaviour,
    identify: identify::Behaviour,
    bandwidth_reporter: request_response::Behaviour<BandwidthReporterCodec>,
    query: request_response::Behaviour<QueryCodec>,
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
            query: request_response::Behaviour::new(
                [(QueryProtocol, ProtocolSupport::Outbound)],
                request_response::Config::default().with_max_concurrent_streams(1000),
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
}

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
        let response = auth_client
            .federated_api_token_authenticate(tonic::Request::new(FederatedApiTokenAuthRequest {
                api_token: CONFIG.bitping_api_key.to_string(),
                signature: base58_monero::encode_check(&signature)?,
                public_key: base58_monero::encode_check(&kp.public().to_bytes())?,
            }))
            .await?;

        Ok(ProxyNetwork(NetworkConnect {
            token: response.into_inner().token,
        }))
    }
}

impl ProxyNetwork<NetworkConnect> {
    pub async fn with_swarm(
        self,
        event_send: Sender<Events>,
    ) -> Result<ProxyNetwork<Bootstrapped>> {
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
        let bootstrap = multiaddr::multiaddr!(Dnsaddr("boot1.bitping.com"));
        swarm.dial(bootstrap.clone())?;

        let bootstrap_peer_id = swarm
            .wait_for(|swarm, event| {
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

        // Store relay peer ID in app state
        let _ = event_send
            .send(Events::Connection(
                models::events::ConnectionEvents::Connected(relay_peer_id),
            ))
            .await;

        info!(%relay_peer_id, %renewal, ?limit, "Reservation accepted, time to connect to peer.");

        let Ok(relay_address) = bootstrap.with_p2p(bootstrap_peer_id) else {
            bail!("Could not construct relay multiaddr")
        };

        Ok(ProxyNetwork(Bootstrapped {
            swarm,
            token: self.0.token,
            event_send,
            relay_address,
            relay_peer_id,
            proxy_message_channel: mpsc::channel(1000),
        }))
    }
}

impl ProxyNetwork<Bootstrapped> {
    pub async fn configure_server(&mut self, server: &'static Server) -> Result<()> {
        let destination_peer_id = self.discover_and_connect_to_peer(server).await?;

        info!(
            ?destination_peer_id,
            "Connection established with destination peer"
        );

        proxy_protocols::socks_stream::create_socks_proxy_stream(
            server,
            &KEYPAIR,
            self.0.token.to_string(),
            destination_peer_id,
            self.0.swarm.behaviour().stream.new_control(),
            self.0.proxy_message_channel.0.clone(),
        )
        .await?;

        Ok(())
    }

    async fn discover_and_connect_to_peer(
        &mut self,
        server: &Server,
    ) -> Result<PeerId, color_eyre::eyre::Error> {
        let mut retry_count = 0;
        const MAX_RETRIES: usize = 20;

        while retry_count < MAX_RETRIES {
            info!(
                "Looking up peer (attempt {}/{})",
                retry_count + 1,
                MAX_RETRIES
            );

            // 1. Discover peers
            let destination_addresses = match self.discover_peer(server).await {
                Ok(addresses) => {
                    if addresses.is_empty() {
                        warn!("No peer addresses discovered");
                        retry_count += 1;
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                    addresses
                }
                Err(e) => {
                    warn!(?e, "Failed to discover peer");
                    retry_count += 1;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            // 2. Dial all peers
            for addr in destination_addresses {
                match self.0.swarm.dial(addr.clone()) {
                    Ok(_) => info!(?addr, "Dialing peer"),
                    Err(e) => warn!(?e, ?addr, "Failed to dial peer"),
                }
            }

            // 3. Wait for any ConnectionEstablished event
            match self
                .0
                .swarm
                .wait_for_with_timeout(
                    |_, event| {
                        if let SwarmEvent::ConnectionEstablished {
                            peer_id,
                            connection_id,
                            endpoint,
                            num_established,
                            concurrent_dial_errors,
                            established_in,
                        } = event
                        {
                            info!(
                                ?peer_id,
                                ?connection_id,
                                ?endpoint,
                                ?num_established,
                                ?concurrent_dial_errors,
                                ?established_in,
                                "Connected to peer"
                            );
                            return Some(*peer_id);
                        }
                        None
                    },
                    Duration::from_secs(10),
                )
                .await
            {
                Ok(peer_id) => return Ok(peer_id),
                Err(_) => {
                    warn!("Connection timeout reached");
                    retry_count += 1;
                }
            }
        }

        bail!(
            "Failed to connect with any peer after {} attempts",
            MAX_RETRIES
        );
    }

    #[instrument(skip(self))]
    async fn discover_peer(
        &mut self,
        server: &Server,
    ) -> Result<HashSet<Multiaddr>, color_eyre::eyre::Error> {
        let destination_address = if let Some(destination_peer) =
            &server.peer_options.destination_peer
        {
            if let Some(Protocol::P2p(_)) = destination_peer.iter().next() {
                info!("Trying to connect to destination peer");
                // Case 1: It starts with a P2p protocol, append it to the relay address
                HashSet::from_iter(vec![self
                    .0
                    .relay_address
                    .clone()
                    .with(Protocol::P2pCircuit)
                    .with_p2p(
                        if let Protocol::P2p(peer_id) = destination_peer.iter().next().unwrap() {
                            peer_id
                        } else {
                            unreachable!()
                        },
                    )
                    .unwrap()])
            } else {
                // Case 2: It's a fully formed multiaddr that doesn't start with P2p, use it directly
                HashSet::from_iter(vec![destination_peer.clone()])
            }
        } else {
            let mut node_reqs = Requirements::default();
            if let Some(c) = &server.peer_options.country {
                node_reqs.countries = vec![c.clone()];
            }

            let request = Auth::new(
                QueryRequest::FindNodes {
                    requirements: Some(node_reqs),
                    exclusions: None,
                    capabilities: None,
                    limit: 25,
                },
                &KEYPAIR,
                self.0.token.clone(),
            )?;

            let outbound_request_id = self
                .0
                .swarm
                .behaviour_mut()
                .query
                .send_request(&self.0.relay_peer_id, request);

            let peer_ids = self
                .0
                .swarm
                .wait_for_with_timeout(
                    move |swarm, event| match event {
                        SwarmEvent::Behaviour(BehaviourEvent::Query(
                            request_response::Event::Message {
                                peer,
                                connection_id,
                                message:
                                    Message::Response {
                                        request_id,
                                        response,
                                    },
                            },
                        )) if *request_id == outbound_request_id => match response {
                            bitping_swarm::query::QueryResponse::Error(e) => {
                                Some(Err(eyre!(e.clone())))
                            }
                            bitping_swarm::query::QueryResponse::FindNode(peer_id) => {
                                Some(Err(eyre!(
                                    "Got wrong query response, expected FindNodes, got: FindNode"
                                )))
                            }
                            bitping_swarm::query::QueryResponse::FindNodes(hash_set) => {
                                Some(Ok(hash_set.clone()))
                            }
                        },
                        _ => None,
                    },
                    Duration::from_secs(5),
                )
                .await??;

            info!(?peer_ids, "Successfully looked up destination peer");

            // Case 3: No destination peer specified, use the peer_id from query
            // TODO: No unwraps

            peer_ids
                .into_iter()
                .filter_map(|peer_id| {
                    self.0
                        .relay_address
                        .clone()
                        .with(Protocol::P2pCircuit)
                        .with_p2p(peer_id)
                        .ok()
                })
                .collect::<HashSet<Multiaddr>>()
        };
        Ok(destination_address)
    }

    pub async fn drive_network(mut self) -> Result<()> {
        // Main event loop
        loop {
            tokio::select! {
                Some(message) = self.0.proxy_message_channel.1.recv() => {
                    if let Err(e) = self.handle_proxy_events(message).await {
                        warn!(?e, "Something went wrong handling proxy events");
                    }
                },
                Some(event) = self.0.swarm.next() => {
                    handle_swarm_events(event);
                }
            };
        }
    }

    async fn handle_proxy_events(&mut self, message: SocksStreamMessage) -> Result<()> {
        match message {
            SocksStreamMessage::Initialized {
                session_id,
                target_addr,
                peer,
            } => {
                debug!(
                    "New session: {} to {:?} via {}",
                    session_id, target_addr, peer
                );
            }
            SocksStreamMessage::DataTransferred {
                session_id,
                direction,
                bytes,
            } => {
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
                debug!(
                    ?session_id,
                    ?incoming_hash,
                    ?outgoing_hash,
                    ?report,
                    "Session finished.",
                );

                let token = self.0.token.clone();
                if let Ok(authed_report) = Auth::new(report, &KEYPAIR, token) {
                    let authed_report = authed_report.clone();
                    self.0
                        .swarm
                        .behaviour_mut()
                        .bandwidth_reporter
                        .send_request(&self.0.relay_peer_id, authed_report);
                }
            }
            SocksStreamMessage::RequestNewPeer {
                callback,
                server_config,
            } => match self.discover_and_connect_to_peer(server_config).await {
                Ok(p) => {
                    let _ = callback
                        .send(p)
                        .map_err(|p| eyre!("Failed to send new peer back to stream {p}"))?;
                }
                e => {
                    let _ = e.wrap_err("Failed to discover peer after connection dropped")?;
                }
            },
        }

        Ok(())
    }
}

fn handle_swarm_events(event: SwarmEvent<BehaviourEvent>) {
    match event {
        SwarmEvent::ConnectionEstablished {
            peer_id, endpoint, ..
        } => {
            // APP_STATE
            //     .update(|state| {
            //         let address = endpoint.get_remote_address().clone();
            //         let is_relay = state.relay_peer_id == Some(peer_id);
            //         state.peers.insert(
            //             peer_id,
            //             PeerInfo {
            //                 address,
            //                 connected_at: Instant::now(),
            //                 is_relay,
            //             },
            //         );
            //         state.connection_status = ConnectionStatus::Connected;
            //         // Update metrics
            //         gauge!("p2proxy_peers_connected").set(state.peers.len() as f64);
            //     })
            //     .await;

            info!("Connection established with peer: {}", peer_id);
        }
        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            // APP_STATE
            //     .update(|state| {
            //         state.peers.remove(&peer_id);

            //         // Update connection status if all peers are gone
            //         if state.peers.is_empty() {
            //             state.connection_status = ConnectionStatus::Disconnected;
            //         }

            //         info!("Connection closed with peer: {}", peer_id);

            //         // Update metrics
            //         gauge!("p2proxy_peers_connected").set(state.peers.len() as f64);
            //     })
            //     .await;
        }
        SwarmEvent::NewListenAddr { address, .. } => {
            debug!("Listening on: {}", address);
        }
        SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received {
            peer_id,
            info,
            ..
        })) => {
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
            debug!("Relay reservation accepted: {}", relay_peer_id);
        }
        SwarmEvent::Behaviour(BehaviourEvent::Dcutr(e)) => {
            info!(?e, "Dcutr event");
        }
        event => {
            debug!(?event, "Other event received");
        }
    }
}
