use std::collections::{HashMap, HashSet};

use libp2p::{Multiaddr, PeerId};
use p2p_bandwidth_protocol::bandwidth_reporter::AuthedBandwidthReport;
use proxy_core::errors::{DialError, DirectoryError};
use proxy_core::events::PoolPeer;
use tokio::sync::{mpsc, oneshot};

use super::command::NetworkCommand;

/// Cloneable command sender for the network actor. Adapters hold one and turn
/// port calls into commands.
#[derive(Clone)]
pub struct NetworkHandle {
    tx: mpsc::Sender<NetworkCommand>,
}

impl NetworkHandle {
    pub fn new(tx: mpsc::Sender<NetworkCommand>) -> Self {
        Self { tx }
    }

    pub async fn resolve_peers(
        &self,
        peers: Vec<PeerId>,
    ) -> Result<HashMap<PeerId, Vec<Multiaddr>>, DirectoryError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(NetworkCommand::ResolvePeers { peers, reply })
            .await
            .map_err(|_| DirectoryError::TaskFailed("network actor stopped".into()))?;
        rx.await
            .map_err(|_| DirectoryError::TaskFailed("network actor dropped reply".into()))?
    }

    pub async fn find_nodes(
        &self,
        country: Option<String>,
        min_bandwidth_bps: u128,
        limit: usize,
    ) -> Result<Vec<PoolPeer>, DirectoryError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(NetworkCommand::FindNodes {
                country,
                min_bandwidth_bps,
                limit,
                reply,
            })
            .await
            .map_err(|_| DirectoryError::TaskFailed("network actor stopped".into()))?;
        rx.await
            .map_err(|_| DirectoryError::TaskFailed("network actor dropped reply".into()))?
    }

    pub async fn dial(&self, addresses: HashSet<Multiaddr>) -> Result<Option<PeerId>, DialError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(NetworkCommand::Dial { addresses, reply })
            .await
            .map_err(|_| DialError::Shutdown)?;
        rx.await.map_err(|_| DialError::Shutdown)?
    }

    pub async fn is_connected(&self, peer: PeerId) -> bool {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(NetworkCommand::IsConnected { peer, reply })
            .await
            .is_err()
        {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    pub async fn notify_bandwidth(&self, report: AuthedBandwidthReport) {
        let _ = self
            .tx
            .send(NetworkCommand::NotifyBandwidth { report })
            .await;
    }
}
