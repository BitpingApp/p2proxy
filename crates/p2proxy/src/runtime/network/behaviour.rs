use std::time::Duration;

use libp2p::{
    dcutr, identify, identity::PublicKey, ping, relay, swarm::NetworkBehaviour,
};
use libp2p_stream as stream;

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub stream: stream::Behaviour,
    pub dcutr: dcutr::Behaviour,
    pub relay: relay::client::Behaviour,
    pub identify: identify::Behaviour,
    /// Without a liveness probe, dead peers linger for hours — libp2p only
    /// notices via transport errors that silent network drops never raise. At
    /// 15s/10s a hung-but-reachable peer is booted within ~30s.
    pub ping: ping::Behaviour,
}

impl Behaviour {
    pub fn new(local_pubkey: PublicKey, relay: relay::client::Behaviour) -> Self {
        Self {
            stream: stream::Behaviour::new(),
            dcutr: dcutr::Behaviour::new(local_pubkey.to_peer_id()),
            relay,
            identify: identify::Behaviour::new(identify::Config::new(
                "bitping-federated/1.0.0".into(),
                local_pubkey,
            )),
            ping: ping::Behaviour::new(
                ping::Config::new()
                    .with_interval(Duration::from_secs(15))
                    .with_timeout(Duration::from_secs(10)),
            ),
        }
    }
}
