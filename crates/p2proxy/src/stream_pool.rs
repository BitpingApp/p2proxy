use bitping_tcp_proxy::TCP_PROXY_PROTOCOL;
use color_eyre::eyre::{eyre, Result};
use libp2p::{identity::Keypair, PeerId, Stream};
use libp2p_stream as stream;
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
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_concurrent_per_peer: 20,
            stream_open_timeout: Duration::from_secs(10),
            enabled: true,
        }
    }
}

/// Statistics for connection management
#[derive(Debug, Default, Clone)]
struct PeerStats {
    total_opened: u64,
    total_failed: u64,
    current_active: usize,
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
        Self {
            peer_id,
            stats: PeerStats::default(),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
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
    pub fn new(
        control: stream::Control,
        config: PoolConfig,
    ) -> Arc<Self> {
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
    pub async fn acquire_stream(&self, peer: PeerId) -> Result<Stream> {
        if !self.config.enabled {
            // Management disabled, open stream directly
            let mut control = self.control.clone();
            return control.open_stream(peer, TCP_PROXY_PROTOCOL)
                .await
                .map_err(|e| eyre!("Failed to open stream: {}", e));
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
                return Err(eyre!("Semaphore acquisition failed: {}", e));
            }
            Err(_) => {
                counter!("p2proxy_stream_acquire_timeout_total").increment(1);
                self.record_failure(peer).await;
                return Err(eyre!("Timeout waiting for stream slot"));
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
            eyre!("Timeout opening stream to peer {}", peer)
        })?
        .map_err(|e| {
            self.record_failure_sync(peer);
            eyre!("Failed to open stream to peer {}: {}", peer, e)
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
            counter!("p2proxy_stream_opened_success_total", "peer" => peer.to_string()).increment(1);
        }
    }

    /// Record failed stream opening (async version)
    async fn record_failure(&self, peer: PeerId) {
        let mut peers = self.peers.write().await;
        if let Some(peer_conn) = peers.get_mut(&peer) {
            peer_conn.stats.total_failed += 1;
            if peer_conn.stats.current_active > 0 {
                peer_conn.stats.current_active -= 1;
            }
            gauge!("p2proxy_stream_pool_active_total", "peer" => peer.to_string())
                .set(peer_conn.stats.current_active as f64);
            counter!("p2proxy_stream_opened_failed_total", "peer" => peer.to_string()).increment(1);
        }
    }

    /// Record failed stream opening (sync version for use in closures)
    fn record_failure_sync(&self, peer: PeerId) {
        let peers_clone = self.peers.clone();
        tokio::spawn(async move {
            let mut peers = peers_clone.write().await;
            if let Some(peer_conn) = peers.get_mut(&peer) {
                peer_conn.stats.total_failed += 1;
                if peer_conn.stats.current_active > 0 {
                    peer_conn.stats.current_active -= 1;
                }
                gauge!("p2proxy_stream_pool_active_total", "peer" => peer.to_string())
                    .set(peer_conn.stats.current_active as f64);
                counter!("p2proxy_stream_opened_failed_total", "peer" => peer.to_string()).increment(1);
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
}
