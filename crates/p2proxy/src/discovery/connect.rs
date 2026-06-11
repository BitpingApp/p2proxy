//! The discover-and-connect retry loops: ordered pinned-peer preference
//! (BIT-597) and attribute-filtered discovery. Both resolve candidate
//! addresses, dial, and adopt the first peer that completes a connection.

use std::collections::HashSet;
use std::time::Duration;

use color_eyre::eyre::{bail, Result};
use libp2p::{multiaddr::Protocol, swarm::SwarmEvent, PeerId};
use metrics::counter;
use models::config::{DestinationPeerEntry, Server};
use models::events::{DestinationSource, Events, PinnedPeerStatus};
use tracing::{debug, info, warn};

use crate::utils::wait_ext::SwarmWaitExt;

use super::{resolve, ConnectedDestination, DiscoveryEngine};

/// Connect `server` to a destination peer: the ordered `destination_peers`
/// preference list when pinned, otherwise sticky-reuse + attribute-filtered
/// discovery. `avoid` is the peer that just disconnected (the hub usually
/// hasn't noticed yet, so re-dialing it first would waste a 10s wait).
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

/// Best-effort sticky pre-pass: re-resolve the remembered exit peer and try
/// it once before discovery. Failure forgets the entry and falls through —
/// unlike pinning, sticky never blocks on a dead peer.
async fn try_sticky_peer(
    engine: &mut DiscoveryEngine<'_>,
    server: &Server,
    shutdown: &tokio_util::sync::CancellationToken,
    avoid: Option<PeerId>,
) -> Result<Option<PeerId>> {
    if !server.peer_options.sticky {
        return Ok(None);
    }
    let fingerprint = server.peer_options.filter_fingerprint(server.port);
    let Some(remembered) = engine.sticky.get(server.port, &fingerprint) else {
        return Ok(None);
    };
    if Some(remembered) == avoid {
        debug!(
            peer = %remembered,
            port = server.port,
            "sticky peer just disconnected — skipping straight to discovery"
        );
        return Ok(None);
    }

    let remembered_ids = [remembered];
    let resolution = tokio::select! {
        r = resolve::resolve_pinned_routes(engine, &remembered_ids) => r,
        _ = shutdown.cancelled() => bail!("Shutdown requested during sticky-route resolution"),
    };
    let addrs: HashSet<libp2p::Multiaddr> = match resolution {
        Ok(routes) => routes.into_values().flatten().collect(),
        Err(resolve::ResolveError::Unsupported(_)) => {
            resolve::synthesize_circuit(engine.relay_address, remembered)
                .into_iter()
                .collect()
        }
        Err(e @ resolve::ResolveError::TaskPanicked(_)) => return Err(e.into()),
    };

    if !addrs.is_empty()
        && let Some(peer) = dial_and_wait(engine, &addrs, shutdown).await?
    {
        counter!("p2proxy_sticky_hits_total", "port" => server.port.to_string()).increment(1);
        info!(%peer, port = server.port, "reconnected to sticky exit peer");
        return Ok(Some(peer));
    }

    counter!("p2proxy_sticky_misses_total", "port" => server.port.to_string()).increment(1);
    info!(
        peer = %remembered,
        port = server.port,
        "sticky exit peer is gone — discovering a replacement"
    );
    engine.sticky.forget(server.port);
    Ok(None)
}

/// Persist a freshly-discovered exit so the next restart/reconnect re-uses
/// it. The hint line is the graduation path from learned affinity to an
/// explicit pin: copy the id into `destination_peers` and the exit survives
/// even a deleted `sticky_peers.json`.
fn remember_sticky_choice(engine: &mut DiscoveryEngine<'_>, server: &Server, peer: PeerId) {
    if !server.peer_options.sticky {
        return;
    }
    let fingerprint = server.peer_options.filter_fingerprint(server.port);
    match engine.sticky.remember(server.port, &fingerprint, peer) {
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

/// Discover candidates for `server` and connect to one, retrying with the
/// same filters until a peer is adopted. The sticky pre-pass runs first so
/// restarts/reconnects keep the same exit IP whenever the remembered peer is
/// still reachable. In TUI mode an exhausted cycle surfaces an
/// `Events::Error` and starts over after a backoff; headless mode bails so
/// the failure is visible in logs/exit code.
async fn connect_discovered(
    mut engine: DiscoveryEngine<'_>,
    server: &Server,
    shutdown: &tokio_util::sync::CancellationToken,
    avoid: Option<PeerId>,
) -> Result<ConnectedDestination, color_eyre::eyre::Error> {
    if let Some(peer) = try_sticky_peer(&mut engine, server, shutdown, avoid).await? {
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
                r = resolve::discover_peer(&mut engine, server) => r,
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
                    remember_sticky_choice(&mut engine, server, peer_id);
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
    addresses
        .iter()
        .filter_map(|addr| {
            addr.iter()
                .filter_map(|p| match p {
                    Protocol::P2p(pid) => Some(pid),
                    _ => None,
                })
                .last()
        })
        .collect()
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
}
