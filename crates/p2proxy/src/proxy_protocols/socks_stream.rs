// socks_stream.rs
use crate::stream_pool::{PoolConfig, StreamPool};
use color_eyre::eyre::{Result, eyre};
use futures::{AsyncReadExt, AsyncWriteExt, FutureExt, Stream};
use libp2p::{PeerId, Stream as LibP2pStream, core::SignedEnvelope, identity::Keypair};
use libp2p_stream as p2p_stream;
use metrics::{counter, gauge, histogram};
use p2p_bandwidth_protocol::{
    DataPhaseMessage, ProxySession, TCP_PROXY_PROTOCOL, TargetAddr,
    bandwidth_reporter::BandwidthReport,
};
use socks5_impl::protocol::{
    Address, AsyncStreamOperation, AuthMethod, Reply, Request, Response, handshake,
};
use std::time::Duration;
use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _, BufReader, BufWriter},
    net::TcpListener,
    select,
    sync::{
        mpsc::{self, Receiver, Sender},
        oneshot,
    },
};
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use models::config::Server;

// Optimized buffer size
const SOCKET_BUF_SIZE: usize = 8196;

// Stream message types that will be emitted
pub enum SocksStreamMessage {
    Initialized {
        session_id: Uuid,
        target_addr: TargetAddr,
        peer: PeerId,
    },
    DataTransferred {
        session_id: Uuid,
        direction: DataDirection,
        bytes: usize,
    },
    Error {
        session_id: Option<Uuid>,
        error: String,
        stage: SessionStage,
    },
    Finished {
        session_id: Uuid,
        incoming_hash: String,
        outgoing_hash: String,
        report: BandwidthReport,
    },
    RequestNewPeer {
        callback: oneshot::Sender<PeerId>,
        server_config: &'static Server,
    },
    /// Proactive peer-replacement nudge from the swarm side. Emitted when
    /// libp2p reports `ConnectionClosed` for a server's *current*
    /// destination peer. The handler runs `discover_and_connect_to_peer`
    /// and writes the new peer id into the shared `RwLock` so the next
    /// SOCKS session opens against the replacement immediately, rather
    /// than waiting for an actual SOCKS connect to discover the failure
    /// via the stream pool. No callback — the swarm side doesn't need
    /// to await the result; the shared state is what matters.
    PeerDisconnected {
        server_config: &'static Server,
        old_peer: PeerId,
    },
    /// Bootstrap request emitted by `configure_server` to ask the
    /// swarm-owning task to run an initial `discover_and_connect_to_peer`
    /// for a freshly-bound server. Lets the SOCKS listener come up
    /// immediately while discovery (which can retry for minutes if no
    /// peers match the filters) proceeds in the background on the swarm
    /// task — otherwise the first server's hung retry loop blocks every
    /// subsequent server from ever binding its port.
    DiscoverPeerForServer { server_config: &'static Server },
}

#[derive(Debug, Clone)]
pub enum DataDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone)]
pub enum SessionStage {
    Handshake,
    ConnectionRequest,
    PeerConnection,
    DataTransfer,
    Shutdown,
}

struct TargetWrapper(TargetAddr);

impl From<Address> for TargetWrapper {
    fn from(value: Address) -> Self {
        match value {
            Address::SocketAddress(socket_addr) => TargetWrapper(TargetAddr::Ip(socket_addr)),
            Address::DomainAddress(d, p) => TargetWrapper(TargetAddr::Domain(d, p)),
        }
    }
}

pub struct SocksProxyStream {
    receiver: Receiver<SocksStreamMessage>,
}

impl Stream for SocksProxyStream {
    type Item = SocksStreamMessage;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_recv(cx)
    }
}

