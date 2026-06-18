use std::sync::Arc;
use std::time::Duration;

use bitping_swarm::auth::Auth;
use libp2p::PeerId;
use libp2p::identity::Keypair;
use metrics::{counter, gauge};
use p2p_bandwidth_protocol::bandwidth_reporter::{AuthedBandwidthReport, BandwidthReport};
use p2p_bandwidth_protocol::{DataPhaseMessage, ProxySession, TargetAddr};
use proxy_core::events::{BandwidthEvents, Events, SessionEvents};
use proxy_core::ports::{EventSink, StreamOpener};
use socks5_impl::protocol::{
    Address, AsyncStreamOperation, AuthMethod, Reply, Request, Response, handshake,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, BufWriter};
use tokio::net::TcpStream;
use tokio::select;
use tokio::time::timeout;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::adapters::channel_sink::ChannelSink;
use crate::runtime::discovery::{DestinationHandle, DiscoveryHandle};
use crate::runtime::network::NetworkHandle;
use crate::runtime::stream_manager::PeerStreamManager;

const SOCKET_BUF_SIZE: usize = 8196;
const JIT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything one SOCKS session needs. Cloned per accepted connection.
#[derive(Clone)]
pub struct SessionContext {
    pub port: u16,
    pub keypair: Arc<Keypair>,
    pub token: String,
    pub destination: DestinationHandle,
    pub discovery: DiscoveryHandle,
    pub streams: Arc<PeerStreamManager>,
    pub net: NetworkHandle,
    pub events: ChannelSink,
}

/// Resolve the destination peer (current sticky/active, or just-in-time
/// discovery), then run the SOCKS5 session against it.
pub async fn run_session(ctx: SessionContext, socket: TcpStream) {
    let peer = match **ctx.destination.load() {
        Some(peer) => peer,
        None => {
            counter!("p2proxy_socks_jit_discovery_total").increment(1);
            debug!(port = ctx.port, "no active destination; running just-in-time discovery");
            match timeout(JIT_DISCOVERY_TIMEOUT, ctx.discovery.request_new_peer(ctx.port)).await {
                Ok(Some(peer)) => peer,
                _ => {
                    counter!("p2proxy_socks_rejected_no_peer_total").increment(1);
                    warn!("no destination peer available — rejecting SOCKS session");
                    send_general_failure(socket).await;
                    return;
                }
            }
        }
    };
    handle_socks_connection(ctx, socket, peer).await;
}

async fn handle_socks_connection(ctx: SessionContext, mut socket: TcpStream, peer: PeerId) {
    let session_id = Uuid::new_v4();

    let request = match handshake::Request::retrieve_from_async_stream(&mut socket).await {
        Ok(request) => request,
        Err(e) => return session_error("handshake", e),
    };
    if !request.evaluate_method(AuthMethod::NoAuth) {
        let response = handshake::Response::new(AuthMethod::NoAcceptableMethods);
        let _ = response.write_to_async_stream(&mut socket).await;
        return session_error("handshake", "no acceptable auth methods");
    }
    let response = handshake::Response::new(AuthMethod::NoAuth);
    if let Err(e) = response.write_to_async_stream(&mut socket).await {
        return session_error("handshake", e);
    }

    let request = match Request::retrieve_from_async_stream(&mut socket).await {
        Ok(request) => request,
        Err(e) => return session_error("request", e),
    };
    let target: TargetAddr = into_target(request.address.clone());
    let response = Response::new(Reply::Succeeded, request.address);
    if let Err(e) = response.write_to_async_stream(&mut socket).await {
        return session_error("request", e);
    }

    let stream = match ctx.streams.open(peer).await {
        Ok(stream) => stream,
        Err(e) => {
            counter!("p2proxy_stream_acquire_failed_total").increment(1);
            if e.is_terminal_for_peer() {
                warn!(%peer, %e, "peer can't proxy — forgetting it and rediscovering");
                ctx.discovery.peer_unusable(peer).await;
            }
            let response = Response::new(Reply::GeneralFailure, Address::unspecified());
            let _ = response.write_to_async_stream(&mut socket).await;
            return session_error("peer-connection", e);
        }
    };

    let mut session = ProxySession::new_client_session(stream, peer, &ctx.keypair);
    let signed_envelope = match session
        .client_init(session_id.to_string(), target.clone(), ctx.token.clone())
        .await
    {
        Ok(envelope) => envelope,
        Err(e) => return session_error("peer-connection", e),
    };

    debug!(%session_id, %peer, "proxy session established");
    ctx.events.emit(Events::Session(SessionEvents::New(
        session_id, target, peer,
    )));
    counter!("p2proxy_sessions_initialized_total").increment(1);
    gauge!("p2proxy_sessions_active").increment(1.0);

    let (outgoing, incoming) = relay(&ctx, &mut socket, &mut session, session_id).await;
    debug!(
        %session_id, %peer,
        up_bytes = outgoing.bytes,
        down_bytes = incoming.bytes,
        "proxy session closing"
    );

    let _ = session.close().await;
    gauge!("p2proxy_sessions_active").decrement(1.0);
    ctx.streams.stream_closed(peer);
    ctx.events
        .emit(Events::Session(SessionEvents::End(session_id)));

    report_bandwidth(&ctx, session_id, outgoing, incoming, signed_envelope).await;
}

struct Transferred {
    bytes: usize,
    hash: blake3::Hash,
}

