use color_eyre::eyre::{Result, eyre};
use libp2p::{PeerId, Stream, StreamProtocol, identity::Keypair};
use libp2p_stream as stream;
use libp2p_stream::OpenStreamError;
use p2p_bandwidth_protocol::TCP_PROXY_PROTOCOL;

/// Typed errors from `StreamPool::acquire_stream`. Modelled as an enum
/// (rather than an `eyre::Report`) so call sites can pattern-match on
/// `UnsupportedProtocol` — which is the signal that the destination peer
/// is running an old `bitping-tcp-forwarder` and we should evict it and
/// find a replacement, not retry indefinitely. The fix for BIT-? where
/// nodes running tcp-forwarder/1.x kept getting picked over and over.
#[derive(thiserror::Error, Debug)]
pub enum StreamPoolError {
    /// Remote peer answered the libp2p multistream-select negotiation
    /// with a protocol-rejection — they don't speak our TCP proxy
    /// protocol version. Definitive: this peer will never work, evict
    /// and rediscover.
    #[error("remote peer {peer} does not support {protocol}")]
    UnsupportedProtocol {
        peer: PeerId,
        protocol: StreamProtocol,
    },
    /// IO failure during the stream open / negotiation. Could be
    /// transient (connection dropped mid-handshake) or persistent.
    /// Caller should retry but not necessarily evict.
    #[error("io error opening stream to {peer}: {source}")]
    Io {
        peer: PeerId,
        #[source]
        source: std::io::Error,
    },
    /// We waited longer than `stream_open_timeout` and the open never
    /// completed. Most often means the peer disappeared mid-open.
    #[error("timeout opening stream to {peer}")]
    OpenTimeout { peer: PeerId },
    /// Pool concurrency semaphore couldn't be acquired (process-level
    /// state corruption). Shouldn't happen in practice.
    #[error("semaphore acquisition failed: {0}")]
    Semaphore(#[from] tokio::sync::AcquireError),
}

impl StreamPoolError {
    /// Whether this error means the peer is fundamentally unable to
    /// serve us — caller should drop the peer and rediscover rather
    /// than retrying the same one.
    pub fn is_terminal_for_peer(&self) -> bool {
        matches!(self, StreamPoolError::UnsupportedProtocol { .. })
    }
}

/// Helper to translate the libp2p `OpenStreamError` into our typed shape,
/// tagging the peer so callers don't have to thread it back through.
fn map_open_error(peer: PeerId, e: OpenStreamError) -> StreamPoolError {
    match e {
        OpenStreamError::UnsupportedProtocol(protocol) => {
            StreamPoolError::UnsupportedProtocol { peer, protocol }
        }
        OpenStreamError::Io(source) => StreamPoolError::Io { peer, source },
        // libp2p_stream's `OpenStreamError` is `#[non_exhaustive]` so we
        // have to handle the future-additions case. Treat unknown
        // variants as Io with a synthetic kind — preserves the message
        // and lets the existing IO-error path take over.
        other => StreamPoolError::Io {
            peer,
            source: std::io::Error::new(std::io::ErrorKind::Other, format!("{other}")),
        },
    }
}
use metrics::{counter, gauge, histogram};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{RwLock, Semaphore};
use tracing::{debug, instrument};

/// Configuration for the stream pool/manager
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum concurrent streams per peer
    pub max_concurrent_per_peer: usize,
    /// Timeout for opening a new stream
    pub stream_open_timeout: Duration,
    /// Whether connection management is enabled (for rollback)
    pub enabled: bool,
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Timeout for peer health checks
    /// Maximum error rate before triggering failover (0.0-1.0)
    pub max_error_rate: f64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_concurrent_per_peer: 5,
            stream_open_timeout: Duration::from_secs(20),
            enabled: true,
            max_retries: 3,
            max_error_rate: 0.15,
        }
    }
}

impl From<&models::config::PoolConfigOptions> for PoolConfig {
    fn from(opts: &models::config::PoolConfigOptions) -> Self {
        Self {
            max_concurrent_per_peer: opts.max_total,
            stream_open_timeout: Duration::from_secs(opts.open_timeout_secs),
            enabled: opts.enabled,
            max_retries: opts.max_retries,
            max_error_rate: opts.max_error_rate,
        }
    }
}

/// Statistics for connection management
#[derive(Debug, Default, Clone)]
struct PeerStats {
    total_opened: u64,
    total_failed: u64,
    current_active: usize,
    /// Error rate calculation window (last N attempts)
    recent_successes: u64,
    recent_failures: u64,
    /// Whether peer is currently healthy
    is_healthy: bool,
}

