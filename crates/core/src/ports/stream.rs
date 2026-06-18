use std::future::Future;

use futures::{AsyncRead, AsyncWrite};
use libp2p::PeerId;
use thiserror::Error;

/// Opens a proxy stream to a destination peer. Production opens a libp2p stream
/// (with per-peer concurrency limiting + failover); the fake hands back an
/// in-memory duplex pipe so the session relay is testable without a network.
pub trait StreamOpener {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send;

    fn open(
        &self,
        peer: PeerId,
    ) -> impl Future<Output = Result<Self::Stream, StreamError>> + Send;

    fn stream_closed(&self, peer: PeerId);
}

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("remote peer {peer} does not support {protocol}")]
    UnsupportedProtocol { peer: PeerId, protocol: String },
    #[error("io error opening stream to {peer}: {source}")]
    Io {
        peer: PeerId,
        #[source]
        source: std::io::Error,
    },
    #[error("timeout opening stream to {peer}")]
    OpenTimeout { peer: PeerId },
}

impl StreamError {
    /// Whether the peer is fundamentally unable to serve us, so the caller
    /// should evict and rediscover rather than retry the same peer.
    pub fn is_terminal_for_peer(&self) -> bool {
        matches!(self, StreamError::UnsupportedProtocol { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> PeerId {
        libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id()
    }

    #[test]
    fn unsupported_protocol_is_terminal() {
        let e = StreamError::UnsupportedProtocol {
            peer: peer(),
            protocol: "/bitping/tcp/1".into(),
        };
        assert!(e.is_terminal_for_peer());
    }

    #[test]
    fn open_timeout_is_not_terminal() {
        assert!(!StreamError::OpenTimeout { peer: peer() }.is_terminal_for_peer());
    }
}
