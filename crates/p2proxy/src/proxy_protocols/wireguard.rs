// wireguard.rs
use bitping_tcp_proxy::bandwidth_reporter::BandwidthReport;
use color_eyre::eyre::{Context as _, Result};
use dashmap::DashMap;
use futures::{AsyncReadExt, AsyncWriteExt, Stream};
use libp2p::{identity::Keypair, PeerId, Stream as LibP2pStream};
use crate::stream_pool::StreamPool;
use metrics::{counter, gauge, histogram};
use std::{
    collections::HashMap,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::UdpSocket,
    sync::{
        mpsc::{self, Receiver, Sender},
        oneshot, Semaphore,
    },
    time::timeout,
};
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use models::config::Server;

// WireGuard MTU-based buffer size (typical WireGuard packet max)
const UDP_BUF_SIZE: usize = 1500;

// Session inactivity timeout (reduced from 3 minutes to 90 seconds)
const DEFAULT_SESSION_TIMEOUT: Duration = Duration::from_secs(90);

// Maximum number of concurrent sessions per server
const MAX_SESSIONS_PER_SERVER: usize = 10000;

// Rate limiting: maximum packets per second per client
const MAX_PACKETS_PER_SEC_PER_CLIENT: u32 = 1000;
const RATE_LIMIT_REFILL_INTERVAL: Duration = Duration::from_millis(100);

// WireGuard protocol constants and packet size validation
const WIREGUARD_MESSAGE_HANDSHAKE_INITIATION: u8 = 1;
const WIREGUARD_MESSAGE_HANDSHAKE_RESPONSE: u8 = 2;
const WIREGUARD_MESSAGE_COOKIE_REPLY: u8 = 3;
const WIREGUARD_MESSAGE_DATA: u8 = 4;

// Minimum packet sizes for each WireGuard message type
const MIN_HANDSHAKE_INITIATION_SIZE: usize = 148;
const MIN_HANDSHAKE_RESPONSE_SIZE: usize = 92;
const MIN_COOKIE_REPLY_SIZE: usize = 64;
const MIN_DATA_PACKET_SIZE: usize = 32;

/// WireGuard-specific error types
#[derive(Error, Debug)]
pub enum WireguardError {
    #[error("Session not found for address {0}")]
    SessionNotFound(SocketAddr),

    #[error("Maximum session limit ({0}) reached")]
    MaxSessionsReached(usize),

    #[error("Rate limit exceeded for client {0}")]
    RateLimitExceeded(SocketAddr),

    #[error("Invalid packet size {size} for message type {msg_type}")]
    InvalidPacketSize { msg_type: u8, size: usize },

    #[error("Failed to acquire stream: {0}")]
    StreamAcquisitionFailed(String),

    #[error("Failed to write to stream: {0}")]
    StreamWriteFailed(String),

    #[error("Failed to read from stream: {0}")]
    StreamReadFailed(String),

    #[error("UDP socket error: {0}")]
    UdpSocketError(String),
}

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

/// Token bucket for rate limiting
struct RateLimiter {
    tokens: Arc<Semaphore>,
    last_refill: Arc<tokio::sync::Mutex<Instant>>,
}

impl RateLimiter {
    fn new(max_tokens: usize) -> Self {
        Self {
            tokens: Arc::new(Semaphore::new(max_tokens)),
            last_refill: Arc::new(tokio::sync::Mutex::new(Instant::now())),
        }
    }

