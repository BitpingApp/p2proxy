use libp2p::PeerId;
use thiserror::Error;

/// Hub-query failures (`FindNodes` / `ResolvePeers`). `Unsupported` is the
/// pre-BIT-597 hub shape — the caller falls back to circuit synthesis and
/// retries next pass; the others are genuine query failures.
#[derive(Debug, Error)]
pub enum DirectoryError {
    #[error("hub query timed out")]
    Timeout,
    #[error("hub rejected the query: {0}")]
    Rejected(String),
    #[error("hub does not support this query: {0}")]
    Unsupported(String),
    #[error("hub query task failed: {0}")]
    TaskFailed(String),
}

#[derive(Debug, Error)]
pub enum DialError {
    #[error("no candidate connected before the deadline")]
    NoneConnected,
    #[error("shutdown requested during dial")]
    Shutdown,
}

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("all {count} pinned peer(s) for :{port} are offline or unresolvable")]
    PinnedExhausted { port: u16, count: usize },
    #[error("no peer matched the filters for :{port} after {attempts} attempts")]
    DiscoveryExhausted { port: u16, attempts: usize },
    #[error("shutdown requested")]
    Shutdown,
    #[error(transparent)]
    Directory(#[from] DirectoryError),
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("auth service returned an empty federated token")]
    EmptyToken,
    #[error("federated token is malformed: {0}")]
    MalformedToken(String),
    #[error("auth transport error: {0}")]
    Transport(String),
    #[error("failed to sign auth request: {0}")]
    Signing(String),
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

#[derive(Debug, Error)]
pub enum StickyStoreError {
    #[error("failed to serialize sticky store: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to persist sticky store to {path}: {source}")]
    Persist {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
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

    #[test]
    fn directory_error_converts_into_connect_error() {
        let e: ConnectError = DirectoryError::Timeout.into();
        assert!(matches!(e, ConnectError::Directory(DirectoryError::Timeout)));
    }
}
