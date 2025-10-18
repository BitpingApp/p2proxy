// socks_stream.rs
use bitping_tcp_proxy::{
    bandwidth_reporter::BandwidthReport, DataPhaseMessage, ProxySession, TargetAddr,
    TCP_PROXY_PROTOCOL,
};
use color_eyre::eyre::Result;
use futures::{AsyncReadExt, AsyncWriteExt, Stream};
use libp2p::{core::SignedEnvelope, identity::Keypair, PeerId, Stream as LibP2pStream};
use libp2p_stream as p2p_stream;
use crate::stream_pool::{PoolConfig, StreamPool};
use metrics::{counter, gauge, histogram};
use socks5_impl::protocol::{
    handshake, Address, AsyncStreamOperation, AuthMethod, Reply, Request, Response,
};
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
    peer: PeerId,
    stream_pool: Arc<StreamPool>,
    sender: mpsc::Sender<SocksStreamMessage>,
) -> Result<()> {
    let listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], server_config.port))).await?;
    counter!("p2proxy_socks_server_started_total").increment(1);
    gauge!("p2proxy_socks_servers_active").increment(1.0);
    info!("SOCKS5 proxy listening");

    tokio::spawn(async move {
        let mut connection_count = 0;

        loop {
            match listener.accept().await {
                Ok((socket, addr)) => {
                    connection_count += 1;
                    counter!("p2proxy_socks_connections_total").increment(1);
                    gauge!("p2proxy_socks_connections_active").increment(1.0);
                    debug!("Accepted connection {} from {}", connection_count, addr);

                    // Set TCP_NODELAY to reduce latency
                    if let Err(e) = socket.set_nodelay(true) {
                        warn!("Failed to set TCP_NODELAY: {}", e);
                    }

                    let connection_pool = stream_pool.clone();
                    let connection_sender = sender.clone();
                    let connection_token = token.clone();

                    tokio::spawn(async move {
                        handle_socks_connection(
                            server_config,
                            socket,
                            local_keypair,
                            connection_token,
                            peer,
                            connection_pool,
                            connection_sender,
                        )
                        .await;
                        // Decrement active connections when handler finishes
                        gauge!("p2proxy_socks_connections_active").decrement(1.0);
                    });
                }
                Err(e) => {
                    counter!("p2proxy_socks_accept_errors_total").increment(1);
                    let _ = sender
                        .send(SocksStreamMessage::Error {
                            session_id: None,
                            error: format!("Failed to accept connection: {}", e),
                            stage: SessionStage::Handshake,
                        })
                        .await;
                }
            }
        }
    });

    Ok(())
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
            warn!("Failed to acquire stream from pool: {}", e);
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

    let report = match bitping_tcp_proxy::bandwidth_reporter::BandwidthReport::builder()
        .incoming_hash(*incoming_hash_bytes.as_bytes())
        .outgoing_hash(*outgoing_hash_bytes.as_bytes())
        .incoming_byte_count(incoming_bytes)
        .outgoing_byte_count(outgoing_bytes)
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
