// wireguard.rs
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
use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::UdpSocket,
    select,
    sync::{
        mpsc::{self, Receiver, Sender},
        oneshot,
    },
};
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use models::config::Server;

// Optimized buffer size for UDP packets
const UDP_BUF_SIZE: usize = 4096;

// WireGuard protocol constants
const WIREGUARD_MESSAGE_HANDSHAKE_INITIATION: u8 = 1;
const WIREGUARD_MESSAGE_HANDSHAKE_RESPONSE: u8 = 2;
const WIREGUARD_MESSAGE_COOKIE_REPLY: u8 = 3;
const WIREGUARD_MESSAGE_DATA: u8 = 4;

// Stream message types that will be emitted
pub enum WireguardStreamMessage {
    Initialized {
        session_id: Uuid,
        peer_endpoint: SocketAddr,
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
    KeyExchange,
    PeerConnection,
    DataTransfer,
    Shutdown,
}

pub struct WireguardProxyStream {
    receiver: Receiver<WireguardStreamMessage>,
}

impl Stream for WireguardProxyStream {
    type Item = WireguardStreamMessage;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_recv(cx)
    }
}

/// Creates a WireGuard proxy server that listens for WireGuard connections
/// and tunnels them through libp2p streams to a destination peer.
///
/// # Arguments
///
/// * `server_config` - Static reference to server configuration
/// * `local_keypair` - Static reference to the local libp2p keypair
/// * `token` - Authentication token for the peer
/// * `peer` - The destination peer ID to connect to
/// * `stream_pool` - Arc-wrapped stream pool for managing libp2p streams
/// * `sender` - Channel sender for emitting WireGuard stream messages
///
/// # Returns
///
/// Returns Ok(()) if the server was successfully started, or an error if binding failed.
#[instrument(level = "warn", skip_all, fields(port = server_config.port))]
pub async fn create_wireguard_proxy_stream(
    server_config: &'static Server,
    local_keypair: &'static Keypair,
    token: String,
    peer: PeerId,
    stream_pool: Arc<StreamPool>,
    sender: mpsc::Sender<WireguardStreamMessage>,
) -> Result<()> {
    let socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], server_config.port))).await?;
    counter!("p2proxy_wireguard_server_started_total").increment(1);
    gauge!("p2proxy_wireguard_servers_active").increment(1.0);
    info!("WireGuard proxy listening on UDP port {}", server_config.port);

    tokio::spawn(async move {
        let mut buf = vec![0u8; UDP_BUF_SIZE];
        let mut session_count = 0;

        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, addr)) => {
                    if len == 0 {
                        continue;
                    }

                    session_count += 1;
                    counter!("p2proxy_wireguard_packets_total").increment(1);
                    counter!("p2proxy_wireguard_bytes_received_total").increment(len as u64);

                    debug!("Received WireGuard packet {} from {} ({} bytes)", session_count, addr, len);

                    // Identify WireGuard message type from first byte
                    let message_type = buf[0];
                    match message_type {
                        WIREGUARD_MESSAGE_HANDSHAKE_INITIATION => {
                            debug!("WireGuard handshake initiation from {}", addr);
                            counter!("p2proxy_wireguard_handshake_initiation_total").increment(1);
                        }
                        WIREGUARD_MESSAGE_HANDSHAKE_RESPONSE => {
                            debug!("WireGuard handshake response from {}", addr);
                            counter!("p2proxy_wireguard_handshake_response_total").increment(1);
                        }
                        WIREGUARD_MESSAGE_COOKIE_REPLY => {
                            debug!("WireGuard cookie reply from {}", addr);
                            counter!("p2proxy_wireguard_cookie_reply_total").increment(1);
                        }
                        WIREGUARD_MESSAGE_DATA => {
                            debug!("WireGuard data packet from {}", addr);
                            counter!("p2proxy_wireguard_data_packets_total").increment(1);
                        }
                        _ => {
                            warn!("Unknown WireGuard message type: {}", message_type);
                            counter!("p2proxy_wireguard_unknown_message_total").increment(1);
                        }
                    }

                    let packet_data = buf[..len].to_vec();
                    let connection_pool = stream_pool.clone();
                    let connection_sender = sender.clone();
                    let connection_token = token.clone();
                    let socket_ref = Arc::new(socket.try_clone().expect("Failed to clone socket"));

                    tokio::spawn(async move {
                        handle_wireguard_packet(
                            server_config,
                            socket_ref,
                            addr,
                            packet_data,
                            local_keypair,
                            connection_token,
                            peer,
                            connection_pool,
                            connection_sender,
                        )
                        .await;
                    });
                }
                Err(e) => {
                    counter!("p2proxy_wireguard_receive_errors_total").increment(1);
                    let _ = sender
                        .send(WireguardStreamMessage::Error {
                            session_id: None,
                            error: format!("Failed to receive UDP packet: {}", e),
                            stage: SessionStage::DataTransfer,
                        })
                        .await;
                }
            }
        }
    });

    Ok(())
}