/// Per-peer connection tracking
struct PeerConnection {
    peer_id: PeerId,
    stats: PeerStats,
    /// Semaphore to limit concurrent streams to this peer
    semaphore: Arc<Semaphore>,
}

impl PeerConnection {
    fn new(peer_id: PeerId, max_concurrent: usize) -> Self {
        let mut stats = PeerStats::default();
        stats.is_healthy = true; // Assume healthy initially
        Self {
            peer_id,
            stats,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    /// Calculate current error rate based on recent activity
    fn error_rate(&self) -> f64 {
        let total = self.stats.recent_successes + self.stats.recent_failures;
        if total == 0 {
            return 0.0;
        }
        self.stats.recent_failures as f64 / total as f64
    }

    /// Reset recent stats (for sliding window)
    fn reset_recent_stats(&mut self) {
        const MAX_WINDOW_SIZE: u64 = 100;
        let total = self.stats.recent_successes + self.stats.recent_failures;
        if total > MAX_WINDOW_SIZE {
            // Keep sliding window
            self.stats.recent_successes = self.stats.recent_successes / 2;
            self.stats.recent_failures = self.stats.recent_failures / 2;
        }
    }
}

/// Manages P2P stream connections with rate limiting and monitoring
pub struct StreamPool {
    peers: Arc<RwLock<HashMap<PeerId, PeerConnection>>>,
    control: stream::Control,
    config: PoolConfig,
}

impl StreamPool {
    /// Create a new stream manager
    pub fn new(control: stream::Control, config: PoolConfig) -> Arc<Self> {
        let pool = Arc::new(Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            control,
            config,
        });

        // Start background metrics task
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            pool_clone.metrics_task().await;
        });

