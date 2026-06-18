use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use libp2p::{Multiaddr, PeerId};
use proxy_core::config::Server;
use proxy_core::domain::connect::{ConnectCtx, ConnectError, ConnectedDestination};
use proxy_core::events::Events;
use proxy_core::ports::{Actor, EventSink, StickyStore};
use tracing::{debug, info};

use super::event::DiscoveryEvent;
use crate::runtime::context::Context;

pub type DestinationHandle = Arc<ArcSwap<Option<PeerId>>>;

const REDISCOVERY_COOLDOWN: Duration = Duration::from_secs(30);

/// Runs the pure `core::connect` flow against the ports in `Context`, owns the
/// sticky store and the per-port destination handles, and reacts to peer
/// connects (sticky promotion) and closes (throttled rediscovery).
pub struct DiscoveryActor<S: StickyStore> {
    sticky: S,
    destinations: HashMap<u16, DestinationHandle>,
    last_rediscovery: HashMap<u16, Instant>,
    pending_rediscovery: HashSet<u16>,
}

impl<S: StickyStore> DiscoveryActor<S> {
    pub fn new(sticky: S, destinations: HashMap<u16, DestinationHandle>) -> Self {
        Self {
            sticky,
            destinations,
            last_rediscovery: HashMap::new(),
            pending_rediscovery: HashSet::new(),
        }
    }

    async fn discover(&mut self, ctx: &Context, port: u16, avoid: Option<PeerId>) -> Option<PeerId> {
        self.pending_rediscovery.remove(&port);
        let server = ctx.config.servers.iter().find(|s| s.port == port).cloned()?;
        info!(port, "running peer discovery");
        let result = self.connect(ctx, &server, avoid).await;
        self.emit_sticky_pool(ctx, &server);
        match result {
            Ok(destination) => {
                let peer = destination.peer;
                debug!(port, %peer, source = ?destination.source, "discovery adopted exit peer");
                self.set_destination(ctx, port, destination);
                Some(peer)
            }
            Err(e) => {
                debug!(port, error = %e, "discovery found no usable peer");
                self.clear_destination(ctx, port);
                None
            }
        }
    }

    /// Surface the remembered standby pool so the NETWORK tab shows every
    /// sticky exit, not just the active one.
    fn emit_sticky_pool(&self, ctx: &Context, server: &Server) {
        let fingerprint = server.peer_options.filter_fingerprint(server.port);
        let peers = self.sticky.snapshot(server.port, &fingerprint);
        ctx.events.emit(Events::StickyPool {
            port: server.port,
            peers,
        });
    }

    async fn handle_peer_closed(&mut self, ctx: &Context, peer: PeerId) {
        let stale: Vec<u16> = self
            .destinations
            .iter()
            .filter(|(_, handle)| **handle.load() == Some(peer))
            .map(|(port, _)| *port)
            .collect();

        if !stale.is_empty() {
            debug!(%peer, ports = ?stale, "active exit peer disconnected");
        }

        for port in stale {
            let elapsed = self.last_rediscovery.get(&port).map(|at| at.elapsed());
            if let Some(elapsed) = elapsed
                && elapsed < REDISCOVERY_COOLDOWN
            {
                // The peer is gone, so drop the active pointer — but don't
                // abandon the port. Schedule a single rediscovery for when the
                // cooldown lapses so a peer flapping inside the window doesn't
                // leave the port permanently dark.
                self.clear_destination(ctx, port);
                if self.pending_rediscovery.insert(port) {
                    let discovery = ctx.discovery.clone();
                    let remaining = REDISCOVERY_COOLDOWN - elapsed;
                    debug!(
                        port,
                        remaining_ms = remaining.as_millis() as u64,
                        "rediscovery throttled; scheduled after cooldown"
                    );
                    tokio::spawn(async move {
                        tokio::time::sleep(remaining).await;
                        discovery.discover_for(port).await;
                    });
                }
                continue;
            }
            self.last_rediscovery.insert(port, Instant::now());
            self.discover(ctx, port, Some(peer)).await;
        }
    }

