//! Peer discovery and destination connection for proxy servers: hub
//! `FindNodes` queries, pinned-peer handling, and the dial-all /
//! first-to-connect loop ([`connect`]), plus the hub query plumbing
//! ([`resolve`]).

mod connect;
mod resolve;

pub(crate) use connect::connect;

use std::collections::HashMap;

use libp2p::{Multiaddr, PeerId, Swarm};
use models::events::{DestinationSource, Events};
use p2p_protocol::client::LibP2pClient;
use tokio::sync::mpsc::Sender;

use crate::swarm::Behaviour;

/// Borrowed handles the discovery/connect path needs from the bootstrapped
/// swarm state. Constructed per call by `ProxyNetwork::<Bootstrapped>` (the
/// only place with access to the underlying fields) and consumed by
/// [`connect`].
pub(crate) struct DiscoveryEngine<'a> {
    pub swarm: &'a mut Swarm<Behaviour>,
    /// Typed outbound handle over the swarm's `libp2p_stream::Control`.
    /// Asks only progress while `swarm` is polled — the helpers here spawn
    /// the ask and keep driving the swarm in a select loop.
    pub client: &'a LibP2pClient,
    pub relay_address: &'a Multiaddr,
    pub relay_peer_id: PeerId,
    pub token: &'a str,
    pub event_send: &'a Sender<Events>,
    /// When `true`, discovery failures bail instead of looping with TUI
    /// error events.
    pub headless: bool,
    /// Whether the hub answered a `ResolvePeers` query yet (`None` =
    /// untried). Only gates warn-once logging — resolution is retried
    /// every pass regardless, so one transient failure never pins the
    /// process to legacy circuit synthesis.
    pub resolve_supported: &'a mut Option<bool>,
    /// Last known per-(port, peer) resolvability, so stale-peer log lines
    /// fire on transitions instead of every retry pass.
    pub pinned_resolvable: &'a mut HashMap<(u16, PeerId), bool>,
}

/// The outcome of [`connect`]: the adopted destination and how it was chosen.
pub(crate) struct ConnectedDestination {
    pub peer: PeerId,
    pub source: DestinationSource,
}
