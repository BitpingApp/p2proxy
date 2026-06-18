use std::collections::{HashMap, HashSet};
use std::time::Duration;

use libp2p::{Multiaddr, PeerId};

use crate::config::{DestinationPeerEntry, Server, StickyReconnect};
use crate::domain::backoff::Backoff;
use crate::domain::circuit::synthesize_circuit;
use crate::domain::selection::candidate_routes;
use thiserror::Error;

use crate::events::{DestinationSource, Events, PinnedPeerStatus};
use crate::ports::{Clock, Dialer, DirectoryError, EventSink, PeerDirectory, StickyStore};

const FINDNODES_DISCOVERY_LIMIT: usize = 25;
const MAX_DISCOVERY_ATTEMPTS: usize = 20;
const MAX_PINNED_PASSES: usize = 20;

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("all {count} pinned peer(s) for :{port} are offline or unresolvable")]
    PinnedExhausted { port: u16, count: usize },
    #[error("no peer matched the filters for :{port} after {attempts} attempts")]
    DiscoveryExhausted { port: u16, attempts: usize },
    #[error("shutdown requested")]
    Shutdown,
    #[error(transparent)]
    Directory(#[from] DirectoryError),
}

#[cfg(test)]
mod connect_error_tests {
    use super::*;

    #[test]
    fn directory_error_converts_into_connect_error() {
        let e: ConnectError = DirectoryError::Timeout.into();
        assert!(matches!(e, ConnectError::Directory(DirectoryError::Timeout)));
    }
}
const STICKY_RECONNECT_ATTEMPTS: usize = 3;

pub struct ConnectedDestination {
    pub peer: PeerId,
    pub source: DestinationSource,
}

/// Borrowed handles the connect flow needs. The swarm is pumped by the network
/// actor, so connect never touches it directly — it only asks the directory to
/// resolve/discover, the dialer to dial, and the clock to back off.
pub struct ConnectCtx<'a, Dir, Dl, Clk, Stk, Ev> {
    pub directory: &'a Dir,
    pub dialer: &'a Dl,
    pub clock: &'a Clk,
    pub sticky: &'a mut Stk,
    pub events: &'a Ev,
    pub relay_address: &'a Multiaddr,
}

