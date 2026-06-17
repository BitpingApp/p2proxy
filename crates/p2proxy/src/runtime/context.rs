use std::sync::Arc;

use libp2p::identity::Keypair;
use libp2p::{Multiaddr, PeerId};
use p2p_protocol::client::LibP2pClient;
use proxy_core::config::Config;

use crate::adapters::channel_sink::ChannelSink;
use crate::adapters::swarm_gateway::SwarmGateway;
use crate::adapters::tokio_clock::TokioClock;
use crate::runtime::discovery::DiscoveryHandle;
use crate::runtime::network::NetworkHandle;
use crate::runtime::stream_manager::PeerStreamManager;

/// Shared dependencies handed to every actor's `handle`. Built once at startup,
/// cheap to clone (Arcs + channel senders). Actors read from it; their own
/// mutable state stays on `self`.
#[derive(Clone)]
pub struct Context {
    pub config: Arc<Config>,
    pub keypair: Arc<Keypair>,
    pub token: String,
    pub relay_peer_id: PeerId,
    pub relay_address: Multiaddr,
    pub bootstrap_peer_id: PeerId,
    pub bootstrap_address: Multiaddr,
    pub client: LibP2pClient,
    pub events: ChannelSink,
    pub network: NetworkHandle,
    pub discovery: DiscoveryHandle,
    pub streams: Arc<PeerStreamManager>,
    pub clock: TokioClock,
}

impl Context {
    /// The discovery/dialing port over the network actor.
    pub fn gateway(&self) -> SwarmGateway {
        SwarmGateway::new(self.network.clone())
    }
}
