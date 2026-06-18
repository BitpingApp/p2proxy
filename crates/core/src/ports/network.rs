use std::collections::HashMap;
use std::collections::HashSet;
use std::future::Future;

use libp2p::{Multiaddr, PeerId};

use thiserror::Error;

use crate::config::Server;
use crate::events::PoolPeer;

/// Hub-backed peer discovery. Adapter drives the libp2p swarm + hub query
/// protocol; the fake returns canned candidate sets.
pub trait PeerDirectory {
    fn resolve_peers(
        &self,
        peers: &[PeerId],
    ) -> impl Future<Output = Result<HashMap<PeerId, Vec<Multiaddr>>, DirectoryError>> + Send;

    fn find_nodes(
        &self,
        server: &Server,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<PoolPeer>, DirectoryError>> + Send;
}

/// Dialing + connection state. Adapter dials through the swarm and waits for the
/// first `ConnectionEstablished` from a candidate; the fake scripts outcomes.
pub trait Dialer {
    fn dial_and_wait(
        &self,
        addresses: HashSet<Multiaddr>,
    ) -> impl Future<Output = Result<Option<PeerId>, DialError>> + Send;

    fn is_connected(&self, peer: PeerId) -> impl Future<Output = bool> + Send;
}

/// Hub-query failures (`FindNodes` / `ResolvePeers`). `Unsupported` is the
/// pre-BIT-597 hub shape — the caller falls back to circuit synthesis and
/// retries next pass; the others are genuine query failures.
#[derive(Debug, Error)]
pub enum DirectoryError {
    #[error("hub query timed out")]
    Timeout,
    #[error("hub rejected the query: {0}")]
    Rejected(String),
    #[error("hub does not support this query: {0}")]
    Unsupported(String),
    #[error("hub query task failed: {0}")]
    TaskFailed(String),
}

#[derive(Debug, Error)]
pub enum DialError {
    #[error("no candidate connected before the deadline")]
    NoneConnected,
    #[error("shutdown requested during dial")]
    Shutdown,
}