    /// Record the real direct address a peer just connected on, so a later
    /// reconnect can dial it straight. No-ops for a peer that isn't already a
    /// remembered exit (e.g. a hub we connect to for bootstrap/relay/discovery).
    fn promote_direct(&mut self, peer: PeerId, address: Multiaddr) {
        debug!(%peer, %address, "recording observed direct address for exit");
        self.sticky.note_direct_address(peer, address);
    }

    /// A peer that can't serve as an exit — it doesn't speak the proxy protocol
    /// (e.g. a hub) or runs an incompatible forwarder. Drop it from every pool
    /// so it's never re-adopted, then rediscover a real exit for any port it was
    /// serving (with no sticky-reconnect back to it).
    async fn handle_peer_unusable(&mut self, ctx: &Context, peer: PeerId) {
        debug!(%peer, "peer cannot serve as an exit; dropping from all pools");
        let servers: Vec<Server> = ctx.config.servers.clone();
        for server in &servers {
            self.sticky.forget_peer(server.port, peer);
        }
        let stale: Vec<u16> = self
            .destinations
            .iter()
            .filter(|(_, handle)| **handle.load() == Some(peer))
            .map(|(port, _)| *port)
            .collect();
        for port in stale {
            self.discover(ctx, port, None).await;
        }
        for server in &servers {
            self.emit_sticky_pool(ctx, server);
        }
    }

    async fn connect(
        &mut self,
        ctx: &Context,
        server: &Server,
        avoid: Option<PeerId>,
    ) -> Result<ConnectedDestination, ConnectError> {
        let gateway = ctx.gateway();
        let mut connect = ConnectCtx {
            directory: &gateway,
            dialer: &gateway,
            clock: &ctx.clock,
            sticky: &mut self.sticky,
            events: &ctx.events,
            relay_address: &ctx.relay_address,
        };
        connect.connect(server, avoid).await
    }

    fn set_destination(&self, ctx: &Context, port: u16, destination: ConnectedDestination) {
        if let Some(handle) = self.destinations.get(&port) {
            handle.store(Arc::new(Some(destination.peer)));
        }
        ctx.events.emit(Events::ActiveDestination {
            port,
            peer: Some(destination.peer),
            source: Some(destination.source),
        });
    }

    fn clear_destination(&self, ctx: &Context, port: u16) {
        if let Some(handle) = self.destinations.get(&port) {
            handle.store(Arc::new(None));
        }
        ctx.events.emit(Events::ActiveDestination {
            port,
            peer: None,
            source: None,
        });
    }
}

impl<S: StickyStore + Send + Sync> Actor for DiscoveryActor<S> {
    type Input = DiscoveryEvent;
    type Output = ();
    type Error = Infallible;
    type Context = Context;

