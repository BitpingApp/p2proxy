//! Hub-side discovery queries: `FindNodes` (attribute-filtered candidate
//! discovery) and legacy pinned-multiaddr route synthesis.

use std::collections::HashSet;
use std::time::Duration;

use bitping_swarm::auth::Auth;
use bitping_swarm::query::QueryRequest;
use color_eyre::eyre::{eyre, Result};
use futures::StreamExt;
use libp2p::{multiaddr::Protocol, Multiaddr};
use models::config::Server;
use models::events::Events;
use p2p_protocol::P2pClient;
use protocols::models::v1::{Bandwidth, Exclusions, Requirements};
use tracing::{info, instrument, warn};

use crate::swarm::KEYPAIR;

use super::DiscoveryEngine;

/// Discover dial addresses for a server: the legacy `destination_peer` pin
/// (verbatim multiaddr, or `<relay>/p2p-circuit/p2p/<id>` synthesis for a
/// bare `/p2p/<id>`), or a hub `FindNodes` query filtered by the server's
/// country / min_bandwidth.
#[instrument(skip(engine))]
pub(crate) async fn discover_peer(
    engine: &mut DiscoveryEngine<'_>,
    server: &Server,
) -> Result<HashSet<Multiaddr>, color_eyre::eyre::Error> {
    let destination_address = if let Some(destination_peer) =
        &server.peer_options.destination_peer
    {
        if let Some(Protocol::P2p(peer_id)) = destination_peer.iter().next() {
            info!("Trying to connect to destination peer");
            // Case 1: It starts with a P2p protocol, append it to the relay address
            let circuit = engine
                .relay_address
                .clone()
                .with(Protocol::P2pCircuit)
                .with_p2p(peer_id)
                .map_err(|addr| eyre!("could not append /p2p/{peer_id} to relay circuit {addr}"))?;
            HashSet::from_iter(vec![circuit])
        } else {
            // Case 2: It's a fully formed multiaddr that doesn't start with P2p, use it directly
            HashSet::from_iter(vec![destination_peer.clone()])
        }
    } else {
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

        // The ask runs on its own task while we keep polling the swarm —
        // the `Control`'s open_stream only progresses while the swarm is
        // driven, and this function holds it exclusively.
        let client = engine.client.clone();
        let relay_peer = engine.relay_peer_id;
        let mut ask = tokio::spawn(async move {
            client
                .ask_with_timeout::<Auth<QueryRequest>>(
                    relay_peer,
                    request,
                    Duration::from_secs(5),
                )
                .await
        });
        let ask_result = loop {
            tokio::select! {
                joined = &mut ask => break joined,
                _ = engine.swarm.next() => {}
            }
        };
        let response = ask_result
            .map_err(|e| eyre!("FindNodes ask task panicked: {e}"))?
            .map_err(|e| eyre!("FindNodes query failed: {e}"))?;
        let peer_ids = match response {
            bitping_swarm::query::QueryResponse::Error(e) => return Err(eyre!(e)),
            bitping_swarm::query::QueryResponse::FindNode(_) => {
                return Err(eyre!(
                    "Got wrong query response, expected FindNodes, got: FindNode"
                ))
            }
            bitping_swarm::query::QueryResponse::FindNodes(hash_set) => hash_set,
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
                    let synth = engine
                        .relay_address
                        .clone()
                        .with(Protocol::P2pCircuit)
                        .with_p2p(p.peer_id)
                        .ok();
                    synth.into_iter().collect::<Vec<_>>()
                } else {
                    p.addresses
                }
            })
            .collect::<HashSet<Multiaddr>>()
    };
    Ok(destination_address)
}
