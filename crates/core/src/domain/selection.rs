use std::collections::{HashMap, HashSet};

use libp2p::{Multiaddr, PeerId, multiaddr::Protocol};

use crate::config::DestinationPeerEntry;

pub fn is_relayed(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| matches!(p, Protocol::P2pCircuit))
}

pub fn last_p2p(addr: &Multiaddr) -> Option<PeerId> {
    addr.iter()
        .filter_map(|p| match p {
            Protocol::P2p(pid) => Some(pid),
            _ => None,
        })
        .last()
}

pub fn destination_peer_ids(addresses: &HashSet<Multiaddr>) -> HashSet<PeerId> {
    addresses.iter().filter_map(last_p2p).collect()
}

pub fn candidate_routes(
    entry: &DestinationPeerEntry,
    resolved: &HashMap<PeerId, Vec<Multiaddr>>,
) -> HashSet<Multiaddr> {
    let mut addrs: HashSet<Multiaddr> = resolved
        .get(&entry.peer_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect();
    addrs.extend(entry.address.clone());
    addrs
}

/// Split routes into (direct, relay-circuit) preserving the preference of
/// dialing direct addresses before circuits.
pub fn partition_direct_first(addrs: impl IntoIterator<Item = Multiaddr>) -> (Vec<Multiaddr>, Vec<Multiaddr>) {
    addrs.into_iter().partition(|a| !is_relayed(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random_peer() -> PeerId {
        libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id()
    }

    #[test]
    fn candidate_routes_unions_resolved_and_verbatim() {
        let peer = random_peer();
        let verbatim: Multiaddr = format!("/ip4/9.9.9.9/tcp/31515/p2p/{peer}")
            .parse()
            .expect("addr");
        let hub_route: Multiaddr =
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

    #[test]
    fn dial_set_uses_last_p2p_component() {
        let relay = random_peer();
        let dest = random_peer();
        let direct_dest = random_peer();
        let circuit: Multiaddr =
            format!("/dns4/hub.example.com/tcp/31515/p2p/{relay}/p2p-circuit/p2p/{dest}")
                .parse()
                .expect("circuit addr");
        let direct: Multiaddr = format!("/ip4/9.9.9.9/tcp/31515/p2p/{direct_dest}")
            .parse()
            .expect("direct addr");

        let ids = destination_peer_ids(&HashSet::from([circuit, direct]));
        assert_eq!(ids, HashSet::from([dest, direct_dest]));
        assert!(!ids.contains(&relay), "relay must never enter the dial set");
    }

    #[test]
    fn bare_direct_address_must_be_tagged_with_peer_id() {
        let peer = random_peer();
        let bare: Multiaddr = "/ip4/203.0.113.7/udp/45445/quic-v1"
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

    #[test]
    fn partition_keeps_direct_before_circuit() {
        let p = random_peer();
        let direct: Multiaddr = format!("/ip4/9.9.9.9/tcp/443/p2p/{p}").parse().expect("addr");
        let circuit: Multiaddr = format!("/dns4/h/tcp/1/p2p-circuit/p2p/{p}")
            .parse()
            .expect("addr");
        let (d, c) = partition_direct_first([circuit.clone(), direct.clone()]);
        assert_eq!(d, vec![direct]);
        assert_eq!(c, vec![circuit]);
    }
}