        pool
    }

    /// Open a stream to the given peer with rate limiting and timeout
    #[instrument(skip(self), fields(peer = %peer))]
    pub async fn acquire_stream(
        &self,
        peer: PeerId,
    ) -> std::result::Result<Stream, StreamPoolError> {
        if !self.config.enabled {
            // Management disabled, open stream directly
            let mut control = self.control.clone();
            return control
                .open_stream(peer, TCP_PROXY_PROTOCOL)
                .await
                .map_err(|e| map_open_error(peer, e));
        }

        let start = Instant::now();

        // Get or create peer connection tracker
        let semaphore = {
            let mut peers = self.peers.write().await;
            let peer_conn = peers
                .entry(peer)
                .or_insert_with(|| PeerConnection::new(peer, self.config.max_concurrent_per_peer));
            peer_conn.stats.current_active += 1;
            gauge!("p2proxy_stream_pool_active_total", "peer" => peer.to_string())
                .set(peer_conn.stats.current_active as f64);
            peer_conn.semaphore.clone()
        };

        // Acquire semaphore permit to limit concurrent streams
        let _permit = match tokio::time::timeout(
            self.config.stream_open_timeout,
            semaphore.acquire(),
        )
        .await
        {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                self.record_failure(peer).await;
                return Err(StreamPoolError::from(e));
            }
            Err(_) => {
                counter!("p2proxy_stream_acquire_timeout_total").increment(1);
                self.record_failure(peer).await;
                return Err(StreamPoolError::OpenTimeout { peer });
            }
        };

        // Open the stream
        let mut control = self.control.clone();
        let stream = tokio::time::timeout(
            self.config.stream_open_timeout,
            control.open_stream(peer, TCP_PROXY_PROTOCOL),
        )
        .await
        .map_err(|_| {
            self.record_failure_sync(peer);
            StreamPoolError::OpenTimeout { peer }
        })?
        .map_err(|e| {
            self.record_failure_sync(peer);
            map_open_error(peer, e)
        })?;

        // Record success
        self.record_success(peer).await;

        let duration = start.elapsed();
        histogram!("p2proxy_stream_acquire_duration_seconds").record(duration.as_secs_f64());
        counter!("p2proxy_stream_opened_total").increment(1);
        counter!("p2proxy_stream_pool_acquire_total").increment(1);
        debug!("Opened stream in {:?}", duration);

        Ok(stream)
    }

    /// Record successful stream opening
    async fn record_success(&self, peer: PeerId) {
        let mut peers = self.peers.write().await;
        if let Some(peer_conn) = peers.get_mut(&peer) {
            peer_conn.stats.total_opened += 1;
            peer_conn.stats.recent_successes += 1;
            peer_conn.reset_recent_stats();

            // Update health status based on error rate
            let error_rate = peer_conn.error_rate();
            if error_rate < self.config.max_error_rate {
                peer_conn.stats.is_healthy = true;
            }

            counter!("p2proxy_stream_opened_success_total", "peer" => peer.to_string())
                .increment(1);
            gauge!("p2proxy_peer_error_rate", "peer" => peer.to_string()).set(error_rate);
        }
    }

    /// Record failed stream opening (async version)
    async fn record_failure(&self, peer: PeerId) {
        let mut peers = self.peers.write().await;
        if let Some(peer_conn) = peers.get_mut(&peer) {
            peer_conn.stats.total_failed += 1;
            peer_conn.stats.recent_failures += 1;
            peer_conn.reset_recent_stats();

            if peer_conn.stats.current_active > 0 {
                peer_conn.stats.current_active -= 1;
            }

            // Check if error rate exceeds threshold
            let error_rate = peer_conn.error_rate();
            if error_rate >= self.config.max_error_rate {
                peer_conn.stats.is_healthy = false;
                counter!("p2proxy_peer_failover_total", "peer" => peer.to_string()).increment(1);
                debug!(
                    "Peer {} marked unhealthy due to high error rate: {:.2}%",
                    peer,
                    error_rate * 100.0
                );
            }

            gauge!("p2proxy_stream_pool_active_total", "peer" => peer.to_string())
                .set(peer_conn.stats.current_active as f64);
            gauge!("p2proxy_peer_error_rate", "peer" => peer.to_string()).set(error_rate);
            counter!("p2proxy_stream_opened_failed_total", "peer" => peer.to_string()).increment(1);
        }
    }

    /// Record failed stream opening (sync version for use in closures)
    fn record_failure_sync(&self, peer: PeerId) {
        let peers_clone = self.peers.clone();
        let max_error_rate = self.config.max_error_rate;
        tokio::spawn(async move {
            let mut peers = peers_clone.write().await;
            if let Some(peer_conn) = peers.get_mut(&peer) {
                peer_conn.stats.total_failed += 1;
                peer_conn.stats.recent_failures += 1;
                peer_conn.reset_recent_stats();

                if peer_conn.stats.current_active > 0 {
                    peer_conn.stats.current_active -= 1;
                }

                // Check if error rate exceeds threshold
                let error_rate = peer_conn.error_rate();
                if error_rate >= max_error_rate {
                    peer_conn.stats.is_healthy = false;
                    counter!("p2proxy_peer_failover_total", "peer" => peer.to_string())
                        .increment(1);
                }

                gauge!("p2proxy_stream_pool_active_total", "peer" => peer.to_string())
                    .set(peer_conn.stats.current_active as f64);
                gauge!("p2proxy_peer_error_rate", "peer" => peer.to_string()).set(error_rate);
                counter!("p2proxy_stream_opened_failed_total", "peer" => peer.to_string())
                    .increment(1);
            }
        });
    }

    /// Notify that a stream has been closed
    pub async fn stream_closed(&self, peer: PeerId) {
        let mut peers = self.peers.write().await;
        if let Some(peer_conn) = peers.get_mut(&peer) {
            if peer_conn.stats.current_active > 0 {
                peer_conn.stats.current_active -= 1;
            }
            gauge!("p2proxy_stream_pool_active_total", "peer" => peer.to_string())
                .set(peer_conn.stats.current_active as f64);
        }
    }

    /// Background task to update metrics
    async fn metrics_task(self: Arc<Self>) {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;

            let peers = self.peers.read().await;
            for (peer_id, peer_conn) in peers.iter() {
                gauge!("p2proxy_stream_pool_active_total", "peer" => peer_id.to_string())
                    .set(peer_conn.stats.current_active as f64);
            }
        }
    }

    /// Get current statistics (for debugging/monitoring)
    pub async fn get_stats(&self) -> HashMap<PeerId, PeerStats> {
        let peers = self.peers.read().await;
        peers
            .iter()
            .map(|(peer, conn)| (*peer, conn.stats.clone()))
            .collect()
    }

    /// Check if a peer is healthy (error rate below threshold)
    pub async fn is_peer_healthy(&self, peer: &PeerId) -> bool {
        let peers = self.peers.read().await;
        peers
            .get(peer)
            .map(|conn| conn.stats.is_healthy)
            .unwrap_or(true) // Assume healthy if not tracked yet
    }

    /// Get error rate for a peer
    pub async fn get_peer_error_rate(&self, peer: &PeerId) -> f64 {
        let peers = self.peers.read().await;
        peers.get(peer).map(|conn| conn.error_rate()).unwrap_or(0.0)
    }
}
