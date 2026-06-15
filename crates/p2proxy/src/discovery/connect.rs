//! The discover-and-connect retry loops: ordered pinned-peer preference
//! (BIT-597) and attribute-filtered discovery. Both resolve candidate
//! addresses, dial, and adopt the first peer that completes a connection.

use std::collections::HashSet;
use std::time::Duration;

use color_eyre::eyre::{Result, bail};
use futures::StreamExt;
use libp2p::{Multiaddr, PeerId, multiaddr::Protocol, swarm::SwarmEvent};
use metrics::counter;
use models::config::{DestinationPeerEntry, Server, StickyReconnect};
use models::events::{DestinationSource, Events, PinnedPeerStatus};
use tracing::{debug, info, warn};

use crate::utils::wait_ext::SwarmWaitExt;

use super::{ConnectedDestination, DiscoveryEngine, resolve};

/// Connect `server` to a destination peer: the ordered `destination_peers`
/// preference list when pinned, otherwise sticky-pool reuse + attribute-
/// filtered discovery. `avoid` is the peer that just disconnected (so the
/// pool pre-pass and discovery skip it — the hub usually hasn't noticed the
/// drop yet, so re-dialing it first would waste a 10s wait).
pub(crate) async fn connect(
    engine: DiscoveryEngine<'_>,
    server: &Server,
    shutdown: &tokio_util::sync::CancellationToken,
    avoid: Option<PeerId>,
) -> Result<ConnectedDestination, color_eyre::eyre::Error> {
    let pinned = server.peer_options.pinned();
    if pinned.is_empty() {
        return connect_discovered(engine, server, shutdown, avoid).await;
    }
    connect_pinned(engine, server, &pinned, shutdown).await
}

/// How many candidates a fresh discovery fan-out asks the hub for — enough
/// to dial several in parallel and adopt the first to connect.
const FINDNODES_DISCOVERY_LIMIT: usize = 25;

/// Does this multiaddr route through a relay circuit (vs a direct address)?
fn is_relayed(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| matches!(p, Protocol::P2pCircuit))
}

/// The destination peer id of a dial address — the LAST `/p2p/` component
/// (a circuit address carries the relay's id earlier in the path).
fn last_p2p(addr: &Multiaddr) -> Option<PeerId> {
    addr.iter()
        .filter_map(|p| match p {
            Protocol::P2p(pid) => Some(pid),
            _ => None,
        })
        .last()
}

/// One reconnect attempt for a known exit `peer`: its remembered direct
/// address first (no hub round-trip when the IP is unchanged), then the
/// hub's CURRENT route for it (which follows a peer that migrated hubs),
/// dialing direct addresses before relay circuits. Returns the adopted peer
/// on success.
async fn try_reach_peer(
    engine: &mut DiscoveryEngine<'_>,
    server: &Server,
    peer: PeerId,
    shutdown: &tokio_util::sync::CancellationToken,
) -> Result<Option<PeerId>> {
    // A stored direct address is a bare transport multiaddr (the observed
    // socket addr from a hole-punched connection); tag it with the peer id
    // so dial_and_wait's accept-predicate can match the resulting
    // ConnectionEstablished. `with_p2p` only errs if a different /p2p/ is
    // already present, which a bare address never has.
    if let Some(direct) = engine.sticky.direct_address(server.port, peer)
        && let Ok(addr) = direct.with_p2p(peer)
    {
        let set = HashSet::from([addr]);
        if let Some(reached) = dial_and_wait(engine, &set, shutdown).await? {
            return Ok(Some(reached));
        }
    }

    let peer_ids = [peer];
    let resolution = tokio::select! {
        r = resolve::resolve_pinned_routes(engine, &peer_ids) => r,
        _ = shutdown.cancelled() => bail!("Shutdown requested during sticky-route resolution"),
    };
    let addrs: Vec<Multiaddr> = match resolution {
        Ok(routes) => routes.into_values().flatten().collect(),
        Err(resolve::ResolveError::Unsupported(_)) => {
            resolve::synthesize_circuit(engine.relay_address, peer)
                .into_iter()
                .collect()
        }
        Err(e @ resolve::ResolveError::TaskPanicked(_)) => return Err(e.into()),
    };

    // Direct routes first, relay circuits as the fallback.
    let (direct, circuit): (Vec<_>, Vec<_>) = addrs.into_iter().partition(|a| !is_relayed(a));
    for group in [direct, circuit] {
        if group.is_empty() {
            continue;
        }
        let set: HashSet<Multiaddr> = group.into_iter().collect();
        if let Some(reached) = dial_and_wait(engine, &set, shutdown).await? {
            return Ok(Some(reached));
        }
    }
    Ok(None)
}

