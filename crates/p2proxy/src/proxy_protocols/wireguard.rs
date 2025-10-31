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
    collections::HashMap,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::UdpSocket,
    select,
    sync::{
        mpsc::{self, Receiver, Sender},
        oneshot,
        RwLock,
    },
    time::timeout,
};
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use models::config::Server;

// WireGuard MTU-based buffer size (typical WireGuard packet max)
const UDP_BUF_SIZE: usize = 1500;

// Session inactivity timeout (WireGuard keepalive is typically 25s)
const SESSION_TIMEOUT: Duration = Duration::from_secs(180); // 3 minutes

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

/// Represents a persistent WireGuard session for a specific client
struct WireguardSession {
    session_id: Uuid,
    client_addr: SocketAddr,
    peer_id: PeerId,
    stream: LibP2pStream,
    last_activity: Instant,
    incoming_bytes: u64,
    outgoing_bytes: u64,
}

impl WireguardSession {
    fn new(
        client_addr: SocketAddr,
        peer_id: PeerId,
        stream: LibP2pStream,
    ) -> Self {
        Self {
            session_id: Uuid::new_v4(),
            client_addr,
            peer_id,
            stream,
            last_activity: Instant::now(),
            incoming_bytes: 0,
            outgoing_bytes: 0,
        }
    }

    fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    fn is_expired(&self) -> bool {
        self.last_activity.elapsed() > SESSION_TIMEOUT
    }
}

/// Manages WireGuard sessions with session affinity
struct SessionManager {
    sessions: Arc<RwLock<HashMap<SocketAddr, WireguardSession>>>,
    stream_pool: Arc<StreamPool>,
}

impl SessionManager {
    fn new(stream_pool: Arc<StreamPool>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            stream_pool,
        }
    }

    /// Get or create a session for a client address
    async fn get_or_create_session(
        &self,
        client_addr: SocketAddr,
        peer_id: PeerId,
    ) -> Result<Uuid, String> {
        // First check if session exists
        {
            let mut sessions = self.sessions.write().await;

            // Clean up expired sessions
            let expired: Vec<SocketAddr> = sessions
                .iter()
                .filter(|(_, session)| session.is_expired())
                .map(|(addr, _)| *addr)
                .collect();

            for addr in expired {
                if let Some(session) = sessions.remove(&addr) {
                    debug!("Cleaning up expired session {} for {}", session.session_id, addr);
                    counter!("p2proxy_wireguard_sessions_expired_total").increment(1);
                }
            }

            // Check for existing session
            if let Some(session) = sessions.get_mut(&client_addr) {
                session.touch();
                return Ok(session.session_id);
            }
        }

        // Need to create new session
        debug!("Creating new WireGuard session for {}", client_addr);

        let stream = self.stream_pool
            .acquire_stream(peer_id)
            .await
            .map_err(|e| format!("Failed to acquire stream: {}", e))?;

        let mut sessions = self.sessions.write().await;
        let session = WireguardSession::new(client_addr, peer_id, stream);
        let session_id = session.session_id;

        sessions.insert(client_addr, session);
        counter!("p2proxy_wireguard_sessions_created_total").increment(1);
        gauge!("p2proxy_wireguard_sessions_active").increment(1.0);

        Ok(session_id)
    }

    /// Send data through a client's session
    async fn send_to_peer(
        &self,
        client_addr: SocketAddr,
        data: &[u8],
    ) -> Result<usize, String> {
        let mut sessions = self.sessions.write().await;

        let session = sessions.get_mut(&client_addr)
            .ok_or_else(|| "Session not found".to_string())?;

        session.touch();

        session.stream.write_all(data).await
            .map_err(|e| format!("Failed to write to stream: {}", e))?;

        session.stream.flush().await
            .map_err(|e| format!("Failed to flush stream: {}", e))?;

        session.outgoing_bytes += data.len() as u64;

        Ok(data.len())
    }

    /// Receive data from a client's session (non-blocking)
    async fn recv_from_peer(
        &self,
        client_addr: SocketAddr,
        buf: &mut [u8],
    ) -> Result<usize, String> {
        let mut sessions = self.sessions.write().await;

        let session = sessions.get_mut(&client_addr)
            .ok_or_else(|| "Session not found".to_string())?;

        session.touch();

        // Non-blocking read with short timeout
        match timeout(Duration::from_millis(10), session.stream.read(buf)).await {
            Ok(Ok(n)) => {
                session.incoming_bytes += n as u64;
                Ok(n)
            }
            Ok(Err(e)) => Err(format!("Read error: {}", e)),
            Err(_) => Ok(0), // Timeout is not an error for UDP
        }
    }

    /// Close a session and return stream to pool
    async fn close_session(&self, client_addr: SocketAddr) -> Option<(PeerId, u64, u64)> {
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.remove(&client_addr) {
            let peer_id = session.peer_id;
            let incoming = session.incoming_bytes;
            let outgoing = session.outgoing_bytes;

            // Return stream to pool
            self.stream_pool.stream_closed(peer_id).await;

            gauge!("p2proxy_wireguard_sessions_active").decrement(1.0);
            counter!("p2proxy_wireguard_sessions_closed_total").increment(1);

            debug!("Closed session {} for {} (in: {} bytes, out: {} bytes)",
                   session.session_id, client_addr, incoming, outgoing);

            Some((peer_id, incoming, outgoing))
        } else {
            None
        }
    }

    /// Start background task to clean up expired sessions
    fn start_cleanup_task(self: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;

                let expired: Vec<SocketAddr> = {
                    let sessions = self.sessions.read().await;
                    sessions
                        .iter()
                        .filter(|(_, session)| session.is_expired())
                        .map(|(addr, _)| *addr)
                        .collect()
                };

                for addr in expired {
                    self.close_session(addr).await;
                }
            }
        });
    }
}

