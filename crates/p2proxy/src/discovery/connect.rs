//! The discover-and-connect retry loop: resolve candidate addresses, dial
//! them all, and adopt the first peer that completes a connection.

use std::collections::HashSet;
use std::time::Duration;

use color_eyre::eyre::{bail, Result};
use libp2p::{multiaddr::Protocol, swarm::SwarmEvent, PeerId};
use models::config::Server;
use models::events::Events;
use tracing::{debug, info, warn};

use crate::utils::wait_ext::SwarmWaitExt;

use super::{resolve, DiscoveryEngine};

/// Discover candidates for `server` and connect to one, retrying with the
/// same filters until a peer is adopted. In TUI mode an exhausted cycle
/// surfaces an `Events::Error` and starts over after a backoff; headless
/// mode bails so the failure is visible in logs/exit code.
pub(crate) async fn connect(
    mut engine: DiscoveryEngine<'_>,
    server: &Server,
    shutdown: &tokio_util::sync::CancellationToken,
) -> Result<PeerId, color_eyre::eyre::Error> {
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
                Some(peer_id) => return Ok(peer_id),
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
    use super::destination_peer_ids;
    use std::collections::HashSet;

    fn random_peer() -> libp2p::PeerId {
        libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id()
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
