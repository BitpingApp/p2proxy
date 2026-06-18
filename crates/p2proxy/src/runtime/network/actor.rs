use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::time::Duration;

use bitping_swarm::auth::Auth;
use bitping_swarm::query::{MAX_RESOLVE_PEERS, QueryRequest, QueryResponse};
use futures::StreamExt;
use libp2p::{PeerId, identify, ping, relay, swarm::SwarmEvent};
use metrics::{counter, gauge, histogram};
use p2p_protocol::P2pClient;
use protocols::models::v1::{Bandwidth, Exclusions, Requirements};
use proxy_core::domain::selection::destination_peer_ids;
use proxy_core::errors::{DialError, DirectoryError};
use proxy_core::events::{ConnectionEvents, Events, PoolPeer};
use proxy_core::ports::{Actor, EventSink};
use tokio::sync::mpsc::Receiver;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::behaviour::{Behaviour, BehaviourEvent};
use super::command::NetworkCommand;
use crate::runtime::context::Context;

const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
const HUB_QUERY_TIMEOUT: Duration = Duration::from_secs(5);
/// Consecutive ping failures tolerated before tearing a connection down. One
/// missed ping is a transient blip (a dropped UDP packet on an idle NAT path);
/// disconnecting on it forces a needless reconnect. We only give up once a peer
/// is unreachable across several pings.
const MAX_PING_STRIKES: u32 = 3;

struct PendingDial {
    candidates: HashSet<PeerId>,
    reply: oneshot::Sender<Result<Option<PeerId>, DialError>>,
    deadline: Instant,
}

/// Inputs the network actor reacts to. The runtime's `drive_network` is the only
/// producer — it owns the swarm stream and the command inbox.
pub enum NetworkInput {
    Command(NetworkCommand),
    Swarm(SwarmEvent<BehaviourEvent>),
    ExpireDials,
    RetryBootstrap,
    Shutdown,
}

/// Sole owner of the libp2p swarm. Holds only the state intrinsic to driving it
/// — the swarm itself, in-flight dials, and the bootstrap-link flags. Everything
/// shared (client, keypair, token, relay/bootstrap addresses, event sink,
/// sibling handles) comes from the `Context`.
pub struct NetworkActor {
    swarm: libp2p::Swarm<Behaviour>,
    pending_dials: Vec<PendingDial>,
    bootstrap_connected: bool,
    bootstrap_dialing: bool,
    ping_strikes: HashMap<PeerId, u32>,
}

impl NetworkActor {
    pub fn new(swarm: libp2p::Swarm<Behaviour>) -> Self {
        Self {
            swarm,
            pending_dials: Vec::new(),
            bootstrap_connected: true,
            bootstrap_dialing: false,
            ping_strikes: HashMap::new(),
        }
    }