/// Creates a WireGuard proxy server that listens for WireGuard connections
/// and tunnels them through libp2p streams to a destination peer.
///
/// # Important Limitations
///
/// This is a FOUNDATION implementation that provides UDP packet forwarding
/// with session management. It does NOT include:
/// - WireGuard key management and cryptography
/// - TUN/TAP interface integration
/// - IP packet routing
/// - Complete WireGuard handshake state machine
///
/// For full WireGuard VPN functionality, additional components are required.
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
    let socket = Arc::new(UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], server_config.port))).await?);
    counter!("p2proxy_wireguard_server_started_total").increment(1);
    gauge!("p2proxy_wireguard_servers_active").increment(1.0);
    info!("WireGuard proxy listening on UDP port {}", server_config.port);

    let session_manager = Arc::new(SessionManager::new(stream_pool.clone()));

    // Start background cleanup task
    session_manager.clone().start_cleanup_task();

    // Spawn receive task
    let recv_socket = socket.clone();
    let recv_manager = session_manager.clone();
    let recv_sender = sender.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; UDP_BUF_SIZE];

        loop {
            match recv_socket.recv_from(&mut buf).await {
                Ok((len, addr)) => {
                    if len == 0 {
                        continue;
                    }

                    counter!("p2proxy_wireguard_packets_total").increment(1);
                    counter!("p2proxy_wireguard_bytes_received_total").increment(len as u64);

                    // Identify WireGuard message type
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
                            counter!("p2proxy_wireguard_data_packets_total").increment(1);
                        }
                        _ => {
                            warn!("Unknown WireGuard message type: {}", message_type);
                            counter!("p2proxy_wireguard_unknown_message_total").increment(1);
                        }
                    }

                    // Get or create session for this client
                    let session_id = match recv_manager.get_or_create_session(addr, peer).await {
                        Ok(id) => id,
                        Err(e) => {
                            error!("Failed to get session for {}: {}", addr, e);
                            counter!("p2proxy_wireguard_session_errors_total").increment(1);
                            let _ = recv_sender.send(WireguardStreamMessage::Error {
                                session_id: None,
                                error: e,
                                stage: SessionStage::PeerConnection,
                            }).await;
                            continue;
                        }
                    };

                    // Forward packet to peer through session
                    let packet_data = buf[..len].to_vec();
                    match recv_manager.send_to_peer(addr, &packet_data).await {
                        Ok(sent) => {
                            histogram!("p2proxy_wireguard_packet_size_bytes").record(len as f64);

                            let _ = recv_sender.send(WireguardStreamMessage::DataTransferred {
                                session_id,
                                direction: DataDirection::Outgoing,
                                bytes: sent,
                            }).await;
                        }
                        Err(e) => {
                            error!("Failed to forward packet from {} to peer: {}", addr, e);
                            counter!("p2proxy_wireguard_write_errors_total").increment(1);

                            let _ = recv_sender.send(WireguardStreamMessage::Error {
                                session_id: Some(session_id),
                                error: e.clone(),
                                stage: SessionStage::DataTransfer,
                            }).await;

                            // Close failed session
                            recv_manager.close_session(addr).await;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to receive UDP packet: {}", e);
                    counter!("p2proxy_wireguard_receive_errors_total").increment(1);

                    let _ = recv_sender.send(WireguardStreamMessage::Error {
                        session_id: None,
                        error: format!("UDP receive error: {}", e),
                        stage: SessionStage::DataTransfer,
                    }).await;
                }
            }
        }
    });

    // Spawn send task - reads from peer streams and sends back to clients
    let send_socket = socket.clone();
    let send_manager = session_manager.clone();
    let send_sender = sender.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; UDP_BUF_SIZE];

        loop {
            // Poll all active sessions for incoming data
            let clients: Vec<SocketAddr> = {
                let sessions = send_manager.sessions.read().await;
                sessions.keys().copied().collect()
            };

            for client_addr in clients {
                match send_manager.recv_from_peer(client_addr, &mut buf).await {
                    Ok(0) => continue, // No data available
                    Ok(n) => {
                        counter!("p2proxy_wireguard_bytes_sent_total").increment(n as u64);
                        counter!("p2proxy_wireguard_responses_received_total").increment(1);

                        // Send response back to client
                        if let Err(e) = send_socket.send_to(&buf[..n], client_addr).await {
                            error!("Failed to send response to client {}: {}", client_addr, e);
                            counter!("p2proxy_wireguard_send_errors_total").increment(1);
                        } else {
                            // Get session ID for event
                            let session_id = {
                                let sessions = send_manager.sessions.read().await;
                                sessions.get(&client_addr).map(|s| s.session_id)
                            };

                            if let Some(session_id) = session_id {
                                let _ = send_sender.send(WireguardStreamMessage::DataTransferred {
                                    session_id,
                                    direction: DataDirection::Incoming,
                                    bytes: n,
                                }).await;
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to read from peer for client {}: {}", client_addr, e);
                        counter!("p2proxy_wireguard_read_errors_total").increment(1);
                        send_manager.close_session(client_addr).await;
                    }
                }
            }

            // Small sleep to prevent busy-waiting
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });

    Ok(())
}