impl<Dir, Dl, Clk, Stk, Ev> ConnectCtx<'_, Dir, Dl, Clk, Stk, Ev>
where
    Dir: PeerDirectory,
    Dl: Dialer,
    Clk: Clock,
    Stk: StickyStore,
    Ev: EventSink,
{
    /// Connect `server` to a destination peer. `avoid` is the peer that just
    /// dropped (skipped on the pool pre-pass and discovery). One bounded effort
    /// — the caller reschedules on `Err`.
    pub async fn connect(
        &mut self,
        server: &Server,
        avoid: Option<PeerId>,
    ) -> Result<ConnectedDestination, ConnectError> {
        let pinned = server.peer_options.pinned();
        if pinned.is_empty() {
            return self.connect_discovered(server, avoid).await;
        }
        self.connect_pinned(server, &pinned).await
    }

    async fn connect_pinned(
        &mut self,
        server: &Server,
        pinned: &[DestinationPeerEntry],
    ) -> Result<ConnectedDestination, ConnectError> {
        let peer_ids: Vec<PeerId> = pinned.iter().map(|e| e.peer_id).collect();

        for _ in 0..MAX_PINNED_PASSES {
            let resolved = self.resolve_routes(&peer_ids).await;
            self.emit_pinned_statuses(server.port, pinned, &resolved);

            for (rank, entry) in pinned.iter().enumerate() {
                let routes = candidate_routes(entry, &resolved);
                if routes.is_empty() {
                    continue;
                }
                let Some(peer) = self.dial(routes).await else {
                    continue;
                };
                let (rank, peer) = self.adopt_best_pinned_rank(pinned, rank, peer).await;
                return Ok(ConnectedDestination {
                    peer,
                    source: DestinationSource::Pinned { rank },
                });
            }

            if server.peer_options.fallback_to_discovery {
                return self.connect_discovered(server, None).await;
            }
            self.clock.sleep(Duration::from_secs(1)).await;
        }

        let msg = format!(
            "All {} pinned peer(s) for :{} are offline or unresolvable.",
            pinned.len(),
            server.port
        );
        self.events.emit(Events::Error(msg));
        Err(ConnectError::PinnedExhausted {
            port: server.port,
            count: pinned.len(),
        })
    }

    /// A higher-preference rank may have connected late; prefer it.
    async fn adopt_best_pinned_rank(
        &self,
        pinned: &[DestinationPeerEntry],
        rank: usize,
        peer: PeerId,
    ) -> (usize, PeerId) {
        for (better, entry) in pinned[..rank].iter().enumerate() {
            if self.dialer.is_connected(entry.peer_id).await {
                return (better, entry.peer_id);
            }
        }
        (rank, peer)
    }

    async fn connect_discovered(
        &mut self,
        server: &Server,
        avoid: Option<PeerId>,
    ) -> Result<ConnectedDestination, ConnectError> {
        let fingerprint = server.peer_options.filter_fingerprint(server.port);

        if let Some(old) = avoid
            && server.peer_options.sticky
            && server.peer_options.sticky_reconnect == StickyReconnect::WithBackoff
        {
            match self.reconnect_sticky_peer(server, old).await {
                Some(peer) => {
                    self.remember(server, &fingerprint, peer);
                    return Ok(sticky(peer));
                }
                None => self.sticky.forget_peer(server.port, old),
            }
        }

        if let Some(peer) = self
            .connect_sticky_pool(server, &fingerprint, avoid)
            .await
        {
            self.remember(server, &fingerprint, peer);
            return Ok(sticky(peer));
        }

        self.run_discovery(server, &fingerprint).await
    }

    async fn run_discovery(
        &mut self,
        server: &Server,
        fingerprint: &str,
    ) -> Result<ConnectedDestination, ConnectError> {
        for _ in 0..MAX_DISCOVERY_ATTEMPTS {
            let candidates = match self
                .directory
                .find_nodes(server, FINDNODES_DISCOVERY_LIMIT)
                .await
            {
                Ok(candidates) => candidates,
                Err(_) => {
                    self.clock.sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            self.events.emit(Events::ServerPool {
                port: server.port,
                peers: candidates.clone(),
            });

            if candidates.is_empty() {
                self.clock.sleep(Duration::from_secs(1)).await;
                continue;
            }

            let addresses: HashSet<Multiaddr> = candidates
                .into_iter()
                .flat_map(|c| {
                    if c.addresses.is_empty() {
                        synthesize_circuit(self.relay_address, c.peer_id)
                            .into_iter()
                            .collect()
                    } else {
                        c.addresses
                    }
                })
                .collect();

            if let Some(peer) = self.dial(addresses).await {
                self.remember(server, fingerprint, peer);
                return Ok(ConnectedDestination {
                    peer,
                    source: DestinationSource::Discovered,
                });
            }
        }

        let msg = format!(
            "Failed to connect with any peer for :{} after {} attempts (filter: {}).",
            server.port, MAX_DISCOVERY_ATTEMPTS, server.peer_options
        );
        self.events.emit(Events::Error(msg));
        Err(ConnectError::DiscoveryExhausted {
            port: server.port,
            attempts: MAX_DISCOVERY_ATTEMPTS,
        })
    }

    /// Fight to reconnect to the same dropped exit peer with exponential backoff
    /// so a transient circuit cycle doesn't rotate the egress IP.
    async fn reconnect_sticky_peer(&mut self, server: &Server, peer: PeerId) -> Option<PeerId> {
        let mut backoff = Backoff::new(Duration::from_secs(1), 2, Duration::from_secs(8));
        for attempt in 0..STICKY_RECONNECT_ATTEMPTS {
            if let Some(reached) = self.try_reach_peer(server, peer).await {
                return Some(reached);
            }
            if attempt + 1 < STICKY_RECONNECT_ATTEMPTS {
                self.clock.sleep(backoff.next_delay()).await;
            }
        }
        None
    }

    /// Try each remembered pool member in most-recently-used order, pruning ones
    /// that are gone. `skip` is the peer that just dropped.
    async fn connect_sticky_pool(
        &mut self,
        server: &Server,
        fingerprint: &str,
        skip: Option<PeerId>,
    ) -> Option<PeerId> {
        if !server.peer_options.sticky {
            return None;
        }
        for peer in self.sticky.pool(server.port, fingerprint) {
            if Some(peer) == skip {
                continue;
            }
            if let Some(reached) = self.try_reach_peer(server, peer).await {
                return Some(reached);
            }
            self.sticky.forget_peer(server.port, peer);
        }
        None
    }

    /// One reconnect attempt for a known peer. Its remembered direct address,
    /// the hub's current routes, and (when the hub can't answer) a synthesized
    /// relay circuit are dialed together in a single race. A stale remembered
    /// address — which a DCUtR reconnect produces every time it rotates the
    /// peer's port — then loses the race instead of blocking a fresh route
    /// behind the whole dial timeout.
    async fn try_reach_peer(&mut self, server: &Server, peer: PeerId) -> Option<PeerId> {
        let mut routes: HashSet<Multiaddr> = HashSet::new();
        if let Some(direct) = self.sticky.direct_address(server.port, peer)
            && let Ok(tagged) = direct.with_p2p(peer)
        {
            routes.insert(tagged);
        }
        let resolved = self.resolve_routes(&[peer]).await;
        routes.extend(resolved.into_values().flatten());
        if routes.is_empty()
            && let Some(circuit) = synthesize_circuit(self.relay_address, peer)
        {
            routes.insert(circuit);
        }
        self.dial(routes).await
    }

    /// Resolve peer ids to current routes, falling back to circuit synthesis
    /// when the hub can't answer. A transient failure yields an empty map and is
    /// retried on the next pass.
    async fn resolve_routes(&self, peers: &[PeerId]) -> HashMap<PeerId, Vec<Multiaddr>> {
        match self.directory.resolve_peers(peers).await {
            Ok(routes) => routes,
            Err(DirectoryError::Unsupported(_)) => peers
                .iter()
                .filter_map(|p| synthesize_circuit(self.relay_address, *p).map(|a| (*p, vec![a])))
                .collect(),
            Err(_) => HashMap::new(),
        }
    }

    async fn dial(&self, addresses: HashSet<Multiaddr>) -> Option<PeerId> {
        if addresses.is_empty() {
            return None;
        }
        self.dialer.dial_and_wait(addresses).await.ok().flatten()
    }

    fn remember(&mut self, server: &Server, fingerprint: &str, peer: PeerId) {
        if server.peer_options.sticky {
            self.sticky
                .remember(server.port, fingerprint, peer, server.pool.max_total);
        }
    }

    fn emit_pinned_statuses(
        &self,
        port: u16,
        pinned: &[DestinationPeerEntry],
        resolved: &HashMap<PeerId, Vec<Multiaddr>>,
    ) {
        let statuses = pinned
            .iter()
            .enumerate()
            .map(|(rank, entry)| PinnedPeerStatus {
                peer_id: entry.peer_id,
                rank,
                resolvable: !candidate_routes(entry, resolved).is_empty(),
            })
            .collect();
        self.events.emit(Events::PinnedPeerStatuses { port, statuses });
    }
}

fn sticky(peer: PeerId) -> ConnectedDestination {
    ConnectedDestination {
        peer,
        source: DestinationSource::Sticky,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::sticky::StickyState;
    use crate::events::PoolPeer;
    use crate::testing::builders::{direct_addr, discovery_server, peer, pinned_server, relay_addr};
    use crate::testing::fakes::{FakeClock, FakeDialer, FakeDirectory, RecordingSink};
    use futures::executor::block_on;
    use std::collections::HashMap;
    use std::time::Duration;

    #[allow(clippy::too_many_arguments)]
    fn run(
        server: &Server,
        avoid: Option<PeerId>,
        dir: &FakeDirectory,
        dl: &FakeDialer,
        clk: &FakeClock,
        sink: &RecordingSink,
        sticky: &mut StickyState,
        relay: &Multiaddr,
    ) -> Result<ConnectedDestination, ConnectError> {
        let mut ctx = ConnectCtx {
            directory: dir,
            dialer: dl,
            clock: clk,
            sticky,
            events: sink,
            relay_address: relay,
        };
        block_on(ctx.connect(server, avoid))
    }

    fn candidate(p: PeerId) -> PoolPeer {
        PoolPeer {
            peer_id: p,
            addresses: vec![direct_addr(p)],
        }
    }

    #[test]
    fn discovered_adopts_and_remembers() {
        let p = peer();
        let server = discovery_server(1080);
        let fp = server.peer_options.filter_fingerprint(1080);
        let dir = FakeDirectory::new().queue_find(vec![Ok(vec![candidate(p)])]);
        let dl = FakeDialer::reachable([p]);
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());
        let mut sticky = StickyState::default();

        let dest = run(&server, None, &dir, &dl, &clk, &sink, &mut sticky, &relay).expect("connects");
        assert_eq!(dest.peer, p);
        assert_eq!(dest.source, DestinationSource::Discovered);
        assert_eq!(sticky.pool(1080, &fp), vec![p], "adopted peer is remembered");
        assert!(
            sink.events()
                .iter()
                .any(|e| matches!(e, Events::ServerPool { .. })),
            "pool surfaced to the TUI"
        );
    }

    #[test]
    fn discovery_retries_until_candidates_appear() {
        let p = peer();
        let server = discovery_server(1080);
        let dir = FakeDirectory::new().queue_find(vec![Ok(vec![]), Ok(vec![candidate(p)])]);
        let dl = FakeDialer::reachable([p]);
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());
        let mut sticky = StickyState::default();

        let dest = run(&server, None, &dir, &dl, &clk, &sink, &mut sticky, &relay).expect("connects");
        assert_eq!(dest.peer, p);
        assert!(dir.find_calls() >= 2);
        assert!(clk.sleeps().contains(&Duration::from_secs(1)));
    }

    #[test]
    fn discovery_exhausts_and_reports_error() {
        let server = discovery_server(1080);
        let dir = FakeDirectory::new();
        let dl = FakeDialer::new();
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());
        let mut sticky = StickyState::default();

        let res = run(&server, None, &dir, &dl, &clk, &sink, &mut sticky, &relay);
        assert!(matches!(
            res,
            Err(ConnectError::DiscoveryExhausted { port: 1080, .. })
        ));
        assert_eq!(dir.find_calls(), MAX_DISCOVERY_ATTEMPTS);
        assert!(!sink.errors().is_empty());
    }

    #[test]
    fn sticky_pool_falls_back_to_relay_circuit_on_transient_hub_failure() {
        // A remembered peer with no stored direct address, and the hub can't
        // resolve a route this pass (transient). Before the fix this dropped the
        // peer and fell through to discovery; now it synthesizes a relay circuit
        // and reconnects to the same exit.
        let p = peer();
        let server = discovery_server(1080);
        let fp = server.peer_options.filter_fingerprint(1080);
        let mut sticky = StickyState::default();
        sticky.remember(1080, &fp, p, 5);
        let dir = FakeDirectory::new();
        let dl = FakeDialer::reachable([p]);
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());

        let dest = run(&server, None, &dir, &dl, &clk, &sink, &mut sticky, &relay)
            .expect("reconnects via synthesized relay circuit");
        assert_eq!(dest.peer, p);
        assert_eq!(dest.source, DestinationSource::Sticky);
        assert_eq!(dir.find_calls(), 0, "never fell through to discovery");
    }

    #[test]
    fn sticky_pool_reconnects_using_remembered_direct_address() {
        let p = peer();
        let server = discovery_server(1080);
        let fp = server.peer_options.filter_fingerprint(1080);
        let mut sticky = StickyState::default();
        sticky.remember(1080, &fp, p, 5);
        sticky.note_direct_address(p, direct_addr(p));
        let dir = FakeDirectory::new();
        let dl = FakeDialer::reachable([p]);
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());

        let dest = run(&server, None, &dir, &dl, &clk, &sink, &mut sticky, &relay).expect("connects");
        assert_eq!(dest.peer, p);
        assert_eq!(dest.source, DestinationSource::Sticky);
        assert_eq!(dir.find_calls(), 0, "remembered peer needs no discovery");
        assert_eq!(dl.dial_count(), 1, "remembered + hub routes raced in one dial");
    }

    #[test]
    fn stale_remembered_direct_does_not_block_a_fresh_route() {
        // The remembered direct address is dead (a DCUtR reconnect rotated the
        // peer's port), but the hub has a fresh route. The reconnect must land
        // on the fresh route in a single dial — never serially behind the stale
        // address's full dial timeout (the ~10s reconnect stall).
        let p = peer();
        let server = discovery_server(1080);
        let fp = server.peer_options.filter_fingerprint(1080);
        let stale: Multiaddr = format!("/ip4/203.0.113.7/tcp/9/p2p/{p}").parse().expect("addr");
        let fresh = direct_addr(p);
        let mut sticky = StickyState::default();
        sticky.remember(1080, &fp, p, 5);
        sticky.note_direct_address(p, stale.clone());
        let dir = FakeDirectory::new().with_resolved(HashMap::from([(p, vec![fresh.clone()])]));
        let dl = FakeDialer::reachable_addresses([fresh]);
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());

        let dest = run(&server, None, &dir, &dl, &clk, &sink, &mut sticky, &relay)
            .expect("reconnects on the fresh route despite the stale remembered address");
        assert_eq!(dest.peer, p);
        assert_eq!(dest.source, DestinationSource::Sticky);
        assert_eq!(dl.dial_count(), 1, "stale + fresh raced in one dial, not serially");
    }

    #[test]
    fn unsupported_resolve_falls_back_to_circuit() {
        let p = peer();
        let server = discovery_server(1080);
        let fp = server.peer_options.filter_fingerprint(1080);
        let mut sticky = StickyState::default();
        sticky.remember(1080, &fp, p, 5);
        let dir = FakeDirectory::unsupported();
        let dl = FakeDialer::reachable([p]);
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());

        let dest = run(&server, None, &dir, &dl, &clk, &sink, &mut sticky, &relay)
            .expect("connects via synthesized circuit");
        assert_eq!(dest.peer, p);
        assert_eq!(dest.source, DestinationSource::Sticky);
        assert_eq!(dir.find_calls(), 0);
    }

    #[test]
    fn sticky_reconnect_backs_off_then_forgets_and_discovers() {
        let old = peer();
        let fresh = peer();
        let server = discovery_server(1080);
        let fp = server.peer_options.filter_fingerprint(1080);
        let mut sticky = StickyState::default();
        sticky.remember(1080, &fp, old, 5);
        let dir = FakeDirectory::new().queue_find(vec![Ok(vec![candidate(fresh)])]);
        let dl = FakeDialer::reachable([fresh]);
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());

        let dest = run(&server, Some(old), &dir, &dl, &clk, &sink, &mut sticky, &relay)
            .expect("rotates to a fresh peer");
        assert_eq!(dest.peer, fresh);
        assert_eq!(dest.source, DestinationSource::Discovered);
        assert_eq!(sticky.pool(1080, &fp), vec![fresh], "old forgotten, fresh remembered");
        assert_eq!(
            clk.sleeps()[..2],
            [Duration::from_secs(1), Duration::from_secs(2)],
            "exponential backoff between reconnect attempts"
        );
    }

    #[test]
    fn pinned_rank0_wins_when_reachable() {
        let (a, b) = (peer(), peer());
        let server = pinned_server(1080, vec![a, b]);
        let dir = FakeDirectory::new().with_resolved(HashMap::from([(a, vec![direct_addr(a)])]));
        let dl = FakeDialer::reachable([a]);
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());
        let mut sticky = StickyState::default();

        let dest = run(&server, None, &dir, &dl, &clk, &sink, &mut sticky, &relay).expect("connects");
        assert_eq!(dest.peer, a);
        assert_eq!(dest.source, DestinationSource::Pinned { rank: 0 });
        assert!(
            sink.events()
                .iter()
                .any(|e| matches!(e, Events::PinnedPeerStatuses { .. }))
        );
    }

    #[test]
    fn pinned_fails_over_to_rank1() {
        let (a, b) = (peer(), peer());
        let server = pinned_server(1080, vec![a, b]);
        let dir = FakeDirectory::new()
            .with_resolved(HashMap::from([(a, vec![direct_addr(a)]), (b, vec![direct_addr(b)])]));
        let dl = FakeDialer::reachable([b]);
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());
        let mut sticky = StickyState::default();

        let dest = run(&server, None, &dir, &dl, &clk, &sink, &mut sticky, &relay).expect("connects");
        assert_eq!(dest.peer, b);
        assert_eq!(dest.source, DestinationSource::Pinned { rank: 1 });
    }

    #[test]
    fn hard_pin_exhausts_without_silently_discovering() {
        let a = peer();
        let server = pinned_server(1080, vec![a]);
        let dir = FakeDirectory::new();
        let dl = FakeDialer::new();
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());
        let mut sticky = StickyState::default();

        let res = run(&server, None, &dir, &dl, &clk, &sink, &mut sticky, &relay);
        assert!(matches!(
            res,
            Err(ConnectError::PinnedExhausted { port: 1080, count: 1 })
        ));
        assert_eq!(clk.sleeps().len(), MAX_PINNED_PASSES);
        assert!(!sink.errors().is_empty());
    }

    #[test]
    fn pinned_falls_back_to_discovery_when_opted_in() {
        let a = peer();
        let fresh = peer();
        let mut server = pinned_server(1080, vec![a]);
        server.peer_options.fallback_to_discovery = true;
        let dir = FakeDirectory::new().queue_find(vec![Ok(vec![candidate(fresh)])]);
        let dl = FakeDialer::reachable([fresh]);
        let (clk, sink, relay) = (FakeClock::new(), RecordingSink::new(), relay_addr());
        let mut sticky = StickyState::default();

        let dest = run(&server, None, &dir, &dl, &clk, &sink, &mut sticky, &relay).expect("connects");
        assert_eq!(dest.peer, fresh);
        assert_eq!(dest.source, DestinationSource::Discovered);
    }
}