/// Fight to reconnect to the SAME exit `peer` after it dropped: retry
/// [`try_reach_peer`] with exponential backoff so a transient circuit cycle
/// doesn't rotate the egress IP. Returns `None` once the attempt budget is
/// exhausted (the caller then falls back to other pool members / discovery).
async fn reconnect_sticky_peer(
    engine: &mut DiscoveryEngine<'_>,
    server: &Server,
    peer: PeerId,
    shutdown: &tokio_util::sync::CancellationToken,
) -> Result<Option<PeerId>> {
    const MAX_ATTEMPTS: usize = 3;
    const MAX_BACKOFF: Duration = Duration::from_secs(8);
    // Cap total wall-clock: a truly-gone peer must rotate before client-side
    // timeouts fire, and this call holds the shared proxy-event handler.
    const OVERALL_DEADLINE: Duration = Duration::from_secs(30);

    let deadline = tokio::time::Instant::now() + OVERALL_DEADLINE;
    let mut backoff = Duration::from_secs(1);
    for attempt in 1..=MAX_ATTEMPTS {
        if shutdown.is_cancelled() {
            bail!("Shutdown requested while reconnecting to sticky exit peer");
        }
        if let Some(reached) = try_reach_peer(engine, server, peer, shutdown).await? {
            counter!("p2proxy_sticky_reconnect_success_total", "port" => server.port.to_string())
                .increment(1);
            info!(%peer, attempt, port = server.port, "reconnected to sticky exit peer");
            return Ok(Some(reached));
        }
        warn!(
            %peer,
            attempt,
            max = MAX_ATTEMPTS,
            port = server.port,
            "sticky reconnect attempt failed"
        );
        if attempt == MAX_ATTEMPTS || tokio::time::Instant::now() >= deadline {
            break;
        }
        // Back off, but keep draining the swarm — a bare sleep here would
        // freeze the whole network task (this call owns the only &mut Swarm),
        // stalling the transport and every other server's events.
        let wake = (tokio::time::Instant::now() + backoff).min(deadline);
        let sleep = tokio::time::sleep_until(wake);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                _ = shutdown.cancelled() => {
                    bail!("Shutdown requested during sticky reconnect backoff")
                }
                _ = engine.swarm.next() => {}
            }
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
    counter!("p2proxy_sticky_reconnect_exhausted_total", "port" => server.port.to_string())
        .increment(1);
    info!(
        %peer,
        port = server.port,
        "sticky exit peer unreachable after retries — falling back to pool/discovery"
    );
    Ok(None)
}

/// Auto-heal from the remembered pool: try each known-good exit in
/// most-recently-used order (one [`try_reach_peer`] pass each), adopting the
/// first that connects and pruning members that are gone. `skip` is the peer
/// that just dropped — the hub usually hasn't noticed yet. Used on startup,
/// for fail-fast rotation, and after a with-backoff exhaustion.
async fn connect_sticky_pool(
    engine: &mut DiscoveryEngine<'_>,
    server: &Server,
    fingerprint: &str,
    shutdown: &tokio_util::sync::CancellationToken,
    skip: Option<PeerId>,
) -> Result<Option<PeerId>> {
    if !server.peer_options.sticky {
        return Ok(None);
    }
    // Bound total time spent probing the pool so a run of dead members can't
    // hold the shared handler indefinitely before discovery takes over.
    const OVERALL_DEADLINE: Duration = Duration::from_secs(30);
    let deadline = tokio::time::Instant::now() + OVERALL_DEADLINE;
    for peer in engine.sticky.pool(server.port, fingerprint) {
        if Some(peer) == skip {
            continue;
        }
        if shutdown.is_cancelled() {
            bail!("Shutdown requested while reconnecting to sticky pool");
        }
        if tokio::time::Instant::now() >= deadline {
            debug!(
                port = server.port,
                "sticky pool probe deadline reached — falling back to discovery"
            );
            break;
        }
        if let Some(reached) = try_reach_peer(engine, server, peer, shutdown).await? {
            counter!("p2proxy_sticky_pool_hits_total", "port" => server.port.to_string())
                .increment(1);
            info!(%peer, port = server.port, "reconnected to sticky pool member");
            return Ok(Some(reached));
        }
        engine.sticky.forget_peer(server.port, peer);
    }
    Ok(None)
}