#[instrument(level = "warn", skip_all, fields(port = server_config.port))]
pub async fn create_socks_proxy_stream(
    server_config: &'static Server,
    local_keypair: &'static Keypair,
    token: String,
    peer: Arc<arc_swap::ArcSwap<Option<PeerId>>>,
    stream_pool: Arc<StreamPool>,
    sender: mpsc::Sender<SocksStreamMessage>,
) -> Result<()> {
    // Fail-fast on the initial bind so configure_server's caller learns
    // about port-in-use right away. Subsequent rebinds happen inside
    // the supervised loop below.
    let initial_listener =
        TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], server_config.port))).await?;
    counter!("p2proxy_socks_server_started_total").increment(1);
    gauge!("p2proxy_socks_servers_active").increment(1.0);
    info!(port = server_config.port, "SOCKS5 proxy listening");

    // Supervisor task. Restarts the accept loop on panic OR on
    // unrecoverable error from inside the loop. The previous accept
    // loop had no panic recovery — if `handle_socks_connection`-style
    // code or any helper panicked while polling, the entire accept
    // task died and the port silently stopped accepting connections
    // (the symptom that motivated this fix). Now a per-port supervisor
    // wraps `run_accept_loop` in `catch_unwind` + rebind retry.
    tokio::spawn(async move {
        let mut listener_opt = Some(initial_listener);
        loop {
            let listener = match listener_opt.take() {
                Some(l) => l,
                None => {
                    // Rebind after a crash. SO_REUSEADDR isn't set by
                    // tokio's `TcpListener::bind` on macOS by default,
                    // so a port in TIME_WAIT will reject — back off
                    // and retry.
                    match TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], server_config.port)))
                        .await
                    {
                        Ok(l) => {
                            counter!(
                                "p2proxy_socks_listener_rebinds_total",
                                "port" => server_config.port.to_string()
                            )
                            .increment(1);
                            info!(
                                port = server_config.port,
                                "SOCKS5 listener re-bound after crash"
                            );
                            l
                        }
                        Err(e) => {
                            error!(
                                port = server_config.port,
                                ?e,
                                "Failed to re-bind, retrying in 5s"
                            );
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            continue;
                        }
                    }
                }
            };

            // catch_unwind catches panics; the outer Result handles
            // graceful errors. Either path triggers a re-bind on
            // the next iteration.
            let inner = run_accept_loop(
                listener,
                server_config,
                local_keypair,
                token.clone(),
                peer.clone(),
                stream_pool.clone(),
                sender.clone(),
            );
            match std::panic::AssertUnwindSafe(inner).catch_unwind().await {
                Ok(Ok(())) => {
                    info!(
                        port = server_config.port,
                        "accept loop returned cleanly — exiting supervisor"
                    );
                    break;
                }
                Ok(Err(e)) => {
                    counter!(
                        "p2proxy_socks_accept_loop_errors_total",
                        "port" => server_config.port.to_string()
                    )
                    .increment(1);
                    error!(
                        port = server_config.port,
                        ?e,
                        "accept loop returned error — re-binding"
                    );
                }
                Err(panic_payload) => {
                    counter!(
                        "p2proxy_socks_accept_loop_panics_total",
                        "port" => server_config.port.to_string()
                    )
                    .increment(1);
                    let msg = panic_message(&panic_payload);
                    error!(port = server_config.port, %msg, "accept loop PANICKED — re-binding");
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        gauge!("p2proxy_socks_servers_active").decrement(1.0);
    });

    Ok(())
}