    fn handle_command(&mut self, ctx: &Context, command: NetworkCommand) {
        match command {
            NetworkCommand::ResolvePeers { peers, reply } => {
                let capped: Vec<PeerId> = peers.into_iter().take(MAX_RESOLVE_PEERS).collect();
                let request = match Auth::new(
                    QueryRequest::ResolvePeers(capped),
                    &ctx.keypair,
                    ctx.token.clone(),
                ) {
                    Ok(request) => request,
                    Err(e) => {
                        let _ = reply.send(Err(DirectoryError::Unsupported(format!(
                            "failed to sign request: {e}"
                        ))));
                        return;
                    }
                };
                let client = ctx.client.clone();
                let relay = ctx.relay_peer_id;
                tokio::spawn(async move {
                    let mapped = match client
                        .ask_with_timeout::<Auth<QueryRequest>>(relay, request, HUB_QUERY_TIMEOUT)
                        .await
                    {
                        Ok(QueryResponse::FindNodes(set)) => {
                            Ok(set.into_iter().map(|p| (p.peer_id, p.addresses)).collect())
                        }
                        Ok(QueryResponse::Error(e)) => Err(DirectoryError::Unsupported(e)),
                        Ok(QueryResponse::FindNode(_)) => {
                            Err(DirectoryError::Rejected("expected FindNodes".into()))
                        }
                        Err(e) => Err(DirectoryError::Unsupported(e.to_string())),
                    };
                    let _ = reply.send(mapped);
                });
            }
            NetworkCommand::FindNodes {
                country,
                min_bandwidth_bps,
                limit,
                reply,
            } => {
                let mut requirements = Requirements::default();
                if let Some(country) = country {
                    requirements.countries = vec![country];
                }
                let exclusions = Exclusions {
                    bandwidth: Some(Bandwidth {
                        less_than: Some(min_bandwidth_bps as f64),
                        greater_than: None,
                    }),
                    ..Default::default()
                };
                let request = match Auth::new(
                    QueryRequest::FindNodes {
                        requirements: Some(requirements),
                        exclusions: Some(exclusions),
                        capabilities: None,
                        limit: limit.min(u16::MAX as usize) as u16,
                    },
                    &ctx.keypair,
                    ctx.token.clone(),
                ) {
                    Ok(request) => request,
                    Err(e) => {
                        let _ = reply.send(Err(DirectoryError::Rejected(format!(
                            "failed to sign request: {e}"
                        ))));
                        return;
                    }
                };
                let client = ctx.client.clone();
                let relay = ctx.relay_peer_id;
                tokio::spawn(async move {
                    let mapped = match client
                        .ask_with_timeout::<Auth<QueryRequest>>(relay, request, HUB_QUERY_TIMEOUT)
                        .await
                    {
                        Ok(QueryResponse::FindNodes(set)) => Ok(set
                            .into_iter()
                            .map(|p| PoolPeer {
                                peer_id: p.peer_id,
                                addresses: p.addresses,
                            })
                            .collect()),
                        Ok(QueryResponse::Error(e)) => Err(DirectoryError::Rejected(e)),
                        Ok(QueryResponse::FindNode(_)) => {
                            Err(DirectoryError::Rejected("expected FindNodes".into()))
                        }
                        Err(_) => Err(DirectoryError::Timeout),
                    };
                    let _ = reply.send(mapped);
                });
            }
            NetworkCommand::Dial { addresses, reply } => {
                let candidates = destination_peer_ids(&addresses);
                if candidates.is_empty() {
                    let _ = reply.send(Ok(None));
                    return;
                }
                for addr in &addresses {
                    if let Err(e) = self.swarm.dial(addr.clone()) {
                        warn!(?e, %addr, "failed to dial candidate");
                    }
                }
                self.pending_dials.push(PendingDial {
                    candidates,
                    reply,
                    deadline: Instant::now() + DIAL_TIMEOUT,
                });
            }
            NetworkCommand::IsConnected { peer, reply } => {
                let _ = reply.send(self.swarm.is_connected(&peer));
            }
            NetworkCommand::NotifyBandwidth { report } => {
                let client = ctx.client.clone();
                let relay = ctx.relay_peer_id;
                tokio::spawn(async move {
                    if let Err(e) = client.notify(relay, report).await {
                        warn!(?e, "bandwidth report notify failed");
                    }
                });
            }
        }
    }