/// Add a freshly-adopted exit to the head of the server's sticky pool so the
/// next restart/reconnect re-uses it (and the pool auto-heals/grows). The
/// hint line is the graduation path from learned affinity to an explicit
/// pin: copy the id into `destination_peers` and the exit survives even a
/// deleted `sticky_peers.json`.
fn remember_pool_peer(
    engine: &mut DiscoveryEngine<'_>,
    server: &Server,
    fingerprint: &str,
    peer: PeerId,
) {
    if !server.peer_options.sticky {
        return;
    }
    // One knob: the stream pool's max_total also bounds the sticky pool.
    let max = server.pool.max_total;
    match engine.sticky.remember(server.port, fingerprint, peer, max) {
        Ok(true) => info!(
            %peer,
            port = server.port,
            "sticky exit for this server is now {peer} — add it to destination_peers in Config.yaml to pin it permanently"
        ),
        Ok(false) => {}
        Err(e) => warn!(?e, port = server.port, "could not persist sticky exit peer"),
    }
}

/// Ordered-preference pinned connect: every pass batch-resolves the whole
/// list to current routes, then tries each rank in order — rank 0 is always
/// tried first, so the egress IP only moves down the list when a more
/// preferred peer is genuinely unreachable. Hard pin by default: when every
/// listed peer is offline this keeps retrying (with TUI errors / backoff)
/// rather than silently exiting through an arbitrary discovered node;
/// `fallback_to_discovery: true` opts into availability instead.
async fn connect_pinned(
    mut engine: DiscoveryEngine<'_>,
    server: &Server,
    pinned: &[DestinationPeerEntry],
    shutdown: &tokio_util::sync::CancellationToken,
) -> Result<ConnectedDestination, color_eyre::eyre::Error> {
    // Shared across ranks and passes — bounds how long one connect call can
    // hold the proxy-event handler (each dial-wait is up to 10s), matching
    // the discovery loop's 20-attempt budget.
    const MAX_DIAL_WAITS: usize = 20;

    'outer: loop {
        let mut budget = MAX_DIAL_WAITS;
        while budget > 0 {
            if shutdown.is_cancelled() {
                bail!("Shutdown requested while connecting to pinned peers");
            }

            // One batched query per pass; re-resolving every pass is what
            // follows a peer transparently when it re-homes to another hub.
            let peer_ids: Vec<PeerId> = pinned.iter().map(|e| e.peer_id).collect();
            let resolution = tokio::select! {
                r = resolve::resolve_pinned_routes(&mut engine, &peer_ids) => r,
                _ = shutdown.cancelled() => bail!("Shutdown requested during pinned-route resolution"),
            };
            let resolved = match resolution {
                Ok(routes) => routes,
                Err(resolve::ResolveError::Unsupported(_)) => pinned
                    .iter()
                    .filter_map(|e| {
                        resolve::synthesize_circuit(engine.relay_address, e.peer_id)
                            .map(|addr| (e.peer_id, vec![addr]))
                    })
                    .collect(),
                Err(e @ resolve::ResolveError::TaskPanicked(_)) => return Err(e.into()),
            };

            emit_pinned_statuses(&mut engine, server.port, pinned, &resolved);

            for (rank, entry) in pinned.iter().enumerate() {
                if budget == 0 {
                    break;
                }
                let addrs = candidate_routes(entry, &resolved);
                if addrs.is_empty() {
                    info!(
                        rank,
                        peer = %entry.peer_id,
                        port = server.port,
                        "pinned peer unresolvable — trying next preference"
                    );
                    continue;
                }
                budget -= 1;
                let Some(peer) = dial_and_wait(&mut engine, &addrs, shutdown).await? else {
                    info!(
                        rank,
                        peer = %entry.peer_id,
                        port = server.port,
                        "pinned peer did not connect — trying next preference"
                    );
                    continue;
                };
                // A higher-preference rank's dial may have completed late
                // (after its 10s wait expired). Adopt it instead of leaving
                // the connection idling forever — free fail-back.
                let (rank, peer) = pinned[..rank]
                    .iter()
                    .enumerate()
                    .find(|(_, e)| engine.swarm.is_connected(&e.peer_id))
                    .map(|(better_rank, e)| (better_rank, e.peer_id))
                    .unwrap_or((rank, peer));
                metrics::gauge!("p2proxy_pinned_rank_active", "port" => server.port.to_string())
                    .set(rank as f64);
                info!(rank, %peer, port = server.port, "connected to pinned peer");
                return Ok(ConnectedDestination {
                    peer,
                    source: DestinationSource::Pinned { rank },
                });
            }

            counter!("p2proxy_pinned_pass_failed_total", "port" => server.port.to_string())
                .increment(1);
            if server.peer_options.fallback_to_discovery {
                warn!(
                    port = server.port,
                    "all pinned peers failed — falling back to discovery (fallback_to_discovery: true)"
                );
                counter!("p2proxy_pinned_fallback_total", "port" => server.port.to_string())
                    .increment(1);
                return connect_discovered(engine, server, shutdown, None).await;
            }

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                _ = shutdown.cancelled() => bail!("Shutdown requested between pinned passes"),
            }
        }

        let msg = format!(
            "All {} pinned peer(s) for :{} are offline or unresolvable; retrying. \
             Set fallback_to_discovery: true on this server to allow discovery instead.",
            pinned.len(),
            server.port
        );
        warn!("{msg}");
        let _ = engine.event_send.send(Events::Error(msg.clone())).await;

        if engine.headless {
            bail!(msg);
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            _ = shutdown.cancelled() => bail!("Shutdown requested during pinned retry backoff"),
        }
        continue 'outer;
    }
}

