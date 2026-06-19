use std::collections::{HashMap, HashSet};

use libp2p::{Multiaddr, PeerId};
use proxy_core::config::Server;
use proxy_core::ports::{DialError, DirectoryError};
use proxy_core::events::PoolPeer;
use proxy_core::ports::{Dialer, PeerDirectory};

use crate::runtime::network::NetworkHandle;

/// Implements the discovery + dialing ports by forwarding to the network actor.
pub struct SwarmGateway {
    net: NetworkHandle,
}

impl SwarmGateway {
    pub fn new(net: NetworkHandle) -> Self {
        Self { net }
    }
}

impl PeerDirectory for SwarmGateway {
    async fn resolve_peers(
        &self,
        peers: &[PeerId],
    ) -> Result<HashMap<PeerId, Vec<Multiaddr>>, DirectoryError> {
        self.net.resolve_peers(peers.to_vec()).await
    }

    async fn find_nodes(
        &self,
        server: &Server,
        limit: usize,
    ) -> Result<Vec<PoolPeer>, DirectoryError> {
        self.net
            .find_nodes(server.peer_options.node_filters(), limit)
            .await
    }
}

impl Dialer for SwarmGateway {
    async fn dial_and_wait(
        &self,
        addresses: HashSet<Multiaddr>,
    ) -> Result<Option<PeerId>, DialError> {
        self.net.dial(addresses).await
    }

    async fn is_connected(&self, peer: PeerId) -> bool {
        self.net.is_connected(peer).await
    }
}