    fn handle_swarm_event(&mut self, ctx: &Context, event: SwarmEvent<BehaviourEvent>) {
        match event {
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                counter!("p2proxy_peer_connections_total").increment(1);
                gauge!("p2proxy_peers_connected").increment(1.0);
                self.complete_pending_dials(peer_id);

                let relayed = endpoint.is_relayed();
                let address = endpoint.get_remote_address().clone();
                if peer_id == ctx.bootstrap_peer_id {
                    self.bootstrap_connected = true;
                    self.bootstrap_dialing = false;
                }
                ctx.events
                    .emit(Events::Connection(ConnectionEvents::Connected(peer_id)));
                ctx.events.emit(Events::PeerRoute {
                    peer_id,
                    address: address.clone(),
                    relayed,
                });
                if !relayed && peer_id != ctx.bootstrap_peer_id {
                    let discovery = ctx.discovery.clone();
                    tokio::spawn(async move {
                        discovery.peer_connected_direct(peer_id, address).await;
                    });
                }
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                gauge!("p2proxy_peers_connected").decrement(1.0);
                self.ping_strikes.remove(&peer_id);
                info!(%peer_id, ?cause, "connection closed");
                if peer_id == ctx.bootstrap_peer_id {
                    self.bootstrap_connected = false;
                    self.bootstrap_dialing = false;
                }
                ctx.events
                    .emit(Events::Connection(ConnectionEvents::Disconnected(peer_id)));
                let discovery = ctx.discovery.clone();
                tokio::spawn(async move {
                    discovery.peer_closed(peer_id).await;
                });
            }
            SwarmEvent::OutgoingConnectionError { peer_id, .. } => {
                if peer_id == Some(ctx.bootstrap_peer_id) {
                    self.bootstrap_dialing = false;
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Ping(ping::Event {
                peer,
                result: Err(failure),
                ..
            })) => {
                counter!("p2proxy_ping_failures_total", "peer" => peer.to_string()).increment(1);
                let strikes = self.ping_strikes.entry(peer).or_insert(0);
                *strikes += 1;
                if *strikes < MAX_PING_STRIKES {
                    warn!(%peer, ?failure, strikes = *strikes, "ping failure — tolerating transient blip");
                } else {
                    warn!(%peer, ?failure, strikes = *strikes, "ping failed repeatedly — disconnecting peer");
                    self.ping_strikes.remove(&peer);
                    let _ = self.swarm.disconnect_peer_id(peer);
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Ping(ping::Event {
                peer,
                result: Ok(rtt),
                ..
            })) => {
                self.ping_strikes.remove(&peer);
                histogram!("p2proxy_ping_rtt_seconds", "peer" => peer.to_string())
                    .record(rtt.as_secs_f64());
            }
            SwarmEvent::Behaviour(BehaviourEvent::Relay(
                relay::client::Event::ReservationReqAccepted { .. },
            )) => {
                counter!("p2proxy_relay_reservations_total").increment(1);
            }
            SwarmEvent::Behaviour(BehaviourEvent::Dcutr(event)) => {
                counter!("p2proxy_dcutr_events_total").increment(1);
                debug!(?event, "dcutr event");
            }
            SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                ..
            })) => {
                counter!("p2proxy_peer_identified_total").increment(1);
                debug!(%peer_id, "identified peer");
            }
            SwarmEvent::NewListenAddr { address, .. } => {
                debug!(%address, "listening");
            }
            _ => {}
        }
    }

    fn complete_pending_dials(&mut self, peer_id: PeerId) {
        let mut i = 0;
        while i < self.pending_dials.len() {
            if self.pending_dials[i].candidates.contains(&peer_id) {
                let pending = self.pending_dials.swap_remove(i);
                let _ = pending.reply.send(Ok(Some(peer_id)));
            } else {
                i += 1;
            }
        }
    }

    fn expire_dials(&mut self) {
        let now = Instant::now();
        let mut i = 0;
        while i < self.pending_dials.len() {
            if self.pending_dials[i].deadline <= now {
                let pending = self.pending_dials.swap_remove(i);
                let _ = pending.reply.send(Ok(None));
            } else {
                i += 1;
            }
        }
    }

    fn try_dial_bootstrap(&mut self, ctx: &Context) {
        if self.bootstrap_connected || self.bootstrap_dialing {
            return;
        }
        match self.swarm.dial(ctx.bootstrap_address.clone()) {
            Ok(()) => {
                self.bootstrap_dialing = true;
                debug!("bootstrap dial initiated");
            }
            Err(e) => warn!(?e, "failed to dial bootstrap server"),
        }
    }

    async fn graceful_shutdown(&mut self) {
        let peers: Vec<PeerId> = self.swarm.connected_peers().copied().collect();
        info!(count = peers.len(), "disconnecting peers");
        for peer in peers {
            let _ = self.swarm.disconnect_peer_id(peer);
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if self.swarm.connected_peers().next().is_none() {
                return;
            }
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => return,
                _ = self.swarm.next() => {}
            }
        }
    }
}