/// A pinned entry's dialable routes this pass: whatever the hub resolved,
/// plus the operator-supplied verbatim multiaddr when present.
fn candidate_routes(
    entry: &DestinationPeerEntry,
    resolved: &std::collections::HashMap<PeerId, Vec<libp2p::Multiaddr>>,
) -> HashSet<libp2p::Multiaddr> {
    let mut addrs: HashSet<libp2p::Multiaddr> = resolved
        .get(&entry.peer_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect();
    addrs.extend(entry.address.clone());
    addrs
}

/// Refresh the per-rank resolvability surface: TUI event, per-rank gauge,
/// and transition-only stale/recovered log lines (state lives in
/// `engine.pinned_resolvable` so retry passes don't spam).
fn emit_pinned_statuses(
    engine: &mut DiscoveryEngine<'_>,
    port: u16,
    pinned: &[DestinationPeerEntry],
    resolved: &std::collections::HashMap<PeerId, Vec<libp2p::Multiaddr>>,
) {
    let statuses: Vec<PinnedPeerStatus> = pinned
        .iter()
        .enumerate()
        .map(|(rank, entry)| PinnedPeerStatus {
            peer_id: entry.peer_id,
            rank,
            resolvable: !candidate_routes(entry, resolved).is_empty(),
        })
        .collect();

    for status in &statuses {
        metrics::gauge!(
            "p2proxy_pinned_peer_resolvable",
            "port" => port.to_string(),
            "rank" => status.rank.to_string()
        )
        .set(if status.resolvable { 1.0 } else { 0.0 });

        let was = engine
            .pinned_resolvable
            .insert((port, status.peer_id), status.resolvable);
        match (was, status.resolvable) {
            (Some(true), false) | (None, false) => warn!(
                peer = %status.peer_id,
                rank = status.rank,
                port,
                "pinned peer went STALE — no route anywhere in the hub mesh"
            ),
            (Some(false), true) => info!(
                peer = %status.peer_id,
                rank = status.rank,
                port,
                "pinned peer recovered — route resolved again"
            ),
            _ => {}
        }
    }

    let event = Events::PinnedPeerStatuses { port, statuses };
    let event_send = engine.event_send.clone();
    tokio::spawn(async move {
        let _ = event_send.send(event).await;
    });
}

/// Connect a discovery-driven `server` to an exit peer. The sticky pool runs
/// first so restarts/reconnects keep a stable exit IP:
///   1. If the active peer just dropped (`avoid`) and the server is in
///      `with-backoff` mode, fight to reconnect to that SAME peer before
///      rotating — a transient circuit cycle shouldn't change the egress IP.
///   2. Otherwise (startup, `fail-fast` rotation, or a with-backoff
///      exhaustion) auto-heal from the remembered pool, trying known-good
///      members in most-recently-used order.
///   3. Only then fall back to fresh attribute-filtered discovery.
///
/// In TUI mode an exhausted discovery cycle surfaces an `Events::Error` and
/// starts over after a backoff; headless mode bails so the failure is visible
/// in logs/exit code.
async fn connect_discovered(
    mut engine: DiscoveryEngine<'_>,
    server: &Server,
    shutdown: &tokio_util::sync::CancellationToken,
    avoid: Option<PeerId>,
) -> Result<ConnectedDestination, color_eyre::eyre::Error> {
    let fingerprint = server.peer_options.filter_fingerprint(server.port);

    // 1. A dropped active sticky peer is worth fighting for in with-backoff
    //    mode — hold the egress IP rather than rotating on a transient drop.
    if let Some(old) = avoid
        && server.peer_options.sticky
        && server.peer_options.sticky_reconnect == StickyReconnect::WithBackoff
    {
        match reconnect_sticky_peer(&mut engine, server, old, shutdown).await? {
            Some(peer) => {
                remember_pool_peer(&mut engine, server, &fingerprint, peer);
                return Ok(ConnectedDestination {
                    peer,
                    source: DestinationSource::Sticky,
                });
            }
            None => engine.sticky.forget_peer(server.port, old),
        }
    }

    // 2. Auto-heal from the remembered pool (skipping the peer that just
    //    dropped — the hub usually hasn't noticed the drop yet).
    if let Some(peer) =
        connect_sticky_pool(&mut engine, server, &fingerprint, shutdown, avoid).await?
    {
        remember_pool_peer(&mut engine, server, &fingerprint, peer);
        return Ok(ConnectedDestination {
            peer,
            source: DestinationSource::Sticky,
        });
    }

    const MAX_RETRIES: usize = 20;
    // Outer loop only re-enters in TUI mode after a full 20-retry
    // exhaustion — emits an Events::Error so the operator sees the
    // failure in the dashboard, sleeps, then starts over. In headless
    // mode we still bail after one cycle since there's no UI to
    // surface the error to.
    'outer: loop {
        let mut retry_count = 0;
        while retry_count < MAX_RETRIES {
            if shutdown.is_cancelled() {
                bail!("Shutdown requested while looking up peers");
            }

            info!(
                "Looking up peer (attempt {}/{})",
                retry_count + 1,
                MAX_RETRIES
            );

            // 1. Discover peers — race the FindNodes query against shutdown
            // so Ctrl+C during the 5s wait_for_with_timeout doesn't have to
            // wait it out.
            let discovery = tokio::select! {
                r = resolve::discover_peer(&mut engine, server, FINDNODES_DISCOVERY_LIMIT) => r,
                _ = shutdown.cancelled() => {
                    bail!("Shutdown requested during peer discovery");
                }
            };
            let destination_addresses = match discovery {
                Ok(addresses) => {
                    if addresses.is_empty() {
                        warn!("No peer addresses discovered");
                        retry_count += 1;
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                            _ = shutdown.cancelled() => {
                                bail!("Shutdown requested between peer-lookup retries");
                            }
                        }
                        continue;
                    }
                    addresses
                }
                Err(e) => {
                    warn!(?e, "Failed to discover peer");
                    retry_count += 1;
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                        _ = shutdown.cancelled() => {
                            bail!("Shutdown requested between peer-lookup retries");
                        }
                    }
                    continue;
                }
            };

            match dial_and_wait(&mut engine, &destination_addresses, shutdown).await? {
                Some(peer_id) => {
                    remember_pool_peer(&mut engine, server, &fingerprint, peer_id);
                    // The rest of the pool fills from the live connections the
                    // swarm promotes as the other dialed candidates hole-punch
                    // (each carrying its real direct address) — see
                    // StickyStore::promote_connected.
                    return Ok(ConnectedDestination {
                        peer: peer_id,
                        source: DestinationSource::Discovered,
                    });
                }
                None => {
                    warn!("Connection timeout reached");
                    retry_count += 1;
                }
            }
        }

        // 20 attempts exhausted. Build a descriptive message, surface it
        // to the TUI, then either bail (headless) or backoff + restart.
        let msg = format!(
            "Failed to connect with any peer after {} attempts (filter: {}). \
             Adjust country/min_bandwidth in Config.yaml, or wait for matching \
             node operators to come online.",
            MAX_RETRIES, server.peer_options
        );
        warn!("{msg}");
        let _ = engine.event_send.send(Events::Error(msg.clone())).await;

        if engine.headless {
            bail!(msg);
        }

        // Long-ish backoff between bursts — FindNodes itself is cheap but
        // the hub-side node pool only meaningfully changes on the scale
        // of seconds-to-minutes.
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            _ = shutdown.cancelled() => bail!("Shutdown requested during retry backoff"),
        }
        continue 'outer;
    }
}

