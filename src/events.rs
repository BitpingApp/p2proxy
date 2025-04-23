use bitping_tcp_proxy::TargetAddr;
use libp2p::PeerId;

#[derive(Debug)]
pub enum Events {
    LocalPeerId(PeerId),
    Connection(ConnectionEvents),
    Session(SessionEvents),
    Bandwidth(BandwidthEvents),
}

#[derive(Debug)]
pub enum ConnectionEvents {
    Connecting,
    Connected(PeerId),
    Disconnected,
}

pub type SessionId = uuid::Uuid;

#[derive(Debug)]
pub enum SessionEvents {
    New(SessionId, TargetAddr, PeerId),
    End(SessionId),
}

#[derive(Debug)]
pub enum BandwidthEvents {
    Upload(SessionId, u64),
    Download(SessionId, u64),
}