    async fn try_acquire(&self) -> bool {
        // Refill tokens based on elapsed time
        {
            let mut last_refill = self.last_refill.lock().await;
            let elapsed = last_refill.elapsed();
            if elapsed >= RATE_LIMIT_REFILL_INTERVAL {
                let tokens_to_add = (elapsed.as_millis() as u32 * MAX_PACKETS_PER_SEC_PER_CLIENT) / 1000;
                if tokens_to_add > 0 {
                    // Add tokens up to the maximum
                    for _ in 0..tokens_to_add.min(MAX_PACKETS_PER_SEC_PER_CLIENT) {
                        if self.tokens.available_permits() < MAX_PACKETS_PER_SEC_PER_CLIENT as usize {
                            self.tokens.add_permits(1);
                        }
                    }
                    *last_refill = Instant::now();
                }
            }
        }

        // Try to acquire a token (non-blocking)
        self.tokens.try_acquire().is_ok()
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
    rate_limiter: RateLimiter,
    /// Handle to the background receive task for this session
    _recv_task_handle: tokio::task::JoinHandle<()>,
}

impl WireguardSession {
    fn new(
        client_addr: SocketAddr,
        peer_id: PeerId,
        mut stream: LibP2pStream,
        socket: Arc<UdpSocket>,
        sender: Sender<WireguardStreamMessage>,
    ) -> Self {
        let session_id = Uuid::new_v4();
        let rate_limiter = RateLimiter::new(MAX_PACKETS_PER_SEC_PER_CLIENT as usize);

        // Spawn a dedicated receive task for this session
        let recv_session_id = session_id;
        let recv_client_addr = client_addr;
        let recv_sender = sender.clone();
        let recv_task_handle = tokio::spawn(async move {
            let mut buf = vec![0u8; UDP_BUF_SIZE];

            loop {
                match timeout(Duration::from_millis(100), stream.read(&mut buf)).await {
                    Ok(Ok(n)) if n > 0 => {
                        counter!("p2proxy_wireguard_bytes_sent_total").increment(n as u64);
                        counter!("p2proxy_wireguard_responses_received_total").increment(1);

                        // Send response back to client via UDP
                        if let Err(e) = socket.send_to(&buf[..n], recv_client_addr).await {
                            error!("Failed to send response to client {}: {}", recv_client_addr, e);
                            counter!("p2proxy_wireguard_send_errors_total").increment(1);
                            break;
                        } else {
                            let _ = recv_sender.send(WireguardStreamMessage::DataTransferred {
                                session_id: recv_session_id,
                                direction: DataDirection::Incoming,
                                bytes: n,
                            }).await;
                        }
                    }
                    Ok(Ok(_)) => {
                        debug!("Stream EOF for session {}", recv_session_id);
                        break; // EOF
                    }
                    Ok(Err(e)) => {
                        error!("Read error on session {}: {}", recv_session_id, e);
                        counter!("p2proxy_wireguard_read_errors_total").increment(1);
                        break;
                    }
                    Err(_) => {
                        // Timeout - continue polling
                        continue;
                    }
                }
            }

            debug!("Receive task terminated for session {}", recv_session_id);
        });

        Self {
            session_id,
            client_addr,
            peer_id,
            stream,
            last_activity: Instant::now(),
            incoming_bytes: 0,
            outgoing_bytes: 0,
            rate_limiter,
            _recv_task_handle: recv_task_handle,
        }
    }

    fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    fn is_expired(&self) -> bool {
        self.last_activity.elapsed() > DEFAULT_SESSION_TIMEOUT
    }

    async fn check_rate_limit(&self) -> Result<(), WireguardError> {
        if !self.rate_limiter.try_acquire().await {
            Err(WireguardError::RateLimitExceeded(self.client_addr))
        } else {
            Ok(())
        }
    }
}

/// Manages WireGuard sessions with session affinity
struct SessionManager {
    sessions: Arc<DashMap<SocketAddr, WireguardSession>>,
    stream_pool: Arc<StreamPool>,
    socket: Arc<UdpSocket>,
    sender: Sender<WireguardStreamMessage>,
}

impl SessionManager {
    fn new(
        stream_pool: Arc<StreamPool>,
        socket: Arc<UdpSocket>,
        sender: Sender<WireguardStreamMessage>,
    ) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            stream_pool,
            socket,
            sender,
        }
    }

    /// Get or create a session for a client address using double-checked locking
    async fn get_or_create_session(
        &self,
        client_addr: SocketAddr,
        peer_id: PeerId,
    ) -> Result<Uuid, WireguardError> {
        // Fast path: check if session exists (read-only access)
        if let Some(mut session) = self.sessions.get_mut(&client_addr) {
            if !session.is_expired() {
                session.touch();
                return Ok(session.session_id);
            }
            // Session expired, will be recreated below
        }

        // Check session limit before creating new session
        if self.sessions.len() >= MAX_SESSIONS_PER_SERVER {
            return Err(WireguardError::MaxSessionsReached(MAX_SESSIONS_PER_SERVER));
        }

        debug!("Creating new WireGuard session for {}", client_addr);

        // Acquire stream (may be slow, don't hold any locks during this)
        let stream = self.stream_pool
            .acquire_stream(peer_id)
            .await
            .map_err(|e| WireguardError::StreamAcquisitionFailed(e.to_string()))?;

        // Double-checked locking: check again if session was created while we were acquiring stream
        if let Some(mut session) = self.sessions.get_mut(&client_addr) {
            if !session.is_expired() {
                // Another task created the session, return it
                // The stream we acquired will be dropped and returned to pool
                drop(stream);
                session.touch();
                return Ok(session.session_id);
            }
            // Session exists but is expired, remove it
            drop(session); // Release reference before removing
            self.sessions.remove(&client_addr);
        }

        // Create new session with dedicated receive task
        let session = WireguardSession::new(
            client_addr,
            peer_id,
            stream,
            self.socket.clone(),
            self.sender.clone(),
        );
        let session_id = session.session_id;

        self.sessions.insert(client_addr, session);
        counter!("p2proxy_wireguard_sessions_created_total").increment(1);
        gauge!("p2proxy_wireguard_sessions_active").increment(1.0);

        Ok(session_id)
    }

    /// Send data through a client's session
    async fn send_to_peer(
        &self,
        client_addr: SocketAddr,
        data: &[u8],
    ) -> Result<usize, WireguardError> {
        let mut session = self.sessions.get_mut(&client_addr)
            .ok_or(WireguardError::SessionNotFound(client_addr))?;

        // Check rate limit
        session.check_rate_limit().await?;

        session.touch();

        session.stream.write_all(data).await
            .map_err(|e| WireguardError::StreamWriteFailed(e.to_string()))?;

        session.stream.flush().await
            .map_err(|e| WireguardError::StreamWriteFailed(e.to_string()))?;

        session.outgoing_bytes += data.len() as u64;

        Ok(data.len())
    }

    /// Close a session and return stream to pool
    async fn close_session(&self, client_addr: SocketAddr) -> Option<(PeerId, u64, u64)> {
        if let Some((_, session)) = self.sessions.remove(&client_addr) {
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

                let expired: Vec<SocketAddr> = self.sessions
                    .iter()
                    .filter(|entry| entry.value().is_expired())
                    .map(|entry| *entry.key())
                    .collect();

                for addr in expired {
                    debug!("Cleaning up expired session for {}", addr);
                    self.close_session(addr).await;
                    counter!("p2proxy_wireguard_sessions_expired_total").increment(1);
                }
            }
        });
    }
}