/// Extract the *destination* peer ID from each address. For a circuit-relay
/// multiaddr `/dns4/<hub>/p2p/<HUB>/p2p-circuit/p2p/<DEST>` the address
/// contains TWO P2p protocols — the relay's and the destination's. We want
/// the last one (the actual node behind the circuit). For a direct address
/// (no `/p2p-circuit/`) the last P2p IS the destination.
fn destination_peer_ids(addresses: &HashSet<libp2p::Multiaddr>) -> HashSet<PeerId> {
    addresses.iter().filter_map(last_p2p).collect()
}

/// Dial every candidate address and wait (10s) for the first
/// `ConnectionEstablished` from a peer in the dial set. Returns `Ok(None)`
/// on timeout so the caller can retry; bails on shutdown.
async fn dial_and_wait(
    engine: &mut DiscoveryEngine<'_>,
    destination_addresses: &HashSet<libp2p::Multiaddr>,
    shutdown: &tokio_util::sync::CancellationToken,
) -> Result<Option<PeerId>> {
    // This set is what the wait_for_with_timeout predicate filters on, so we
    // only treat ConnectionEstablished events for these peers as "discovery
    // succeeded" — fixes two bugs:
    //   1. The predicate previously matched the relay's
    //      ConnectionEstablished event (the first to fire on a circuit
    //      dial) and stored the HUB as the destination peer.
    //   2. Late-arriving connections from the *previous* server's dial set
    //      leaked into the next server's wait — making a port configured
    //      for NZ store an AT peer that another server had dialed.
    let our_destinations = destination_peer_ids(destination_addresses);
    info!(
        count = our_destinations.len(),
        "scoping wait_for_with_timeout to candidate destination peers"
    );

    // 2. Dial all peers
    for addr in destination_addresses {
        match engine.swarm.dial(addr.clone()) {
            Ok(_) => info!(?addr, "Dialing peer"),
            Err(e) => warn!(?e, ?addr, "Failed to dial peer"),
        }
    }

    // 3. Wait for any ConnectionEstablished event — also racing
    // against shutdown so the 10s timeout doesn't block exit.
    let wait_result = tokio::select! {
        r = engine.swarm.wait_for_with_timeout(
            move |_, event| {
                if let SwarmEvent::ConnectionEstablished {
                    peer_id,
                    connection_id,
                    endpoint,
                    num_established,
                    concurrent_dial_errors,
                    established_in,
                } = event
                {
                    // Reject events for peers we didn't dial in
                    // this call. Without this filter, the
                    // predicate would happily return the relay
                    // hub's peer_id (the FIRST connection in a
                    // circuit dial), or a leftover destination
                    // from a previous server's discovery that
                    // happens to complete during our wait —
                    // either way, the wrong peer gets stored
                    // as this server's destination and traffic
                    // exits through the wrong country.
                    if !our_destinations.contains(peer_id) {
                        debug!(
                            ?peer_id,
                            candidates = our_destinations.len(),
                            "ignoring ConnectionEstablished for peer not in our dial set"
                        );
                        return None;
                    }
                    info!(
                        ?peer_id,
                        ?connection_id,
                        ?endpoint,
                        ?num_established,
                        ?concurrent_dial_errors,
                        ?established_in,
                        "Connected to candidate destination peer"
                    );
                    return Some(*peer_id);
                }
                None
            },
            Duration::from_secs(10),
        ) => r,
        _ = shutdown.cancelled() => {
            bail!("Shutdown requested while waiting for peer connection");
        }
    };
    Ok(wait_result.ok())
}