/// Inner accept loop. Returns `Err` only on unrecoverable conditions
/// where the listener needs to be rebound (every error other than
/// transient per-connection failures). Per-session work is spawned as
/// independent tasks that themselves wrap their work in
/// `catch_unwind`, so a panic in any one SOCKS session never bubbles
/// up to this loop or the supervisor.
async fn run_accept_loop(
    listener: TcpListener,
    server_config: &'static Server,
    local_keypair: &'static Keypair,
    token: String,
    peer: Arc<arc_swap::ArcSwap<Option<PeerId>>>,
    stream_pool: Arc<StreamPool>,
    sender: mpsc::Sender<SocksStreamMessage>,
) -> Result<()> {
    let mut connection_count = 0_u64;
    loop {
        match listener.accept().await {
            Ok((socket, addr)) => {
                connection_count = connection_count.saturating_add(1);
                counter!("p2proxy_socks_connections_total").increment(1);
                gauge!("p2proxy_socks_connections_active").increment(1.0);
                debug!("Accepted connection {} from {}", connection_count, addr);

                if let Err(e) = socket.set_nodelay(true) {
                    warn!("Failed to set TCP_NODELAY: {}", e);
                }

                let connection_pool = stream_pool.clone();
                let connection_sender = sender.clone();
                let connection_token = token.clone();
                let peer_arc = peer.clone();

                // Per-session task with panic guard. A panic inside
                // `handle_session` is contained here — the accept loop
                // and the supervisor never see it.
                tokio::spawn(async move {
                    let fut = handle_session(
                        server_config,
                        socket,
                        local_keypair,
                        connection_token,
                        peer_arc,
                        connection_pool,
                        connection_sender,
                    );
                    if let Err(panic_payload) =
                        std::panic::AssertUnwindSafe(fut).catch_unwind().await
                    {
                        let msg = panic_message(&panic_payload);
                        error!(%msg, "SOCKS session handler panicked");
                        counter!("p2proxy_socks_session_panics_total").increment(1);
                    }
                    gauge!("p2proxy_socks_connections_active").decrement(1.0);
                });
            }
            Err(e) => {
                counter!("p2proxy_socks_accept_errors_total").increment(1);
                // try_send so a saturated `proxy_message_channel`
                // doesn't wedge the accept loop on `.send().await`.
                // The Error variant is observability-only; dropping
                // it is acceptable.
                let _ = sender.try_send(SocksStreamMessage::Error {
                    session_id: None,
                    error: format!("Failed to accept connection: {}", e),
                    stage: SessionStage::Handshake,
                });
                // Force a rebind via the supervisor — accept errors
                // are usually fatal to the listener (e.g. fd table
                // exhaustion). Returning here triggers the catch_
                // unwind/error path in the supervisor.
                return Err(eyre!("accept failed: {}", e));
            }
        }
    }
}

/// Handle a single SOCKS session end-to-end. Resolves the destination
/// peer at session-open time, requesting one JIT via
/// `SocksStreamMessage::RequestNewPeer` if the per-server `ArcSwap`
/// is currently `None` (lazy rediscovery — previously the accept loop
/// would reject the session with `GeneralFailure` and only when the
/// background sweep eventually rediscovered would things recover).
///
/// Channel sends here are non-blocking (`try_send`): if the
/// proxy_message_channel is saturated, we'd rather drop the session
/// than wedge the accept loop on a `send().await`. The earlier
/// behaviour was the root cause of "listener stops accepting after a
/// rediscovery storm" symptom.
async fn handle_session(
    server_config: &'static Server,
    socket: tokio::net::TcpStream,
    local_keypair: &'static Keypair,
    token: String,
    peer_arc: Arc<arc_swap::ArcSwap<Option<PeerId>>>,
    stream_pool: Arc<StreamPool>,
    sender: mpsc::Sender<SocksStreamMessage>,
) {
    let session_peer = match **peer_arc.load() {
        Some(p) => p,
        None => {
            // JIT discovery. The PeerDisconnected handler clears the
            // ArcSwap on libp2p ConnectionClosed without running
            // discovery — discovery happens here on demand instead.
            let (cb_tx, cb_rx) = tokio::sync::oneshot::channel();
            counter!("p2proxy_socks_jit_discovery_total").increment(1);
            if sender
                .try_send(SocksStreamMessage::RequestNewPeer {
                    callback: cb_tx,
                    server_config,
                })
                .is_err()
            {
                warn!("dropping SOCKS session — proxy_message_channel saturated");
                counter!("p2proxy_socks_rejected_channel_full_total").increment(1);
                send_socks5_general_failure(socket).await;
                return;
            }
            // 15s ceiling on JIT discovery. discover_and_connect_to_peer
            // has its own internal retry budget; we're guarding
            // against a wedged swarm task.
            match tokio::time::timeout(Duration::from_secs(15), cb_rx).await {
                Ok(Ok(p)) => p,
                Ok(Err(_)) | Err(_) => {
                    warn!("JIT peer discovery timed out — rejecting SOCKS session");
                    counter!("p2proxy_socks_rejected_no_peer_total").increment(1);
                    send_socks5_general_failure(socket).await;
                    return;
                }
            }
        }
    };

    handle_socks_connection(
        server_config,
        socket,
        local_keypair,
        token,
        session_peer,
        stream_pool,
        sender,
    )
    .await;
}

