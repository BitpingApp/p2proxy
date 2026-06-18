use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use libp2p::{Multiaddr, PeerId};

use crate::domain::selection::{destination_peer_ids, last_p2p};
use crate::errors::{DialError, DirectoryError};
use crate::events::{Events, PoolPeer};
use crate::ports::{Clock, Dialer, EventSink, PeerDirectory};

/// Scriptable hub directory. `find_nodes` returns queued responses (then empty);
/// `resolve_peers` returns routes for known peers, or `Unsupported` when so set.
#[derive(Default)]
pub struct FakeDirectory {
    find_responses: Mutex<VecDeque<Result<Vec<PoolPeer>, DirectoryError>>>,
    resolved: Mutex<HashMap<PeerId, Vec<Multiaddr>>>,
    resolve_unsupported: bool,
    find_calls: Mutex<usize>,
    resolve_calls: Mutex<usize>,
}

impl FakeDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn unsupported() -> Self {
        Self {
            resolve_unsupported: true,
            ..Self::default()
        }
    }

    pub fn queue_find(mut self, responses: Vec<Result<Vec<PoolPeer>, DirectoryError>>) -> Self {
        self.find_responses = Mutex::new(responses.into());
        self
    }

    pub fn with_resolved(self, map: HashMap<PeerId, Vec<Multiaddr>>) -> Self {
        *self.resolved.lock().expect("lock") = map;
        self
    }

    pub fn find_calls(&self) -> usize {
        *self.find_calls.lock().expect("lock")
    }

    pub fn resolve_calls(&self) -> usize {
        *self.resolve_calls.lock().expect("lock")
    }
}

impl PeerDirectory for FakeDirectory {
    async fn resolve_peers(
        &self,
        peers: &[PeerId],
    ) -> Result<HashMap<PeerId, Vec<Multiaddr>>, DirectoryError> {
        *self.resolve_calls.lock().expect("lock") += 1;
        if self.resolve_unsupported {
            return Err(DirectoryError::Unsupported("fake".into()));
        }
        let map = self.resolved.lock().expect("lock");
        Ok(peers
            .iter()
            .filter_map(|p| map.get(p).map(|a| (*p, a.clone())))
            .collect())
    }

    async fn find_nodes(
        &self,
        _server: &crate::config::Server,
        _limit: usize,
    ) -> Result<Vec<PoolPeer>, DirectoryError> {
        *self.find_calls.lock().expect("lock") += 1;
        let mut queue = self.find_responses.lock().expect("lock");
        queue.pop_front().unwrap_or_else(|| Ok(Vec::new()))
    }
}

/// Dialer that connects any address whose destination peer is in `reachable`,
/// or — when `reachable_addrs` is set — only the exact addresses listed (so a
/// stale address can be modelled as dead while the peer is reachable elsewhere).
#[derive(Default)]
pub struct FakeDialer {
    reachable: Mutex<HashSet<PeerId>>,
    reachable_addrs: Mutex<Option<HashSet<Multiaddr>>>,
    connected: Mutex<HashSet<PeerId>>,
    dialed: Mutex<Vec<HashSet<Multiaddr>>>,
}

impl FakeDialer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reachable(peers: impl IntoIterator<Item = PeerId>) -> Self {
        let d = Self::default();
        *d.reachable.lock().expect("lock") = peers.into_iter().collect();
        d
    }

    /// Only these exact addresses connect — used to model a stale remembered
    /// address that no longer resolves while a fresh route does.
    pub fn reachable_addresses(addrs: impl IntoIterator<Item = Multiaddr>) -> Self {
        let d = Self::default();
        *d.reachable_addrs.lock().expect("lock") = Some(addrs.into_iter().collect());
        d
    }

    pub fn dial_count(&self) -> usize {
        self.dialed.lock().expect("lock").len()
    }
}

impl Dialer for FakeDialer {
    async fn dial_and_wait(
        &self,
        addresses: HashSet<Multiaddr>,
    ) -> Result<Option<PeerId>, DialError> {
        self.dialed.lock().expect("lock").push(addresses.clone());
        if let Some(addrs) = self.reachable_addrs.lock().expect("lock").as_ref() {
            let hit = addresses
                .iter()
                .find(|a| addrs.contains(*a))
                .and_then(last_p2p);
            if let Some(p) = hit {
                self.connected.lock().expect("lock").insert(p);
            }
            return Ok(hit);
        }
        let reachable = self.reachable.lock().expect("lock");
        let hit = destination_peer_ids(&addresses)
            .into_iter()
            .find(|p| reachable.contains(p));
        if let Some(p) = hit {
            self.connected.lock().expect("lock").insert(p);
        }
        Ok(hit)
    }

    async fn is_connected(&self, peer: PeerId) -> bool {
        self.connected.lock().expect("lock").contains(&peer)
    }
}

/// Clock that never actually sleeps — records requested durations so backoff is
/// asserted, never waited on.
#[derive(Default)]
pub struct FakeClock {
    slept: Mutex<Vec<Duration>>,
}

impl FakeClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sleeps(&self) -> Vec<Duration> {
        self.slept.lock().expect("lock").clone()
    }
}

impl Clock for FakeClock {
    async fn sleep(&self, duration: Duration) {
        self.slept.lock().expect("lock").push(duration);
    }
}

/// Captures emitted events for assertions.
#[derive(Default)]
pub struct RecordingSink {
    events: Mutex<Vec<Events>>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<Events> {
        self.events.lock().expect("lock").clone()
    }

    pub fn errors(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                Events::Error(m) => Some(m),
                _ => None,
            })
            .collect()
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: Events) {
        self.events.lock().expect("lock").push(event);
    }
}
