use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use libp2p::{PeerId, Stream};
use libp2p_stream as stream;
use libp2p_stream::OpenStreamError;
use metrics::{counter, histogram};
use p2p_bandwidth_protocol::TCP_PROXY_PROTOCOL;
use proxy_core::ports::StreamError;
use proxy_core::ports::StreamOpener;
use tokio::sync::Semaphore;
use tokio::time::{Instant, timeout};

/// Opens proxy streams to destination peers, capping concurrent opens per peer.
/// Not a connection pool — every call opens a fresh libp2p stream; the cap just
/// prevents a single peer from being hammered. `UnsupportedProtocol` surfaces so
/// the session can evict a peer running an incompatible forwarder.
pub struct PeerStreamManager {
    control: stream::Control,
    max_concurrent_per_peer: usize,
    open_timeout: Duration,
    permits: DashMap<PeerId, Arc<Semaphore>>,
}

impl PeerStreamManager {
    pub fn new(control: stream::Control, max_concurrent_per_peer: usize, open_timeout: Duration) -> Self {
        Self {
            control,
            max_concurrent_per_peer: max_concurrent_per_peer.max(1),
            open_timeout,
            permits: DashMap::new(),
        }
    }
}

impl StreamOpener for PeerStreamManager {
    type Stream = Stream;

    async fn open(&self, peer: PeerId) -> Result<Stream, StreamError> {
        let semaphore = self
            .permits
            .entry(peer)
            .or_insert_with(|| Arc::new(Semaphore::new(self.max_concurrent_per_peer)))
            .clone();

        let _permit = match timeout(self.open_timeout, semaphore.acquire_owned()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) | Err(_) => {
                counter!("p2proxy_stream_acquire_timeout_total").increment(1);
                return Err(StreamError::OpenTimeout { peer });
            }
        };

        let start = Instant::now();
        let mut control = self.control.clone();
        let stream = match timeout(self.open_timeout, control.open_stream(peer, TCP_PROXY_PROTOCOL))
            .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => return Err(map_open_error(peer, e)),
            Err(_) => return Err(StreamError::OpenTimeout { peer }),
        };

        histogram!("p2proxy_stream_acquire_duration_seconds").record(start.elapsed().as_secs_f64());
        counter!("p2proxy_stream_opened_total").increment(1);
        Ok(stream)
    }

    fn stream_closed(&self, _peer: PeerId) {
        counter!("p2proxy_stream_closed_total").increment(1);
    }
}

fn map_open_error(peer: PeerId, error: OpenStreamError) -> StreamError {
    match error {
        OpenStreamError::UnsupportedProtocol(protocol) => StreamError::UnsupportedProtocol {
            peer,
            protocol: protocol.to_string(),
        },
        OpenStreamError::Io(source) => StreamError::Io { peer, source },
        other => StreamError::Io {
            peer,
            source: std::io::Error::other(other.to_string()),
        },
    }
}
