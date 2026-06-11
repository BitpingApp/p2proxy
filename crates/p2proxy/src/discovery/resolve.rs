//! Hub-side discovery queries: `FindNodes` (attribute-filtered candidate
//! discovery), `ResolvePeers` (explicit peer-id → current-route resolution,
//! BIT-597), and legacy circuit-route synthesis.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bitping_swarm::auth::Auth;
use bitping_swarm::query::{QueryRequest, QueryResponse, MAX_RESOLVE_PEERS};
use color_eyre::eyre::{eyre, Result};
use futures::StreamExt;
use libp2p::{multiaddr::Protocol, Multiaddr, PeerId};
use metrics::counter;
use models::config::Server;
use models::events::Events;
use p2p_protocol::P2pClient;
use protocols::models::v1::{Bandwidth, Exclusions, Requirements};
use thiserror::Error;
use tracing::{info, instrument, warn};

use crate::swarm::KEYPAIR;

use super::DiscoveryEngine;

#[derive(Debug, Error)]
pub(crate) enum ResolveError {
    /// The hub dropped the stream or answered with an error — the shape an
    /// old (pre-BIT-597) hub produces on the unknown query variant. The
    /// caller falls back to circuit synthesis for this pass; the query is
    /// retried on the next pass, so a transient network failure here never
    /// permanently downgrades resolution.
    #[error("hub could not answer ResolvePeers: {0}")]
    Unsupported(String),
    #[error("ResolvePeers ask task panicked: {0}")]
    TaskPanicked(String),
}

/// Spawn `ask` on its own task while keeping the swarm polled — the
/// `Control`'s open_stream only progresses while the swarm is driven, and
/// the discovery path holds it exclusively.
async fn ask_hub(
    engine: &mut DiscoveryEngine<'_>,
    request: Auth<QueryRequest>,
) -> Result<Result<QueryResponse, p2p_protocol::P2pError>, tokio::task::JoinError> {
    let client = engine.client.clone();
    let relay_peer = engine.relay_peer_id;
    let mut ask = tokio::spawn(async move {
        client
            .ask_with_timeout::<Auth<QueryRequest>>(relay_peer, request, Duration::from_secs(5))
            .await
    });
    loop {
        tokio::select! {
            joined = &mut ask => break joined,
            _ = engine.swarm.next() => {}
        }
    }
}

/// The legacy `<relay>/p2p-circuit/p2p/<id>` route through our own bootstrap
/// hub — only reaches peers homed on that hub. Used when the hub can't
/// resolve a current route (old hub, or no `public_address` configured).
pub(crate) fn synthesize_circuit(relay_address: &Multiaddr, peer_id: PeerId) -> Option<Multiaddr> {
    relay_address
        .clone()
        .with(Protocol::P2pCircuit)
        .with_p2p(peer_id)
        .ok()
}

/// Resolve pinned peer ids to their CURRENT routes via the hub's
/// `ResolvePeers` query (BIT-597). Ids absent from the result are not
/// connected anywhere in the reachable hub mesh. Runs on every (re)connect
/// pass so hub re-homing is followed transparently.
pub(crate) async fn resolve_pinned_routes(
    engine: &mut DiscoveryEngine<'_>,
    peer_ids: &[PeerId],
) -> Result<HashMap<PeerId, Vec<Multiaddr>>, ResolveError> {
    let capped: Vec<PeerId> = peer_ids.iter().copied().take(MAX_RESOLVE_PEERS).collect();
    if capped.len() < peer_ids.len() {
        warn!(
            requested = peer_ids.len(),
            cap = MAX_RESOLVE_PEERS,
            "destination_peers exceeds the hub cap — resolving only the first entries"
        );
    }

    counter!("p2proxy_resolve_peers_total").increment(1);
    let request = Auth::new(
        QueryRequest::ResolvePeers(capped),
        &KEYPAIR,
        engine.token.to_string(),
    )
    .map_err(|e| ResolveError::Unsupported(format!("failed to sign request: {e}")))?;

    let response = match ask_hub(engine, request).await {
        Err(join_err) => return Err(ResolveError::TaskPanicked(join_err.to_string())),
        Ok(Err(ask_err)) => {
            note_resolve_unsupported(engine, &ask_err.to_string());
            return Err(ResolveError::Unsupported(ask_err.to_string()));
        }
        Ok(Ok(response)) => response,
    };

    let discovered = match response {
        QueryResponse::FindNodes(set) => set,
        QueryResponse::Error(e) => {
            note_resolve_unsupported(engine, &e);
            return Err(ResolveError::Unsupported(e));
        }
        QueryResponse::FindNode(_) => {
            return Err(ResolveError::Unsupported(
                "expected FindNodes response, got FindNode".to_string(),
            ))
        }
    };

    if engine.resolve_supported.is_none() {
        info!("hub supports ResolvePeers — pinned routes resolve dynamically");
    }
    *engine.resolve_supported = Some(true);
    Ok(discovered
        .into_iter()
        .map(|p| (p.peer_id, p.addresses))
        .collect())
}

