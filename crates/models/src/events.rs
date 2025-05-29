use bitping_tcp_proxy::TargetAddr;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Events {
    LocalPeerId(PeerId),
    Connection(ConnectionEvents),
    Session(SessionEvents),
    Bandwidth(BandwidthEvents),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionEvents {
    Connecting,
    Connected(PeerId),
    Disconnected,
}

pub type SessionId = uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvents {
    New(SessionId, TargetAddr, PeerId),
    End(SessionId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BandwidthEvents {
    Upload(SessionId, u64),
    Download(SessionId, u64),
}
