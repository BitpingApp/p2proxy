use libp2p::{Multiaddr, PeerId};
use tokio::sync::{mpsc, oneshot};

/// Inputs to the discovery actor. The network actor reports peer
/// connects/closes; servers ask for initial or just-in-time destinations.
pub enum DiscoveryEvent {
    DiscoverFor {
        port: u16,
    },
    RequestNewPeer {
        port: u16,
        reply: oneshot::Sender<Option<PeerId>>,
    },
    PeerClosed(PeerId),
    /// A peer that can't serve as an exit (doesn't speak the proxy protocol, or
    /// runs an incompatible forwarder). Forget it from every pool and rediscover
    /// — never reconnect to it.
    PeerUnusable(PeerId),
    PeerConnectedDirect {
        peer: PeerId,
        address: Multiaddr,
    },
}

#[derive(Clone)]
pub struct DiscoveryHandle {
    tx: mpsc::Sender<DiscoveryEvent>,
}

impl DiscoveryHandle {
    pub fn new(tx: mpsc::Sender<DiscoveryEvent>) -> Self {
        Self { tx }
    }

    pub async fn discover_for(&self, port: u16) {
        let _ = self.tx.send(DiscoveryEvent::DiscoverFor { port }).await;
    }

    pub async fn request_new_peer(&self, port: u16) -> Option<PeerId> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(DiscoveryEvent::RequestNewPeer { port, reply })
            .await
            .is_err()
        {
            return None;
        }
        rx.await.ok().flatten()
    }

    pub async fn peer_closed(&self, peer: PeerId) {
        let _ = self.tx.send(DiscoveryEvent::PeerClosed(peer)).await;
    }

    pub async fn peer_unusable(&self, peer: PeerId) {
        let _ = self.tx.send(DiscoveryEvent::PeerUnusable(peer)).await;
    }

    pub async fn peer_connected_direct(&self, peer: PeerId, address: Multiaddr) {
        let _ = self
            .tx
            .send(DiscoveryEvent::PeerConnectedDirect { peer, address })
            .await;
    }
}