/// Warn loudly the first time resolution fails (likely a pre-BIT-597 hub);
/// later failures only count a metric — the caller retries every pass.
fn note_resolve_unsupported(engine: &mut DiscoveryEngine<'_>, reason: &str) {
    counter!("p2proxy_resolve_peers_unsupported_total").increment(1);
    if engine.resolve_supported.is_none() {
        warn!(
            reason,
            "hub did not answer ResolvePeers — falling back to relay-circuit synthesis. \
             Routes to peers homed on other hubs will be unreachable until the hub is upgraded."
        );
        *engine.resolve_supported = Some(false);
    }
}

/// Discover dial addresses for a server via a hub `FindNodes` query
/// filtered by the server's country / min_bandwidth. Pinned servers never
/// reach this — `connect` routes them through `resolve_pinned_routes`.
#[instrument(skip(engine))]
pub(crate) async fn discover_peer(
    engine: &mut DiscoveryEngine<'_>,
    server: &Server,
) -> Result<HashSet<Multiaddr>, color_eyre::eyre::Error> {
    let destination_address = {
        let mut node_reqs = Requirements::default();
        let node_excs = Exclusions {
            bandwidth: Some(Bandwidth {
                less_than: Some(server.peer_options.min_bandwidth.as_bps() as f64),
                greater_than: None,
            }),
            ..Default::default()
        };

        if let Some(c) = &server.peer_options.country {
            node_reqs.countries = vec![c.clone()];
        }

        // Snapshot what we're sending to the hub. After we saw
        // 3/3 country-tagged servers exit through Austria despite
        // RU/AU/NZ filters, this is the canary that tells us
        // whether the country code is actually reaching the wire.
        // Visible in TUI under LOGS with RUST_LOG=info default.
        info!(
            port = server.port,
            countries = ?node_reqs.countries,
            min_bandwidth_bps = server.peer_options.min_bandwidth.as_bps(),
            "FindNodes request — country filter going on the wire"
        );

        let request = Auth::new(
            QueryRequest::FindNodes {
                requirements: Some(node_reqs),
                exclusions: Some(node_excs),
                capabilities: None,
                limit: 25,
            },
            &KEYPAIR,
            engine.token.to_string(),
        )?;

        let response = ask_hub(engine, request)
            .await
            .map_err(|e| eyre!("FindNodes ask task panicked: {e}"))?
            .map_err(|e| eyre!("FindNodes query failed: {e}"))?;
        let peer_ids = match response {
            QueryResponse::Error(e) => return Err(eyre!(e)),
            QueryResponse::FindNode(_) => {
                return Err(eyre!(
                    "Got wrong query response, expected FindNodes, got: FindNode"
                ))
            }
            QueryResponse::FindNodes(hash_set) => hash_set,
        };

        // Hub answered the FindNodes query; the *count* tells you whether
        // any operators currently match your country / min_bandwidth
        // filters. An empty set isn't a transport failure — it's the hub
        // saying "no candidates right now" — so we log at info with the
        // count up front rather than dumping the (often empty) set as
        // "success".
        let discovered = peer_ids;
        if discovered.is_empty() {
            warn!(
                ?server.peer_options,
                "FindNodes returned 0 peers — no operators currently match these filters"
            );
        } else {
            info!(
                peer_count = discovered.len(),
                peers = ?discovered.iter().map(|p| p.peer_id).collect::<Vec<_>>(),
                "FindNodes returned candidate peers"
            );
        }

        // Surface the pool to the TUI's NETWORK tab so the operator
        // can see who's available for each server, not just the one
        // we picked. Replace-not-merge — peers that fell out of the
        // filter shouldn't linger in the rendered list.
        let pool_peers: Vec<libp2p::PeerId> =
            discovered.iter().map(|p| p.peer_id).collect();
        metrics::gauge!("p2proxy_server_pool_size", "port" => server.port.to_string())
            .set(pool_peers.len() as f64);
        let _ = engine
            .event_send
            .send(Events::ServerPool {
                port: server.port,
                peers: pool_peers,
            })
            .await;

        // Prefer the addresses the hub gave us — they point
        // at whichever hub the peer is actually homed on, not just
        // ours. If the hub returned no addresses for a peer (older
        // hub without `public_address` configured), fall back to the
        // legacy synthesis through our own relay; this still works
        // for local-hub peers.
        discovered
            .into_iter()
            .flat_map(|p| {
                if p.addresses.is_empty() {
                    // Legacy fallback — only reaches peers on the
                    // hub we're connected to.
                    synthesize_circuit(engine.relay_address, p.peer_id)
                        .into_iter()
                        .collect::<Vec<_>>()
                } else {
                    p.addresses
                }
            })
            .collect::<HashSet<Multiaddr>>()
    };
    Ok(destination_address)
}
