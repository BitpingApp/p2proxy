//! Shared test scaffolding for the runtime: an in-memory libp2p swarm (so the
//! network actor / bootstrap exercise real libp2p over `MemoryTransport`) and a
//! `Context` whose unused-by-the-actor-under-test deps are cheap dummies.

use std::sync::Arc;
use std::time::Duration;

use libp2p::core::transport::MemoryTransport;
use libp2p::core::upgrade;
use libp2p::identity::Keypair;
use libp2p::{Swarm, Transport, noise, yamux};
use p2p_protocol::client::LibP2pClient;
use proxy_core::config::Config;
use proxy_core::testing::builders::{discovery_server, peer, relay_addr};

use crate::adapters::channel_sink::ChannelSink;
use crate::adapters::tokio_clock::TokioClock;
use crate::runtime::context::Context;
use crate::runtime::discovery::DiscoveryHandle;
use crate::runtime::network::NetworkHandle;
use crate::runtime::network::behaviour::Behaviour;
use crate::runtime::stream_manager::PeerStreamManager;

pub fn test_config() -> Arc<Config> {
    Arc::new(Config {
        servers: vec![discovery_server(1080)],
        listen_addrs: vec!["0.0.0.0:0".parse().expect("addr")],
        bitping_api_key: "test-key".into(),
        bootstrap_address: "/dnsaddr/boot2.bitping.com".parse().expect("addr"),
        grpc_url: "https://grpc.bitping.com".into(),
        keypair_path: "node_keypair.bin".into(),
        metrics_port: 9091,
    })
}

/// A `LibP2pClient` over a free-standing stream control — fine for actors that
/// don't drive hub asks in the test.
pub fn dummy_client() -> LibP2pClient {
    LibP2pClient::new(libp2p_stream::Behaviour::new().new_control(), peer())
}

pub fn dummy_streams() -> Arc<PeerStreamManager> {
    Arc::new(PeerStreamManager::new(
        libp2p_stream::Behaviour::new().new_control(),
        5,
        Duration::from_secs(5),
    ))
}

pub fn dummy_context(
    network: NetworkHandle,
    discovery: DiscoveryHandle,
    events: ChannelSink,
) -> Context {
    Context {
        config: test_config(),
        keypair: Arc::new(Keypair::generate_ed25519()),
        token: "token".into(),
        relay_peer_id: peer(),
        relay_address: relay_addr(),
        bootstrap_peer_id: peer(),
        bootstrap_address: "/memory/999999999".parse().expect("addr"),
        client: dummy_client(),
        events,
        network,
        discovery,
        streams: dummy_streams(),
        clock: TokioClock,
    }
}

/// A real `Swarm<Behaviour>` over an in-memory transport — two of these can be
/// connected without touching the network.
pub fn memory_swarm() -> Swarm<Behaviour> {
    libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_other_transport(|key| {
            MemoryTransport::default()
                .upgrade(upgrade::Version::V1)
                .authenticate(noise::Config::new(key).expect("noise"))
                .multiplex(yamux::Config::default())
        })
        .expect("transport")
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .expect("relay client")
        .with_behaviour(|k, relay| Behaviour::new(k.public(), relay))
        .expect("behaviour")
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
        .build()
}
