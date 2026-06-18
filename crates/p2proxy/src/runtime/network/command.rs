use std::collections::{HashMap, HashSet};

use libp2p::{Multiaddr, PeerId};
use p2p_bandwidth_protocol::bandwidth_reporter::AuthedBandwidthReport;
use proxy_core::ports::{DialError, DirectoryError};
use proxy_core::events::PoolPeer;
use tokio::sync::oneshot;

/// Requests serviced by the network actor — the only owner of the swarm. Each
/// reply rides an embedded oneshot so callers await a typed response.
pub enum NetworkCommand {
    ResolvePeers {
        peers: Vec<PeerId>,
        reply: oneshot::Sender<Result<HashMap<PeerId, Vec<Multiaddr>>, DirectoryError>>,
    },
    FindNodes {
        country: Option<String>,
        min_bandwidth_bps: u128,
        limit: usize,
        reply: oneshot::Sender<Result<Vec<PoolPeer>, DirectoryError>>,
    },
    Dial {
        addresses: HashSet<Multiaddr>,
        reply: oneshot::Sender<Result<Option<PeerId>, DialError>>,
    },
    IsConnected {
        peer: PeerId,
        reply: oneshot::Sender<bool>,
    },
    NotifyBandwidth {
        report: AuthedBandwidthReport,
    },
}