/// Validate WireGuard packet size based on message type
fn validate_packet_size(message_type: u8, size: usize) -> Result<(), WireguardError> {
    let min_size = match message_type {
        WIREGUARD_MESSAGE_HANDSHAKE_INITIATION => MIN_HANDSHAKE_INITIATION_SIZE,
        WIREGUARD_MESSAGE_HANDSHAKE_RESPONSE => MIN_HANDSHAKE_RESPONSE_SIZE,
        WIREGUARD_MESSAGE_COOKIE_REPLY => MIN_COOKIE_REPLY_SIZE,
        WIREGUARD_MESSAGE_DATA => MIN_DATA_PACKET_SIZE,
        _ => return Ok(()), // Unknown message type, skip validation
    };

    if size < min_size {
        return Err(WireguardError::InvalidPacketSize { msg_type: message_type, size });
    }

    Ok(())
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
/// See WIREGUARD_LIMITATIONS.md for details.
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
    let socket = Arc::new(
        UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], server_config.port)))
            .await
            .with_context(|| format!("Failed to bind WireGuard UDP socket on port {}", server_config.port))?
    );

    counter!("p2proxy_wireguard_server_started_total").increment(1);
    gauge!("p2proxy_wireguard_servers_active").increment(1.0);
    info!("WireGuard proxy listening on UDP port {}", server_config.port);

    let session_manager = Arc::new(SessionManager::new(
        stream_pool.clone(),
        socket.clone(),
        sender.clone(),
    ));

    // Start background cleanup task
    session_manager.clone().start_cleanup_task();

    // Spawn main receive task (handles incoming UDP packets from clients)
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

                    // Validate packet has at least message type byte
                    if len < 1 {
                        warn!("Received packet too small from {}: {} bytes", addr, len);
                        counter!("p2proxy_wireguard_invalid_packet_total").increment(1);
                        continue;
                    }

                    // Identify and validate WireGuard message type
                    let message_type = buf[0];

                    // Validate packet size
                    if let Err(e) = validate_packet_size(message_type, len) {
                        warn!("Invalid packet from {}: {}", addr, e);
                        counter!("p2proxy_wireguard_invalid_packet_total").increment(1);
                        continue;
                    }

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
                                error: e.to_string(),
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
                            match &e {
                                WireguardError::RateLimitExceeded(_) => {
                                    debug!("Rate limit exceeded for {}", addr);
                                    counter!("p2proxy_wireguard_rate_limited_total").increment(1);
                                }
                                _ => {
                                    error!("Failed to forward packet from {} to peer: {}", addr, e);
                                    counter!("p2proxy_wireguard_write_errors_total").increment(1);

                                    let _ = recv_sender.send(WireguardStreamMessage::Error {
                                        session_id: Some(session_id),
                                        error: e.to_string(),
                                        stage: SessionStage::DataTransfer,
                                    }).await;

                                    // Close failed session (except for rate limit errors)
                                    recv_manager.close_session(addr).await;
                                }
                            }
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

    Ok(())
}
