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
use tokio::sync::{RwLock, Semaphore, SemaphorePermit};
use tracing::{debug, instrument, warn};

/// Configuration for the stream pool/manager
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum concurrent streams per peer
    pub max_concurrent_per_peer: usize,
    /// Timeout for opening a new stream (P2P network operation)
    pub stream_open_timeout: Duration,
    /// Timeout for acquiring semaphore permit (rate limiting)
    pub semaphore_timeout: Duration,
    /// Whether connection management is enabled (for rollback)
    pub enabled: bool,
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Timeout for peer health checks
    pub health_check_timeout: Duration,
    /// Maximum error rate before triggering failover (0.0-1.0)
    pub max_error_rate: f64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_concurrent_per_peer: 30,
            stream_open_timeout: Duration::from_secs(20),
            semaphore_timeout: Duration::from_secs(5),
            enabled: true,
            max_retries: 3,
            health_check_timeout: Duration::from_secs(5),
            max_error_rate: 0.15,
        }
    }
}

impl From<&models::config::PoolConfigOptions> for PoolConfig {
    fn from(opts: &models::config::PoolConfigOptions) -> Self {
        Self {
            max_concurrent_per_peer: opts.max_total,
            stream_open_timeout: Duration::from_secs(opts.open_timeout_secs),
            semaphore_timeout: Duration::from_secs(opts.semaphore_timeout_secs.unwrap_or(5)),
            enabled: opts.enabled,
            max_retries: opts.max_retries,
            health_check_timeout: Duration::from_secs(opts.health_check_timeout_secs),
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
    /// Timestamp of last health check
    last_health_check: Option<Instant>,
    /// Whether peer is currently healthy
    is_healthy: bool,
    /// Exponential moving average of response times (in seconds)
    avg_response_time: f64,
    /// Total number of response time samples
    response_time_samples: u64,
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

/// RAII guard that automatically decrements active count on drop
///
/// This guard ensures the active stream counter is properly managed even if:
/// - The stream open operation panics
/// - An error occurs during acquisition
/// - The function returns early
struct StreamGuard {
    peer: PeerId,
    peers: Arc<RwLock<HashMap<PeerId, PeerConnection>>>,
    /// If true, counter has already been handled (don't decrement in Drop)
    already_handled: bool,
}

impl StreamGuard {
    fn new(peer: PeerId, peers: Arc<RwLock<HashMap<PeerId, PeerConnection>>>) -> Self {
        Self {
            peer,
            peers,
            already_handled: false,
        }
    }

    /// Mark as successful - prevents decrement on drop
    ///
    /// Call this when the stream is successfully opened and ownership
    /// is transferred elsewhere. The counter will be decremented when
    /// the stream is closed later.
    fn mark_success(&mut self) {
        self.already_handled = true;
    }

    /// Manually trigger failure decrement (for sync contexts)
    ///
    /// This is needed in contexts where we can't use async in Drop.
    fn trigger_failure(&mut self) {
        if !self.already_handled {
            self.decrement_counter();
            self.already_handled = true;
        }
    }

    /// Decrement the counter (shared logic)
    fn decrement_counter(&self) {
        // Use try_write since this might be called from Drop
        if let Ok(mut peers) = self.peers.try_write() {
            if let Some(peer_conn) = peers.get_mut(&self.peer) {
                if peer_conn.stats.current_active > 0 {
                    peer_conn.stats.current_active -= 1;
                }
                gauge!("p2proxy_stream_pool_active_total", "peer" => self.peer.to_string())
                    .set(peer_conn.stats.current_active as f64);
            }
        } else {
            // Lock contention - log but don't block Drop
            tracing::warn!(
                "Could not acquire lock to decrement counter for peer {} in StreamGuard",
                self.peer
            );
            counter!("p2proxy_stream_guard_lock_contention_total").increment(1);
        }
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        if !self.already_handled {
            // This runs on panic or error - decrement the counter
            tracing::debug!(
                "StreamGuard dropped without explicit handling - \
                 decrementing active count for peer {}",
                self.peer
            );
            counter!("p2proxy_stream_guard_auto_cleanup_total").increment(1);

            self.decrement_counter();
        }
    }
}

/// Manages P2P stream connections with rate limiting and monitoring
pub struct StreamPool {
    peers: Arc<RwLock<HashMap<PeerId, PeerConnection>>>,
    control: stream::Control,
    config: PoolConfig,
    /// Available peer list for load balancing
    available_peers: Arc<RwLock<Vec<PeerId>>>,
    /// Round-robin counter for peer selection
    round_robin_counter: Arc<RwLock<usize>>,
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
            available_peers: Arc::new(RwLock::new(Vec::new())),
            round_robin_counter: Arc::new(RwLock::new(0)),
        });

        // Start background metrics task
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            pool_clone.metrics_task().await;
        });

        pool
    }

    /// Add a peer to the available peer list for load balancing
    pub async fn add_peer(&self, peer: PeerId) {
        let mut peers = self.available_peers.write().await;
        if !peers.contains(&peer) {
            peers.push(peer);
            debug!("Added peer {} to pool (total: {})", peer, peers.len());
        }
    }

    /// Select next peer using health-based weighted strategy
    ///
    /// Peer selection algorithm:
    /// 1. Filter out unhealthy peers (high error rate)
    /// 2. Calculate health score for each peer based on:
    ///    - Error rate (lower is better)
    ///    - Response time (faster is better)
    /// 3. Prefer peers with higher health scores
    /// 4. Fall back to round-robin if health data unavailable
    async fn select_peer_round_robin(&self) -> Result<PeerId> {
        let available_peers = self.available_peers.read().await;
        if available_peers.is_empty() {
            return Err(eyre!("No peers available in pool"));
        }

        let peer_stats = self.peers.read().await;

        // Calculate health scores for each peer
        let mut peer_scores: Vec<(PeerId, f64)> = Vec::new();

        for &peer in available_peers.iter() {
            if let Some(peer_conn) = peer_stats.get(&peer) {
                // Skip unhealthy peers unless all peers are unhealthy
                if !peer_conn.stats.is_healthy && peer_scores.len() > 0 {
                    continue;
                }

                let error_rate = peer_conn.error_rate();
                let response_time = peer_conn.stats.avg_response_time;

                // Calculate health score (higher is better)
                // Score components:
                // 1. Error rate penalty: (1.0 - error_rate) gives 1.0 for 0% errors, 0.0 for 100% errors
                // 2. Response time penalty: 1.0 / (1.0 + response_time) gives higher scores for faster peers
                //    - 0ms response = 1.0
                //    - 1s response = 0.5
                //    - 5s response = 0.17
                // Weight: 70% error rate, 30% response time
                let error_score = (1.0 - error_rate).max(0.0);
                let response_score = if response_time > 0.0 {
                    1.0 / (1.0 + response_time)
                } else {
                    1.0  // No data yet, give benefit of doubt
                };

                let health_score = 0.7 * error_score + 0.3 * response_score;
                peer_scores.push((peer, health_score));
            } else {
                // No stats yet, give new peer a decent score
                peer_scores.push((peer, 0.8));
            }
        }

        // If all peers are unhealthy, allow them back in
        if peer_scores.is_empty() {
            for &peer in available_peers.iter() {
                if let Some(peer_conn) = peer_stats.get(&peer) {
                    let error_rate = peer_conn.error_rate();
                    let response_time = peer_conn.stats.avg_response_time;
                    let error_score = (1.0 - error_rate).max(0.0);
                    let response_score = if response_time > 0.0 {
                        1.0 / (1.0 + response_time)
                    } else {
                        1.0
                    };
                    let health_score = 0.7 * error_score + 0.3 * response_score;
                    peer_scores.push((peer, health_score));
                } else {
                    peer_scores.push((peer, 0.5));  // Lower score for unhealthy unknowns
                }
            }
        }

        // Sort by score (highest first)
        peer_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Use weighted random selection from top 50% of peers
        // This balances between always using the best peer (which might overload it)
        // and distributing load across healthy peers
        let top_half = (peer_scores.len() / 2).max(1);
        let candidates = &peer_scores[..top_half];

        // Simple round-robin among top candidates
        let mut counter = self.round_robin_counter.write().await;
        let index = *counter % candidates.len();
        *counter = (*counter + 1) % candidates.len();

        let selected_peer = candidates[index].0;
        debug!("Selected peer {} with health score {:.3} (rank {}/{})",
               selected_peer, candidates[index].1, index + 1, candidates.len());

        Ok(selected_peer)
    }

    /// Acquire stream with automatic peer selection and failover
    ///
    /// This method implements request-level peer failover:
    /// 1. Selects a healthy peer using weighted round-robin
    /// 2. Attempts to acquire a stream from that peer
    /// 3. On failure, retries with a different peer (up to max_retries times)
    /// 4. Tracks which peers have been tried to avoid retry loops
    pub async fn acquire_stream_auto(&self) -> Result<(Stream, Option<SemaphorePermit<'static>>)> {
        let max_retries = self.config.max_retries as usize;
        let mut tried_peers = std::collections::HashSet::new();
        let mut last_error = None;

        for attempt in 0..=max_retries {
            // Select a peer we haven't tried yet
            let peer = match self.select_peer_round_robin().await {
                Ok(p) if !tried_peers.contains(&p) => p,
                Ok(p) => {
                    // Already tried this peer, try to find another one
                    debug!("Peer {} already tried, selecting another peer", p);
                    // Continue to next iteration to get a different peer
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    continue;
                }
                Err(e) => {
                    last_error = Some(e);
                    break;
                }
            };

            tried_peers.insert(peer);

            match self.acquire_stream(peer).await {
                Ok((stream, permit)) => {
                    if attempt > 0 {
                        debug!("Successfully acquired stream from peer {} after {} retries", peer, attempt);
                        counter!("p2proxy_failover_success_total").increment(1);
                    }
                    return Ok((stream, permit));
                }
                Err(e) => {
                    warn!("Failed to acquire stream from peer {} (attempt {}/{}): {}",
                          peer, attempt + 1, max_retries + 1, e);
                    counter!("p2proxy_failover_attempt_total").increment(1);
                    last_error = Some(e);

                    // Don't retry if we've exhausted all attempts
                    if attempt >= max_retries {
                        break;
                    }

                    // Small delay before retry to avoid hammering
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                }
            }
        }

        counter!("p2proxy_failover_exhausted_total").increment(1);
        Err(last_error.unwrap_or_else(|| eyre!("No peers available for failover")))
    }

    /// Open a stream to the given peer with rate limiting and timeout
    ///
    /// Returns a tuple of (Stream, Option<SemaphorePermit>) where the permit
    /// MUST be held by the caller for the entire session duration. The permit
    /// will be automatically released when dropped (i.e., when the session ends).
    #[instrument(skip(self), fields(peer = %peer))]
    pub async fn acquire_stream(&self, peer: PeerId) -> Result<(Stream, Option<SemaphorePermit<'static>>)> {
        if !self.config.enabled {
            // Management disabled, open stream directly
            let mut control = self.control.clone();
            let stream = control.open_stream(peer, TCP_PROXY_PROTOCOL)
                .await
                .map_err(|e| eyre!("Failed to open stream: {}", e))?;
            return Ok((stream, None));
        }

        let start = Instant::now();

        // Get or create peer connection tracker
        let semaphore = {
            let mut peers = self.peers.write().await;
            let peer_conn = peers
                .entry(peer)
                .or_insert_with(|| PeerConnection::new(peer, self.config.max_concurrent_per_peer));
            peer_conn.semaphore.clone()
        };

        // Acquire semaphore permit to limit concurrent streams (shorter timeout for rate limiting)
        // This permit MUST be returned to the caller and held for the entire session duration
        let permit = match tokio::time::timeout(
            self.config.semaphore_timeout,
            semaphore.acquire(),
        )
        .await
        {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                self.record_failure(peer).await;
                counter!("p2proxy_stream_semaphore_acquire_errors_total").increment(1);
                return Err(eyre!("Semaphore acquisition failed: {}", e));
            }
            Err(_) => {
                counter!("p2proxy_stream_semaphore_timeout_total").increment(1);
                self.record_failure(peer).await;
                return Err(eyre!(
                    "Timeout waiting for stream slot (too many concurrent connections to peer {})",
                    peer
                ));
            }
        };

        // Increment counter with RAII guard
        let mut guard = {
            let mut peers = self.peers.write().await;
            let peer_conn = peers
                .entry(peer)
                .or_insert_with(|| PeerConnection::new(peer, self.config.max_concurrent_per_peer));
            peer_conn.stats.current_active += 1;
            gauge!("p2proxy_stream_pool_active_total", "peer" => peer.to_string())
                .set(peer_conn.stats.current_active as f64);

            StreamGuard::new(peer, self.peers.clone())
        };

        // Open the stream - guard will auto-decrement if this fails/panics
        let mut control = self.control.clone();
        let stream = tokio::time::timeout(
            self.config.stream_open_timeout,
            control.open_stream(peer, TCP_PROXY_PROTOCOL),
        )
        .await
        .map_err(|_| {
            self.record_failure_sync(peer);
            counter!("p2proxy_stream_open_timeout_total").increment(1);
            eyre!("Timeout opening stream to peer {} (network timeout)", peer)
        })?
        .map_err(|e| {
            self.record_failure_sync(peer);
            counter!("p2proxy_stream_open_errors_total").increment(1);
            eyre!("Failed to open stream to peer {}: {}", peer, e)
        })?;

        // Mark guard as successful (prevents auto-decrement on drop)
        guard.mark_success();

        // Record success with duration for response time tracking
        let duration = start.elapsed();
        self.record_success_with_duration(peer, Some(duration)).await;

        histogram!("p2proxy_stream_acquire_duration_seconds").record(duration.as_secs_f64());
        counter!("p2proxy_stream_opened_total").increment(1);
        counter!("p2proxy_stream_pool_acquire_total").increment(1);
        debug!("Opened stream to peer {} in {:?}", peer, duration);

        // Convert permit to 'static lifetime
        // SAFETY: This is safe because:
        // 1. The semaphore (Arc<Semaphore>) lives for the entire program lifetime
        // 2. The permit is being returned to the caller who MUST hold it for the session
        // 3. When the permit is dropped, it will properly release the semaphore slot
        let static_permit: SemaphorePermit<'static> = unsafe {
            std::mem::transmute(permit)
        };

        Ok((stream, Some(static_permit)))
    }

    /// Record successful stream opening
    async fn record_success(&self, peer: PeerId) {
        self.record_success_with_duration(peer, None).await;
    }

    /// Record successful stream opening with response time
    async fn record_success_with_duration(&self, peer: PeerId, duration: Option<Duration>) {
        let mut peers = self.peers.write().await;
        if let Some(peer_conn) = peers.get_mut(&peer) {
            peer_conn.stats.total_opened += 1;
            peer_conn.stats.recent_successes += 1;
            peer_conn.reset_recent_stats();

            // Update exponential moving average of response time
            if let Some(d) = duration {
                let duration_secs = d.as_secs_f64();
                const ALPHA: f64 = 0.2; // Weight for new samples (0-1, higher = more reactive)
                if peer_conn.stats.response_time_samples == 0 {
                    peer_conn.stats.avg_response_time = duration_secs;
                } else {
                    peer_conn.stats.avg_response_time =
                        ALPHA * duration_secs + (1.0 - ALPHA) * peer_conn.stats.avg_response_time;
                }
                peer_conn.stats.response_time_samples += 1;
                gauge!("p2proxy_peer_avg_response_time", "peer" => peer.to_string())
                    .set(peer_conn.stats.avg_response_time);
            }

            // Update health status based on error rate
            let error_rate = peer_conn.error_rate();
            if error_rate < self.config.max_error_rate {
                peer_conn.stats.is_healthy = true;
            }

            counter!("p2proxy_stream_opened_success_total", "peer" => peer.to_string()).increment(1);
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
                    counter!("p2proxy_peer_failover_total", "peer" => peer.to_string()).increment(1);
                }

                gauge!("p2proxy_stream_pool_active_total", "peer" => peer.to_string())
                    .set(peer_conn.stats.current_active as f64);
                gauge!("p2proxy_peer_error_rate", "peer" => peer.to_string()).set(error_rate);
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
        peers
            .get(peer)
            .map(|conn| conn.error_rate())
            .unwrap_or(0.0)
    }
}