impl Actor for NetworkActor {
    type Input = NetworkInput;
    type Output = ();
    type Error = Infallible;
    type Context = Context;

    async fn handle(&mut self, ctx: &Context, input: NetworkInput) -> Result<(), Infallible> {
        match input {
            NetworkInput::Command(command) => self.handle_command(ctx, command),
            NetworkInput::Swarm(event) => self.handle_swarm_event(ctx, event),
            NetworkInput::ExpireDials => self.expire_dials(),
            NetworkInput::RetryBootstrap => self.try_dial_bootstrap(ctx),
            NetworkInput::Shutdown => self.graceful_shutdown().await,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::channel_sink::ChannelSink;
    use crate::runtime::discovery::DiscoveryHandle;
    use crate::runtime::network::NetworkHandle;
    use crate::runtime::testutil::{dummy_context, memory_swarm};
    use std::collections::HashSet;
    use tokio::sync::mpsc;

    /// Drives the real network actor over an in-memory libp2p transport: a peer
    /// swarm listens, the actor dials it on command, and the resulting
    /// `ConnectionEstablished` completes the pending dial.
    #[tokio::test]
    async fn dials_a_peer_and_reports_the_connection() {
        let mut peer_swarm = memory_swarm();
        let peer_id = *peer_swarm.local_peer_id();
        peer_swarm
            .listen_on("/memory/0".parse().expect("addr"))
            .expect("listen");
        let listen_addr = loop {
            if let SwarmEvent::NewListenAddr { address, .. } = peer_swarm.select_next_some().await {
                break address;
            }
        };
        tokio::spawn(async move {
            loop {
                let _ = peer_swarm.select_next_some().await;
            }
        });

        let (net_tx, net_rx) = mpsc::channel::<NetworkCommand>(8);
        let (ev_tx, _ev_rx) = mpsc::channel(16);
        let ctx = dummy_context(
            NetworkHandle::new(net_tx.clone()),
            DiscoveryHandle::new(mpsc::channel(1).0),
            ChannelSink::new(ev_tx),
        );
        let shutdown = CancellationToken::new();
        tokio::spawn(drive_network(
            NetworkActor::new(memory_swarm()),
            net_rx,
            ctx,
            shutdown.clone(),
        ));

        let net = NetworkHandle::new(net_tx);
        let target = listen_addr.with_p2p(peer_id).expect("with_p2p");
        let reached = net.dial(HashSet::from([target])).await.expect("dial ok");
        assert_eq!(reached, Some(peer_id), "dialed peer adopted");
        assert!(net.is_connected(peer_id).await, "connection is live");

        shutdown.cancel();
    }
}

/// The runtime's swarm driver: owns the swarm stream + command inbox and feeds
/// each into `NetworkActor::handle`. This is the loop — the actor stays passive.
pub async fn drive_network(
    mut actor: NetworkActor,
    mut commands: Receiver<NetworkCommand>,
    ctx: Context,
    shutdown: CancellationToken,
) {
    let mut expire = tokio::time::interval(Duration::from_secs(1));
    expire.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut bootstrap = tokio::time::interval(Duration::from_secs(5));
    bootstrap.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let _ = actor.handle(&ctx, NetworkInput::RetryBootstrap).await;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                let _ = actor.handle(&ctx, NetworkInput::Shutdown).await;
                return;
            }
            Some(event) = actor.swarm.next() => {
                let _ = actor.handle(&ctx, NetworkInput::Swarm(event)).await;
            }
            Some(command) = commands.recv() => {
                let _ = actor.handle(&ctx, NetworkInput::Command(command)).await;
            }
            _ = expire.tick() => {
                let _ = actor.handle(&ctx, NetworkInput::ExpireDials).await;
            }
            _ = bootstrap.tick() => {
                let _ = actor.handle(&ctx, NetworkInput::RetryBootstrap).await;
            }
        }
    }
}
