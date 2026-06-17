use std::future::Future;

use futures::{AsyncRead, AsyncWrite};
use libp2p::PeerId;

use crate::errors::StreamError;

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
