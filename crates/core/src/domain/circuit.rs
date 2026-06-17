use libp2p::{Multiaddr, PeerId, multiaddr::Protocol};

/// The `<relay>/p2p-circuit/p2p/<id>` route through a bootstrap hub — only
/// reaches peers homed on that hub. Used when the hub can't resolve a current
/// route for a peer.
pub fn synthesize_circuit(relay_address: &Multiaddr, peer_id: PeerId) -> Option<Multiaddr> {
    relay_address
        .clone()
        .with(Protocol::P2pCircuit)
        .with_p2p(peer_id)
        .ok()
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
    fn synthesizes_circuit_through_relay() {
        let relay: Multiaddr = "/dns4/boot.example.com/tcp/45445".parse().expect("addr");
        let peer = random_peer();
        let circuit = synthesize_circuit(&relay, peer).expect("circuit");
        assert!(circuit.iter().any(|p| matches!(p, Protocol::P2pCircuit)));
        assert_eq!(crate::domain::selection::last_p2p(&circuit), Some(peer));
    }
}