async fn relay(
    ctx: &SessionContext,
    socket: &mut TcpStream,
    session: &mut ProxySession<'_>,
    session_id: Uuid,
) -> (Transferred, Transferred) {
    let (mut socket_read, socket_write) = socket.split();
    let mut socket_write = BufWriter::with_capacity(SOCKET_BUF_SIZE, socket_write);
    let mut buf = vec![0u8; SOCKET_BUF_SIZE];
    let mut incoming_hasher = blake3::Hasher::new();
    let mut outgoing_hasher = blake3::Hasher::new();
    let mut incoming_bytes = 0usize;
    let mut outgoing_bytes = 0usize;

    loop {
        select! {
            read = socket_read.read(&mut buf) => match read {
                Ok(0) => {
                    let _ = session.send_close().await;
                    break;
                }
                Ok(n) => {
                    outgoing_hasher.update(&buf[..n]);
                    if session.send_data(buf[..n].to_vec()).await.is_err() {
                        session_error("data-transfer", "failed to write to peer");
                        break;
                    }
                    outgoing_bytes += n;
                    counter!("p2proxy_upload_bytes_total").increment(n as u64);
                    ctx.events.emit(Events::Bandwidth(BandwidthEvents::Upload(session_id, n as u64)));
                }
                Err(e) => { session_error("data-transfer", e); break; }
            },
            message = session.read_data() => match message {
                Ok(DataPhaseMessage::Transfer(transfer)) if transfer.id == session_id.to_string() => {
                    incoming_hasher.update(&transfer.bytes);
                    let n = transfer.bytes.len();
                    if socket_write.write_all(&transfer.bytes).await.is_err()
                        || socket_write.flush().await.is_err()
                    {
                        session_error("data-transfer", "failed to write to client");
                        break;
                    }
                    incoming_bytes += n;
                    counter!("p2proxy_download_bytes_total").increment(n as u64);
                    ctx.events.emit(Events::Bandwidth(BandwidthEvents::Download(session_id, n as u64)));
                }
                Ok(DataPhaseMessage::Transfer(_)) => {}
                Ok(DataPhaseMessage::Close(id)) if id == session_id.to_string() => {
                    let _ = session.send_close().await;
                    break;
                }
                Ok(DataPhaseMessage::Close(_)) => {}
                Ok(DataPhaseMessage::Error(e)) => { session_error("data-transfer", e); break; }
                Err(e) => { session_error("data-transfer", e); break; }
            }
        }
    }

    let _ = socket_write.flush().await;
    let _ = socket_write.shutdown().await;
    (
        Transferred { bytes: outgoing_bytes, hash: outgoing_hasher.finalize() },
        Transferred { bytes: incoming_bytes, hash: incoming_hasher.finalize() },
    )
}

async fn report_bandwidth(
    ctx: &SessionContext,
    session_id: Uuid,
    outgoing: Transferred,
    incoming: Transferred,
    signed_envelope: libp2p::core::SignedEnvelope,
) {
    let report = match BandwidthReport::builder()
        .incoming_hash(*incoming.hash.as_bytes())
        .outgoing_hash(*outgoing.hash.as_bytes())
        .incoming_byte_count(incoming.bytes)
        .outgoing_byte_count(outgoing.bytes)
        .session_uuid(session_id)
        .peer_signed_envelope(signed_envelope)
        .build()
    {
        Ok(report) => report,
        Err(e) => return session_error("shutdown", e),
    };

    let Ok(authed) = Auth::new(report, &ctx.keypair, ctx.token.clone()) else {
        counter!("p2proxy_bandwidth_report_errors_total").increment(1);
        return;
    };
    counter!("p2proxy_bandwidth_reports_sent_total").increment(1);
    ctx.net.notify_bandwidth(AuthedBandwidthReport(authed)).await;
}

fn into_target(address: Address) -> TargetAddr {
    match address {
        Address::SocketAddress(addr) => TargetAddr::Ip(addr),
        Address::DomainAddress(domain, port) => TargetAddr::Domain(domain, port),
    }
}

fn session_error(stage: &str, error: impl std::fmt::Display) {
    counter!("p2proxy_session_errors_total", "stage" => stage.to_string()).increment(1);
    warn!(stage, %error, "socks session error");
}

async fn send_general_failure(mut socket: TcpStream) {
    let response = Response::new(Reply::GeneralFailure, Address::unspecified());
    let _ = response.write_to_async_stream(&mut socket).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_swap::ArcSwap;
    use libp2p::identity::Keypair;
    use proxy_core::testing::builders::peer;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;

    use crate::runtime::discovery::DiscoveryHandle;
    use crate::runtime::network::NetworkHandle;
    use crate::runtime::testutil::dummy_streams;

    fn session_context(destination: PeerId) -> SessionContext {
        SessionContext {
            port: 1080,
            keypair: Arc::new(Keypair::generate_ed25519()),
            token: "token".into(),
            destination: Arc::new(ArcSwap::from_pointee(Some(destination))),
            discovery: DiscoveryHandle::new(mpsc::channel(1).0),
            streams: dummy_streams(),
            net: NetworkHandle::new(mpsc::channel(1).0),
            events: ChannelSink::new(mpsc::channel(8).0),
        }
    }

    /// Drives the real `run_session` over a loopback socket: a SOCKS5 client that
    /// offers no acceptable auth method is rejected before any peer stream is
    /// opened.
    #[tokio::test]
    async fn rejects_socks_client_without_noauth() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        let client = tokio::spawn(async move {
            let mut conn = TcpStream::connect(addr).await.expect("connect");
            // SOCKS5 greeting offering only method 0xFF (no NoAuth).
            conn.write_all(&[0x05, 0x01, 0xFF]).await.expect("write");
            let mut reply = [0u8; 2];
            conn.read_exact(&mut reply).await.expect("read");
            reply
        });

        let (socket, _) = listener.accept().await.expect("accept");
        run_session(session_context(peer()), socket).await;

        assert_eq!(
            client.await.expect("join"),
            [0x05, 0xFF],
            "server replies NoAcceptableMethods"
        );
    }
}