#[cfg(test)]
mod tests {
    use super::{candidate_routes, destination_peer_ids};
    use models::config::DestinationPeerEntry;
    use std::collections::{HashMap, HashSet};

    fn random_peer() -> libp2p::PeerId {
        libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id()
    }

    /// A pinned entry dials the union of hub-resolved routes and the
    /// operator's verbatim multiaddr; entries with neither have no routes
    /// (and render as STALE).
    #[test]
    fn candidate_routes_unions_resolved_and_verbatim() {
        let peer = random_peer();
        let verbatim: libp2p::Multiaddr = format!("/ip4/9.9.9.9/tcp/31515/p2p/{peer}")
            .parse()
            .expect("addr");
        let hub_route: libp2p::Multiaddr =
            format!("/dns4/hub.example.com/tcp/31515/p2p-circuit/p2p/{peer}")
                .parse()
                .expect("addr");
        let resolved = HashMap::from([(peer, vec![hub_route.clone()])]);

        let entry = DestinationPeerEntry {
            peer_id: peer,
            address: Some(verbatim.clone()),
        };
        assert_eq!(
            candidate_routes(&entry, &resolved),
            HashSet::from([verbatim.clone(), hub_route])
        );

        let bare = DestinationPeerEntry {
            peer_id: random_peer(),
            address: None,
        };
        assert!(
            candidate_routes(&bare, &resolved).is_empty(),
            "unresolved bare id has no routes"
        );

        let verbatim_only = DestinationPeerEntry {
            peer_id: random_peer(),
            address: Some(verbatim.clone()),
        };
        assert_eq!(
            candidate_routes(&verbatim_only, &resolved),
            HashSet::from([verbatim]),
            "verbatim address keeps an unresolved entry dialable"
        );
    }

