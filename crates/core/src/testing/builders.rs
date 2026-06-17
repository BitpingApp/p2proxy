use human_bandwidth::re::bandwidth::Bandwidth;
use libp2p::{Multiaddr, PeerId};

use crate::config::{
    DestinationPeerEntry, PoolConfigOptions, ProxyProtocols, Server, ServerPeerOptions,
    StickyReconnect,
};

pub fn peer() -> PeerId {
    libp2p::identity::Keypair::generate_ed25519()
        .public()
        .to_peer_id()
}

pub fn direct_addr(p: PeerId) -> Multiaddr {
    format!("/ip4/198.51.100.7/tcp/443/p2p/{p}")
        .parse()
        .expect("addr")
}

pub fn relay_addr() -> Multiaddr {
    "/dns4/boot.example.com/tcp/45445".parse().expect("addr")
}

fn options() -> ServerPeerOptions {
    ServerPeerOptions {
        destination_peers: None,
        fallback_to_discovery: false,
        sticky: true,
        sticky_reconnect: StickyReconnect::WithBackoff,
        country: None,
        min_bandwidth: Bandwidth::from_mbps(0),
    }
}

pub fn discovery_server(port: u16) -> Server {
    Server {
        protocol: ProxyProtocols::Socks5,
        port,
        peer_options: options(),
        pool: PoolConfigOptions::default(),
    }
}

pub fn pinned_server(port: u16, peers: Vec<PeerId>) -> Server {
    let mut server = discovery_server(port);
    server.peer_options.destination_peers = Some(
        peers
            .into_iter()
            .map(|peer_id| DestinationPeerEntry {
                peer_id,
                address: None,
            })
            .collect(),
    );
    server
}