    async fn handle(&mut self, ctx: &Context, event: DiscoveryEvent) -> Result<(), Infallible> {
        match event {
            DiscoveryEvent::DiscoverFor { port } => {
                self.discover(ctx, port, None).await;
            }
            DiscoveryEvent::RequestNewPeer { port, reply } => {
                let peer = self.discover(ctx, port, None).await;
                let _ = reply.send(peer);
            }
            DiscoveryEvent::PeerClosed(peer) => self.handle_peer_closed(ctx, peer).await,
            DiscoveryEvent::PeerUnusable(peer) => self.handle_peer_unusable(ctx, peer).await,
            DiscoveryEvent::PeerConnectedDirect { peer, address } => {
                self.promote_direct(peer, address)
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::DiscoveryHandle;
    use crate::adapters::channel_sink::ChannelSink;
    use crate::adapters::tokio_clock::TokioClock;
    use crate::runtime::network::{NetworkCommand, NetworkHandle};
    use libp2p::identity::Keypair;
    use p2p_protocol::client::LibP2pClient;
    use proxy_core::config::Config;
    use proxy_core::domain::selection::destination_peer_ids;
    use proxy_core::events::{Events, PoolPeer};
    use proxy_core::testing::builders::{direct_addr, discovery_server, peer, relay_addr};
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn dummy_client() -> LibP2pClient {
        let control = libp2p_stream::Behaviour::new().new_control();
        LibP2pClient::new(control, peer())
    }

    fn test_config() -> Arc<Config> {
        Arc::new(Config {
            servers: vec![discovery_server(1080)],
            listen_addrs: vec!["0.0.0.0:0".parse().expect("addr")],
            bitping_api_key: "test-key".into(),
            bootstrap_address: "/dnsaddr/boot2.bitping.com".parse().expect("addr"),
            grpc_url: "https://grpc.bitping.com".into(),
            keypair_path: "node_keypair.bin".into(),
            metrics_port: 9091,
        })
    }

    /// Stands in for the network actor: answers `FindNodes` with `exit`, and
    /// `Dial` with `exit` whenever it is the dialed destination.
    async fn fake_network(
        mut rx: mpsc::Receiver<NetworkCommand>,
        exit: PeerId,
        addr: libp2p::Multiaddr,
    ) {
        while let Some(command) = rx.recv().await {
            match command {
                NetworkCommand::FindNodes { reply, .. } => {
                    let _ = reply.send(Ok(vec![PoolPeer {
                        peer_id: exit,
                        addresses: vec![addr.clone()],
                    }]));
                }
                NetworkCommand::Dial { addresses, reply } => {
                    let hit = destination_peer_ids(&addresses).contains(&exit).then_some(exit);
                    let _ = reply.send(Ok(hit));
                }
                NetworkCommand::ResolvePeers { reply, .. } => {
                    let _ = reply.send(Ok(Default::default()));
                }
                NetworkCommand::IsConnected { reply, .. } => {
                    let _ = reply.send(false);
                }
                NetworkCommand::NotifyBandwidth { .. } => {}
            }
        }
    }

    fn context(net: NetworkHandle, events: ChannelSink, config: Arc<Config>) -> Context {
        Context {
            config,
            keypair: Arc::new(Keypair::generate_ed25519()),
            token: "token".into(),
            relay_peer_id: peer(),
            relay_address: relay_addr(),
            bootstrap_peer_id: peer(),
            bootstrap_address: relay_addr(),
            client: dummy_client(),
            events,
            network: net,
            discovery: DiscoveryHandle::new(mpsc::channel(1).0),
            streams: Arc::new(crate::runtime::stream_manager::PeerStreamManager::new(
                libp2p_stream::Behaviour::new().new_control(),
                5,
                Duration::from_secs(5),
            )),
            clock: TokioClock,
        }
    }

    /// Drives the real `DiscoveryActor::handle` → `core::connect` → `SwarmGateway`
    /// → `NetworkHandle` round-trip against a fake network, asserting the adopted
    /// peer lands in the destination handle and surfaces an `ActiveDestination`.
    #[tokio::test]
    async fn discovery_actor_adopts_a_discovered_peer() {
        let exit = peer();
        let (net_tx, net_rx) = mpsc::channel::<NetworkCommand>(16);
        let (ev_tx, mut ev_rx) = mpsc::channel::<Events>(64);
        tokio::spawn(fake_network(net_rx, exit, direct_addr(exit)));

        let ctx = context(
            NetworkHandle::new(net_tx),
            ChannelSink::new(ev_tx),
            test_config(),
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let destination: DestinationHandle = Arc::new(ArcSwap::from_pointee(None));
        let mut destinations = HashMap::new();
        destinations.insert(1080u16, destination.clone());
        let mut actor = DiscoveryActor::new(
            crate::adapters::file_sticky::FileStickyStore::load(dir.path().join("sticky.json")),
            destinations,
        );

        let _ = actor
            .handle(&ctx, DiscoveryEvent::DiscoverFor { port: 1080 })
            .await;

        assert_eq!(**destination.load(), Some(exit), "destination adopted");

        let mut saw_active = false;
        let mut saw_sticky_pool = false;
        while let Ok(event) = ev_rx.try_recv() {
            match event {
                Events::ActiveDestination { peer: Some(p), .. } if p == exit => saw_active = true,
                Events::StickyPool { port: 1080, peers }
                    if peers.iter().any(|pp| pp.peer_id == exit) =>
                {
                    saw_sticky_pool = true;
                }
                _ => {}
            }
        }
        assert!(saw_active, "ActiveDestination surfaced to the TUI");
        assert!(
            saw_sticky_pool,
            "remembered sticky pool surfaced to the NETWORK tab"
        );
    }
}