    /// Regression for the relay-stored-as-destination bug: the dial-set
    /// extraction must take the LAST /p2p/ component of a circuit address,
    /// never the relay hub's.
    #[test]
    fn dial_set_uses_last_p2p_component() {
        let relay = random_peer();
        let dest = random_peer();
        let direct_dest = random_peer();
        let circuit: libp2p::Multiaddr =
            format!("/dns4/hub.example.com/tcp/31515/p2p/{relay}/p2p-circuit/p2p/{dest}")
                .parse()
                .expect("circuit addr");
        let direct: libp2p::Multiaddr = format!("/ip4/9.9.9.9/tcp/31515/p2p/{direct_dest}")
            .parse()
            .expect("direct addr");

        let ids = destination_peer_ids(&HashSet::from([circuit, direct]));
        assert_eq!(ids, HashSet::from([dest, direct_dest]));
        assert!(!ids.contains(&relay), "relay must never enter the dial set");
    }

    /// The direct-address fast path stores a BARE transport multiaddr (the
    /// observed socket addr, no `/p2p/`). dial_and_wait scopes its accept
    /// predicate to `destination_peer_ids`, so the address must be tagged
    /// with the peer id before dialing or nothing will ever match.
    #[test]
    fn bare_direct_address_must_be_tagged_with_peer_id() {
        let peer = random_peer();
        let bare: libp2p::Multiaddr = "/ip4/203.0.113.7/udp/45445/quic-v1"
            .parse()
            .expect("bare addr");

        assert!(
            destination_peer_ids(&HashSet::from([bare.clone()])).is_empty(),
            "a bare address yields no dial target — would never adopt"
        );

        let tagged = bare.with_p2p(peer).expect("append /p2p/");
        assert_eq!(
            destination_peer_ids(&HashSet::from([tagged])),
            HashSet::from([peer]),
            "tagging with the peer id makes the stored direct address dialable"
        );
    }
}