/// Best-effort SOCKS5 GeneralFailure reply, used when we have to
/// reject the client *before* the normal handshake-and-data flow has
/// taken ownership of the socket. Errors are silently dropped — the
/// client's about to see EOF anyway.
async fn send_socks5_general_failure(mut socket: tokio::net::TcpStream) {
    let response = socks5_impl::protocol::Response::new(
        socks5_impl::protocol::Reply::GeneralFailure,
        socks5_impl::protocol::Address::unspecified(),
    );
    let _ = response.write_to_async_stream(&mut socket).await;
}

/// Best-effort extraction of a panic message from
/// `catch_unwind`'s `Box<dyn Any + Send>` payload. Panics in Rust
/// usually carry a `&'static str` or `String`; we handle both and
/// fall back to a generic marker.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "<non-string panic payload>".to_string()
}

#[instrument(level = "warn", skip_all, fields(peer, server_addr = ?socket.local_addr()))]
async fn handle_socks_connection(
    server_config: &'static Server,
    mut socket: tokio::net::TcpStream,
    local_keypair: &'static Keypair,
    token: String,
    mut peer: PeerId,
    stream_pool: Arc<StreamPool>,
    sender: Sender<SocksStreamMessage>,
) {
    let session_id = Uuid::new_v4();
    let mut incoming_bytes = 0;
    let mut outgoing_bytes = 0;
    let mut session_envelope_bytes: Option<Vec<u8>> = None;

    // SOCKS5 Handshake
    let request = match handshake::Request::retrieve_from_async_stream(&mut socket).await {
        Ok(r) => r,
        Err(e) => {
            counter!("p2proxy_socks_handshake_errors_total").increment(1);
            let _ = sender
                .send(SocksStreamMessage::Error {
                    session_id: None,
                    error: format!("Failed to read initial handshake: {}", e),
                    stage: SessionStage::Handshake,
                })
                .await;
            return;
        }
    };

    if !request.evaluate_method(AuthMethod::NoAuth) {
        counter!("p2proxy_socks_auth_method_errors_total").increment(1);
        let response = handshake::Response::new(AuthMethod::NoAcceptableMethods);
        let _ = response.write_to_async_stream(&mut socket).await;
        let _ = sender
            .send(SocksStreamMessage::Error {
                session_id: None,
                error: "No acceptable authentication methods".to_string(),
                stage: SessionStage::Handshake,
            })
            .await;
        return;
    }

    let response = handshake::Response::new(AuthMethod::NoAuth);
    if let Err(e) = response.write_to_async_stream(&mut socket).await {
        counter!("p2proxy_socks_handshake_response_errors_total").increment(1);
        let _ = sender
            .send(SocksStreamMessage::Error {
                session_id: None,
                error: format!("Failed to write auth response: {}", e),
                stage: SessionStage::Handshake,
            })
            .await;
        return;
    }

    // Connection Request
    let request = match Request::retrieve_from_async_stream(&mut socket).await {
        Ok(r) => r,
        Err(e) => {
            counter!("p2proxy_socks_request_errors_total").increment(1);
            let _ = sender
                .send(SocksStreamMessage::Error {
                    session_id: None,
                    error: format!("Failed to read connection request: {}", e),
                    stage: SessionStage::ConnectionRequest,
                })
                .await;
            return;
        }
    };

    let target_addr: TargetWrapper = request.address.clone().into();
    let response = Response::new(Reply::Succeeded, request.address);
    if let Err(e) = response.write_to_async_stream(&mut socket).await {
        counter!("p2proxy_socks_response_errors_total").increment(1);
        let _ = sender
            .send(SocksStreamMessage::Error {
                session_id: None,
                error: format!("Failed to write connection response: {}", e),
                stage: SessionStage::ConnectionRequest,
            })
            .await;
        return;
    }

    counter!("p2proxy_socks_connections_established_total").increment(1);

    // Acquire a stream from the pool (pool handles rate limiting and timeouts)
    let stream = match stream_pool.acquire_stream(peer).await {
        Ok(s) => s,
        Err(e) => {
            counter!("p2proxy_stream_acquire_failed_total").increment(1);
            // If the failure is terminal for this peer (most commonly:
            // they're running an old `bitping-tcp-forwarder` that doesn't
            // speak our protocol version), nudge the swarm side to evict
            // this peer and rediscover, *before* tearing the SOCKS
            // request down. Without this, every subsequent SOCKS session
            // would keep picking the same broken peer until the swarm
            // happened to drop the underlying TCP connection.
            if e.is_terminal_for_peer() {
                counter!("p2proxy_peer_terminal_error_total").increment(1);
                warn!(
                    %peer,
                    error = %e,
                    "peer is unusable — emitting PeerDisconnected to force rediscovery"
                );
                let _ = sender
                    .send(SocksStreamMessage::PeerDisconnected {
                        server_config,
                        old_peer: peer,
                    })
                    .await;
            } else {
                warn!("Failed to acquire stream from pool: {}", e);
            }
            let response = Response::new(Reply::GeneralFailure, Address::unspecified());
            let _ = response.write_to_async_stream(&mut socket).await;
            let _ = sender
                .send(SocksStreamMessage::Error {
                    session_id: Some(session_id),
                    error: format!("Failed to acquire stream: {}", e),
                    stage: SessionStage::PeerConnection,
                })
                .await;
            return;
        }
    };

    // Create a proxy session for the client side
    let mut proxy_session = ProxySession::new_client_session(stream, peer, local_keypair);

    // Initialize the session with the target address
    let signed_envelope = match proxy_session
        .client_init(session_id.to_string(), target_addr.0.clone(), token)
        .await
    {
        Err(e) => {
            let _ = sender
                .send(SocksStreamMessage::Error {
                    session_id: Some(session_id),
                    error: format!("Failed to initialize session: {}", e),
                    stage: SessionStage::PeerConnection,
                })
                .await;
            return;
        }
        Ok(v) => v,
    };

    // Notify stream initialization
    let _ = sender
        .send(SocksStreamMessage::Initialized {
            session_id,
            target_addr: target_addr.0.clone(),
            peer,
        })
        .await;

    // Optimize data transfer with buffered writers and larger buffers
    let (mut socket_read, socket_write) = socket.split();
    let mut socket_write = BufWriter::with_capacity(SOCKET_BUF_SIZE, socket_write);

    // Pre-allocate a single buffer for reading from socket
    let mut socket_buf = vec![0u8; SOCKET_BUF_SIZE];

    // Create hashers for tracking data integrity
    let mut incoming_hasher = blake3::Hasher::new();
    let mut outgoing_hasher = blake3::Hasher::new();

    // Begin data transfer phase
    loop {
        select! {
            result = socket_read.read(&mut socket_buf) => match result {
                Ok(0) => {
                    debug!("Client closed connection, sending close signal");
                    counter!("p2proxy_socks_client_closed_total").increment(1);
                    let _ = proxy_session.send_close().await;
                    break;
                },
                Ok(n) => {
                    // Send data through the proxy session
                    let bytes_slice = &socket_buf[..n];

                    // Update hash before sending
                    outgoing_hasher.update(bytes_slice);

                    // Send data and report metrics immediately
                    match proxy_session.send_data(bytes_slice.to_vec()).await {
                        Ok(_) => {
                            let bytes_len = bytes_slice.len();
                            outgoing_bytes += bytes_len;
                            histogram!("p2proxy_outgoing_chunk_size_bytes").record(bytes_len as f64);
                            let _ = sender.send(SocksStreamMessage::DataTransferred {
                                session_id,
                                direction: DataDirection::Outgoing,
                                bytes: bytes_len,
                            }).await;
                        },
                        Err(e) => {
                            counter!("p2proxy_data_send_errors_total").increment(1);
                            let _ = sender.send(SocksStreamMessage::Error {
                                session_id: Some(session_id),
                                error: format!("Failed to write to peer: {}", e),
                                stage: SessionStage::DataTransfer,
                            }).await;
                            break;
                        }
                    }
                }
                Err(e) => {
                    counter!("p2proxy_socket_read_errors_total").increment(1);
                    let _ = sender.send(SocksStreamMessage::Error {
                        session_id: Some(session_id),
                        error: format!("Failed to read from client: {}", e),
                        stage: SessionStage::DataTransfer,
                    }).await;
                    break;
                }
            },
            result = proxy_session.read_data() => match result {
                Ok(message) => {
                    match message {
                        DataPhaseMessage::Transfer(transfer) => {
                            if transfer.id == session_id.to_string() {
                                // Update hash before writing
                                incoming_hasher.update(&transfer.bytes);
                                let bytes_len = transfer.bytes.len();
                                histogram!("p2proxy_incoming_chunk_size_bytes").record(bytes_len as f64);

                                // Write data to socket
                                if let Err(e) = socket_write.write_all(&transfer.bytes).await {
                                    counter!("p2proxy_socket_write_errors_total").increment(1);
                                    let _ = sender.send(SocksStreamMessage::Error {
                                        session_id: Some(session_id),
                                        error: format!("Failed to write to client: {}", e),
                                        stage: SessionStage::DataTransfer,
                                    }).await;
                                    break;
                                }

                                // Flush after each write to prevent hanging
                                if let Err(e) = socket_write.flush().await {
                                    counter!("p2proxy_socket_flush_errors_total").increment(1);
                                    let _ = sender.send(SocksStreamMessage::Error {
                                        session_id: Some(session_id),
                                        error: format!("Failed to flush client write: {}", e),
                                        stage: SessionStage::DataTransfer,
                                    }).await;
                                    break;
                                }

                                // Report metrics immediately
                                incoming_bytes += bytes_len;
                                let _ = sender.send(SocksStreamMessage::DataTransferred {
                                    session_id,
                                    direction: DataDirection::Incoming,
                                    bytes: bytes_len,
                                }).await;
                            }
                        }
                        DataPhaseMessage::Error(err) => {
                            counter!("p2proxy_peer_data_errors_total").increment(1);
                            let _ = sender.send(SocksStreamMessage::Error {
                                session_id: Some(session_id),
                                error: format!("Received error message: {}", err),
                                stage: SessionStage::DataTransfer,
                            }).await;
                            break;
                        },
                        DataPhaseMessage::Close(id) => {
                            if id == session_id.to_string() {
                                counter!("p2proxy_peer_closed_total").increment(1);
                                debug!("Received close signal from server");
                                // Acknowledge the close by sending our own close if we haven't already
                                let _ = proxy_session.send_close().await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    counter!("p2proxy_peer_read_errors_total").increment(1);
                    let _ = sender.send(SocksStreamMessage::Error {
                        session_id: Some(session_id),
                        error: format!("Failed to read from peer: {}", e),
                        stage: SessionStage::DataTransfer,
                    }).await;
                    break;
                }
            }
        }
    }

    // Clean up
    // Make sure to flush the buffered writer before closing
    let _ = socket_write.flush().await;
    let _ = proxy_session.close().await;
    let _ = socket_write.shutdown().await;

    let incoming_hash_bytes = incoming_hasher.finalize();
    let outgoing_hash_bytes = outgoing_hasher.finalize();

    let incoming_hash = hex::encode(incoming_hash_bytes.as_bytes());
    let outgoing_hash = hex::encode(outgoing_hash_bytes.as_bytes());

    counter!("p2proxy_sessions_finished_total").increment(1);
    gauge!("p2proxy_bytes_transferred_total").increment((incoming_bytes + outgoing_bytes) as f64);

    let report = match p2p_bandwidth_protocol::bandwidth_reporter::BandwidthReport::builder()
        .incoming_hash(*incoming_hash_bytes.as_bytes())
        .outgoing_hash(*outgoing_hash_bytes.as_bytes())
        .incoming_byte_count(incoming_bytes)
        .outgoing_byte_count(outgoing_bytes)
        .session_uuid(session_id)
        .peer_signed_envelope(signed_envelope)
        .build()
    {
        Ok(v) => v,
        Err(e) => {
            counter!("p2proxy_bandwidth_report_errors_total").increment(1);
            let _ = sender
                .send(SocksStreamMessage::Error {
                    session_id: Some(session_id),
                    error: e.to_string(),
                    stage: SessionStage::Shutdown,
                })
                .await;
            return;
        }
    };

    debug!(
        ?session_id,
        ?report,
        "Session finished with bandwidth report",
    );

    // Notify pool that stream is closed
    stream_pool.stream_closed(peer).await;

    // Send finished message with report
    let _ = sender
        .send(SocksStreamMessage::Finished {
            session_id,
            incoming_hash,
            outgoing_hash,
            report,
        })
        .await;
}