/// Handles a single WireGuard packet by forwarding it through the libp2p stream
/// to the destination peer.
#[instrument(level = "warn", skip_all, fields(peer, client_addr = ?client_addr))]
async fn handle_wireguard_packet(
    server_config: &'static Server,
    socket: Arc<UdpSocket>,
    client_addr: SocketAddr,
    packet_data: Vec<u8>,
    local_keypair: &'static Keypair,
    token: String,
    mut peer: PeerId,
    stream_pool: Arc<StreamPool>,
    sender: Sender<WireguardStreamMessage>,
) {
    let session_id = Uuid::new_v4();
    let packet_len = packet_data.len();

    debug!(
        "Processing WireGuard packet from {} ({} bytes)",
        client_addr, packet_len
    );

    // Acquire a stream from the pool
    let mut p2p_stream = match stream_pool.acquire_stream(peer).await {
        Ok(stream) => stream,
        Err(e) => {
            error!("Failed to acquire stream from pool: {}", e);
            counter!("p2proxy_wireguard_stream_acquire_errors_total").increment(1);
            let _ = sender
                .send(WireguardStreamMessage::Error {
                    session_id: Some(session_id),
                    error: format!("Failed to acquire stream: {}", e),
                    stage: SessionStage::PeerConnection,
                })
                .await;
            return;
        }
    };

    debug!("Acquired stream from pool for session {}", session_id);

    // Forward the WireGuard packet to the peer through the libp2p stream
    if let Err(e) = p2p_stream.write_all(&packet_data).await {
        error!("Failed to write packet to p2p stream: {}", e);
        counter!("p2proxy_wireguard_write_errors_total").increment(1);
        let _ = sender
            .send(WireguardStreamMessage::Error {
                session_id: Some(session_id),
                error: format!("Failed to write to stream: {}", e),
                stage: SessionStage::DataTransfer,
            })
            .await;
        return;
    }

    if let Err(e) = p2p_stream.flush().await {
        error!("Failed to flush p2p stream: {}", e);
        counter!("p2proxy_wireguard_flush_errors_total").increment(1);
        return;
    }

    histogram!("p2proxy_wireguard_packet_size_bytes").record(packet_len as f64);

    let _ = sender
        .send(WireguardStreamMessage::DataTransferred {
            session_id,
            direction: DataDirection::Outgoing,
            bytes: packet_len,
        })
        .await;

    debug!(
        "Successfully forwarded WireGuard packet from {} ({} bytes)",
        client_addr, packet_len
    );

    // Read response from peer (if any)
    let mut response_buf = vec![0u8; UDP_BUF_SIZE];
    match tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        p2p_stream.read(&mut response_buf),
    )
    .await
    {
        Ok(Ok(n)) if n > 0 => {
            debug!("Received response from peer ({} bytes)", n);
            counter!("p2proxy_wireguard_responses_received_total").increment(1);
            counter!("p2proxy_wireguard_bytes_sent_total").increment(n as u64);

            // Send response back to client
            if let Err(e) = socket.send_to(&response_buf[..n], client_addr).await {
                error!("Failed to send response to client: {}", e);
                counter!("p2proxy_wireguard_send_errors_total").increment(1);
            } else {
                let _ = sender
                    .send(WireguardStreamMessage::DataTransferred {
                        session_id,
                        direction: DataDirection::Incoming,
                        bytes: n,
                    })
                    .await;
            }
        }
        Ok(Ok(_)) => {
            debug!("Peer closed connection");
        }
        Ok(Err(e)) => {
            error!("Failed to read from p2p stream: {}", e);
            counter!("p2proxy_wireguard_read_errors_total").increment(1);
        }
        Err(_) => {
            debug!("Timeout waiting for peer response");
            counter!("p2proxy_wireguard_timeouts_total").increment(1);
        }
    }
}
