# P2Proxy Connection Failure Analysis - Deep Dive Report

## Executive Summary

P2Proxy is a peer-to-peer proxy system with three critical connection layers:
1. **P2P Networking** (libp2p with relay fallback)
2. **SOCKS5 Proxy Protocol** (TCP-based client connections)
3. **RPC Communication** (Daemon-to-UI interaction over TCP)

This analysis identifies failure points, timeout mechanisms, error handling patterns, and potential race conditions across all layers.

---

## 1. P2P NETWORKING LAYER (`swarm.rs`)

### 1.1 Critical Timeout Definitions

**File**: `/home/user/p2proxy/crates/p2proxy/src/swarm.rs`

| Timeout | Value | Purpose | Location |
|---------|-------|---------|----------|
| QUIC Handshake | 120 seconds | QUIC protocol negotiation | Line 190 |
| QUIC Max Idle | u32::MAX | Prevent idle connection termination | Line 189 |
| Swarm Idle Connection | u64::MAX | Prevent swarm from closing connections | Line 197 |
| Bootstrap Connection | 10 seconds | Identify event after bootstrap dial | Line 283 |
| Peer Connection | 10 seconds | ConnectionEstablished event | Line 451 |
| Peer Discovery Query | 5 seconds | Query response from relay | Line 559 |
| Bootstrap Retry Timer | 5 seconds | Periodic bootstrap reconnection check | Line 604 |

### 1.2 Bootstrap Connection Failure Points

**Bootstrap Initialization** (Lines 228-304)

```rust
let mut bootstrap_retry_count = 0;
const MAX_BOOTSTRAP_RETRIES: usize = 10;

let bootstrap_peer_id = loop {
    match swarm.dial(bootstrap.clone()) {
        Ok(_) => {
            info!("Attempting to connect to bootstrap server (attempt {}/{})",
                bootstrap_retry_count + 1, MAX_BOOTSTRAP_RETRIES);
        }
        Err(e) => {
            warn!(?e, "Failed to dial bootstrap server");
            if bootstrap_retry_count >= MAX_BOOTSTRAP_RETRIES {
                bail!("Failed to dial bootstrap server after {} attempts", MAX_BOOTSTRAP_RETRIES);
            }
            bootstrap_retry_count += 1;
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
    }
    
    // Wait for identify event with timeout
    match swarm.wait_for_with_timeout(
        |swarm, event| {
            if let SwarmEvent::Behaviour(BehaviourEvent::Identify(
                identify::Event::Received { peer_id, info, .. }
            )) = event {
                // Extract relay address
                Some(*peer_id)
            } else {
                None
            }
        },
        Duration::from_secs(10),  // 10-second timeout
    ).await {
        Ok(peer_id) => {
            info!("Successfully connected to bootstrap server");
            break peer_id;
        }
        Err(_) => {
            warn!("Bootstrap connection timeout");
            if bootstrap_retry_count >= MAX_BOOTSTRAP_RETRIES {
                bail!("Failed to connect to bootstrap server after {} attempts");
            }
            bootstrap_retry_count += 1;
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
    }
};
```

**Failure Modes**:
1. **Dial Failure**: DNS resolution, network unreachability, peer not accepting connections
2. **Identify Timeout**: Bootstrap peer not responding to identify protocol (10s timeout)
3. **Max Retries Exceeded**: 10 attempts with 2-second backoff = 20 seconds minimum failure
4. **No Backoff Increase**: Retry count increments but sleep is fixed at 2 seconds (linear, not exponential)

**Risk**: Bootstrap connection is mandatory and uses linear backoff (not exponential). System will bail after 20 seconds without connecting to bootstrap.

### 1.3 Peer Discovery and Connection Failure

**Peer Discovery Flow** (Lines 382-467)

```rust
async fn discover_and_connect_to_peer(
    &mut self,
    server: &Server,
) -> Result<PeerId> {
    let mut retry_count = 0;
    const MAX_RETRIES: usize = 20;

    while retry_count < MAX_RETRIES {
        info!("Looking up peer (attempt {}/{})", retry_count + 1, MAX_RETRIES);
        
        // Step 1: Discover peers via query
        let destination_addresses = match self.discover_peer(server).await {
            Ok(addresses) => {
                if addresses.is_empty() {
                    warn!("No peer addresses discovered");
                    retry_count += 1;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                addresses
            }
            Err(e) => {
                warn!(?e, "Failed to discover peer");
                retry_count += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        
        // Step 2: Dial all discovered peers
        for addr in destination_addresses {
            match self.0.swarm.dial(addr.clone()) {
                Ok(_) => info!(?addr, "Dialing peer"),
                Err(e) => warn!(?e, ?addr, "Failed to dial peer"),
            }
        }
        
        // Step 3: Wait for any ConnectionEstablished event
        match self.0.swarm.wait_for_with_timeout(
            |_, event| {
                if let SwarmEvent::ConnectionEstablished {
                    peer_id,
                    connection_id,
                    endpoint,
                    num_established,
                    concurrent_dial_errors,
                    established_in,
                } = event {
                    info!(?peer_id, ?connection_id, ?endpoint, 
                          ?num_established, ?concurrent_dial_errors, 
                          ?established_in, "Connected to peer");
                    return Some(*peer_id);
                }
                None
            },
            Duration::from_secs(10),  // 10-second timeout
        ).await {
            Ok(peer_id) => return Ok(peer_id),
            Err(_) => {
                warn!("Connection timeout reached");
                retry_count += 1;
            }
        }
    }

    bail!("Failed to connect with any peer after {} attempts", MAX_RETRIES);
}
```

**Failure Modes**:
1. **Peer Discovery Failure**: Query returns no addresses (network issue, no peers matching criteria)
2. **Query Timeout**: 5-second timeout on query response (Line 559)
3. **Dial Failures**: All discovered addresses fail to dial
4. **Connection Timeout**: No ConnectionEstablished event within 10 seconds
5. **Max Retries**: 20 attempts × 1 second sleep = 20+ second total failure

**CRITICAL**: If all peers are offline or unreachable, system waits 20+ seconds before failing.

### 1.4 Peer Discovery Query (`discover_peer`)

**Query Operation** (Lines 469-581)

```rust
async fn discover_peer(&mut self, server: &Server) -> Result<HashSet<Multiaddr>> {
    // If destination peer is specified, use it directly
    if let Some(destination_peer) = &server.peer_options.destination_peer {
        // Direct return, no query
        return Ok(destination_address);
    }
    
    // Otherwise, query relay for matching peers
    let request = Auth::new(
        QueryRequest::FindNodes {
            requirements: Some(node_reqs),
            exclusions: Some(node_excs),
            capabilities: None,
            limit: 25,
        },
        &KEYPAIR,
        self.0.token.clone(),
    )?;

    let outbound_request_id = self.0.swarm.behaviour_mut()
        .query.send_request(&self.0.relay_peer_id, request);

    let peer_ids = self.0.swarm
        .wait_for_with_timeout(
            move |swarm, event| match event {
                SwarmEvent::Behaviour(BehaviourEvent::Query(
                    request_response::Event::Message {
                        peer,
                        connection_id,
                        message: Message::Response {
                            request_id,
                            response,
                        },
                    },
                )) if *request_id == outbound_request_id => match response {
                    bitping_swarm::query::QueryResponse::Error(e) => {
                        Some(Err(eyre!(e.clone())))
                    }
                    bitping_swarm::query::QueryResponse::FindNodes(hash_set) => {
                        Some(Ok(hash_set.clone()))
                    }
                    _ => Some(Err(eyre!("Got wrong query response")))
                },
                _ => None,
            },
            Duration::from_secs(5),  // 5-second timeout
        ).await??;
    
    Ok(peer_ids)
}
```

**Failure Modes**:
1. **Query Request Failure**: Auth::new fails (invalid token)
2. **No Response**: Relay doesn't respond within 5 seconds
3. **Error Response**: Relay returns error (no peers match criteria)
4. **Wrong Response Type**: Query returns unexpected message type

**Risk**: Query timeout is only 5 seconds. Slow relay responses will timeout quickly.

### 1.5 Bootstrap Reconnection Management

**Main Event Loop** (Lines 599-624)

```rust
pub async fn drive_network(mut self, server_state: Arc<RwLock<ServerContainer>>) -> Result<()> {
    // Initial bootstrap dial check
    self.try_dial_bootstrap();

    // Bootstrap reconnection timer
    let mut bootstrap_retry_timer = 
        tokio::time::interval(Duration::from_secs(5));
    bootstrap_retry_timer.set_missed_tick_behavior(
        tokio::time::MissedTickBehavior::Skip);

    // Main event loop
    loop {
        tokio::select! {
            Some(message) = self.0.proxy_message_channel.1.recv() => {
                if let Err(e) = self.handle_proxy_events(message, server_state.clone()).await {
                    warn!(?e, "Something went wrong handling proxy events");
                }
            },
            Some(event) = self.0.swarm.next() => {
                self.handle_swarm_events_with_bootstrap(event, server_state.clone());
            }
            _ = bootstrap_retry_timer.tick() => {
                // Periodically check if we need to reconnect to bootstrap
                self.try_dial_bootstrap();
            }
        };
    }
}
```

**Bootstrap Reconnection** (Lines 584-597)

```rust
fn try_dial_bootstrap(&mut self) {
    if !self.0.bootstrap_connected && !self.0.bootstrap_dialing {
        info!("Attempting to dial bootstrap server");
        match self.0.swarm.dial(self.0.bootstrap_address.clone()) {
            Ok(_) => {
                self.0.bootstrap_dialing = true;
                debug!("Bootstrap dial initiated");
            }
            Err(e) => {
                warn!(?e, "Failed to dial bootstrap server");
            }
        }
    }
}
```

**Failure Modes**:
1. **Bootstrap Disconnection**: Detected in event handler (Lines 764-769)
2. **Connection Lost Recovery**: Periodically retries every 5 seconds
3. **Race Condition**: `bootstrap_dialing` flag prevents duplicate dials, but no timeout if dial never completes
4. **No Backoff**: Retry timer is fixed at 5 seconds

**Risk**: If bootstrap server responds to dial but never sends identify, system will retry forever (5 second intervals).

---

## 2. STREAM POOL & CONNECTION MANAGEMENT (`stream_pool.rs`)

### 2.1 Stream Pool Configuration

**Default Config** (Lines 31-42)

```rust
impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_concurrent_per_peer: 30,           // 30 concurrent streams
            stream_open_timeout: Duration::from_secs(20),    // 20-second timeout
            enabled: true,
            max_retries: 3,                        // Configured but NOT USED
            health_check_timeout: Duration::from_secs(5),
            max_error_rate: 0.15,                  // 15% error rate threshold
        }
    }
}
```

**Configurable via Config.yaml**:

```yaml
pool:
  enabled: true
  open_timeout_secs: 20    # Stream open timeout
  max_retries: 3           # Not currently used in code
  health_check_timeout_secs: 5
  max_error_rate: 0.15     # Failover trigger threshold
```

### 2.2 Stream Acquisition with Rate Limiting

**Critical Function** (Lines 140-210)

```rust
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
            .or_insert_with(|| PeerConnection::new(peer, 
                self.config.max_concurrent_per_peer));
        peer_conn.stats.current_active += 1;
        gauge!("p2proxy_stream_pool_active_total", "peer" => peer.to_string())
            .set(peer_conn.stats.current_active as f64);
        peer_conn.semaphore.clone()
    };

    // Phase 1: Wait for semaphore permit (max_concurrent_per_peer slot)
    let _permit = match tokio::time::timeout(
        self.config.stream_open_timeout,  // 20-second timeout
        semaphore.acquire(),
    ).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            self.record_failure(peer).await;
            return Err(eyre!("Semaphore acquisition failed: {}", e));
        }
        Err(_) => {  // TIMEOUT waiting for semaphore
            counter!("p2proxy_stream_acquire_timeout_total").increment(1);
            self.record_failure(peer).await;
            return Err(eyre!("Timeout waiting for stream slot"));
        }
    };

    // Phase 2: Open the stream
    let mut control = self.control.clone();
    let stream = tokio::time::timeout(
        self.config.stream_open_timeout,  // Another 20-second timeout
        control.open_stream(peer, TCP_PROXY_PROTOCOL),
    ).await
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
```

**Failure Modes**:

| Stage | Timeout | Failure Mode |
|-------|---------|--------------|
| Semaphore Acquire | 20s | Too many concurrent streams (30+ limit) |
| Stream Open | 20s | Peer unresponsive, no libp2p-stream protocol |
| Stream Open | 20s | Network partition, libp2p dial fails |

**Risk**: Two sequential 20-second timeouts = 40 seconds total if both fail.

### 2.3 Peer Health Management

**Health Tracking** (Lines 90-110)

```rust
fn error_rate(&self) -> f64 {
    let total = self.stats.recent_successes + self.stats.recent_failures;
    if total == 0 {
        return 0.0;
    }
    self.stats.recent_failures as f64 / total as f64
}

fn reset_recent_stats(&mut self) {
    const MAX_WINDOW_SIZE: u64 = 100;
    let total = self.stats.recent_successes + self.stats.recent_failures;
    if total > MAX_WINDOW_SIZE {
        // Keep sliding window
        self.stats.recent_successes = self.stats.recent_successes / 2;
        self.stats.recent_failures = self.stats.recent_failures / 2;
    }
}
```

**Health Status Update on Failure** (Lines 232-260)

```rust
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
        if error_rate >= self.config.max_error_rate {  // 15% threshold
            peer_conn.stats.is_healthy = false;
            counter!("p2proxy_peer_failover_total", "peer" => peer.to_string())
                .increment(1);
            debug!(
                "Peer {} marked unhealthy due to high error rate: {:.2}%",
                peer,
                error_rate * 100.0
            );
        }

        gauge!("p2proxy_stream_pool_active_total", "peer" => peer.to_string())
            .set(peer_conn.stats.current_active as f64);
        gauge!("p2proxy_peer_error_rate", "peer" => peer.to_string())
            .set(error_rate);
        counter!("p2proxy_stream_opened_failed_total", "peer" => peer.to_string())
            .increment(1);
    }
}
```

**Failure Modes**:
1. **Sliding Window Weakness**: Errors only trigger failover after 15% of recent 100 attempts fail
2. **Recovery Assumed on Success**: Any successful stream opens recovery (is_healthy = true)
3. **No Health Check Timeout Used**: `health_check_timeout` is defined but never invoked
4. **No Exponential Backoff**: Failed peers are retried immediately if healthy again

---

## 3. SOCKS5 PROXY LAYER

### 3.1 SOCKS5 Stream-Based Implementation (`socks_stream.rs`)

**Connection Handler** (Lines 172-181)

```rust
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
```

**Failure Points in SOCKS5 Handshake** (Lines 188-228)

| Stage | Timeout | Failure Mode | Metric |
|-------|---------|--------------|--------|
| Handshake Read | None | Client doesn't send handshake | `p2proxy_socks_handshake_errors_total` |
| Auth Method Eval | None | Client requests unsupported auth | `p2proxy_socks_auth_method_errors_total` |
| Handshake Response Write | None | Client disconnects during response | `p2proxy_socks_handshake_response_errors_total` |
| Connection Request Read | None | Client doesn't send request | `p2proxy_socks_request_errors_total` |
| Connection Response Write | None | Client disconnects during response | `p2proxy_socks_response_errors_total` |

**Stream Pool Acquisition** (Lines 262-279)

```rust
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
```

**Failure Modes**:
1. **Pool Timeout**: Stream pool timeout (20s semaphore + 20s stream open) prevents connection
2. **Max Concurrent Streams**: If 30+ concurrent streams per peer, client waits 20+ seconds
3. **Unhealthy Peer**: Peer health status not checked before attempting stream

### 3.2 Data Transfer Phase

**Data Loop** (Lines 323-443)

```rust
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
                outgoing_hasher.update(bytes_slice);
                
                match proxy_session.send_data(bytes_slice.to_vec()).await {
                    Ok(_) => {
                        let bytes_len = bytes_slice.len();
                        outgoing_bytes += bytes_len;
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
                            incoming_hasher.update(&transfer.bytes);
                            let bytes_len = transfer.bytes.len();
                            
                            if let Err(e) = socket_write.write_all(&transfer.bytes).await {
                                counter!("p2proxy_socket_write_errors_total").increment(1);
                                break;
                            }
                            if let Err(e) = socket_write.flush().await {
                                counter!("p2proxy_socket_flush_errors_total").increment(1);
                                break;
                            }
                            incoming_bytes += bytes_len;
                        }
                    }
                    DataPhaseMessage::Error(err) => {
                        counter!("p2proxy_peer_data_errors_total").increment(1);
                        break;
                    },
                    DataPhaseMessage::Close(id) => {
                        if id == session_id.to_string() {
                            counter!("p2proxy_peer_closed_total").increment(1);
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
```

**Failure Modes** (all cause immediate connection close):
1. **Client Disconnect**: Socket returns 0 bytes (handled gracefully)
2. **Client Read Error**: Socket error during read (abrupt close)
3. **Peer Send Failure**: Failed to send data to peer stream
4. **Peer Read Error**: Lost connection to peer
5. **Peer Error Message**: Peer sent error response
6. **Socket Write Error**: Failed to write response to client
7. **Socket Flush Error**: Failed to flush buffered write

**Risk**: Any single I/O error terminates the session. No retry logic for transient failures.

### 3.3 Session Cleanup

**Cleanup Phase** (Lines 446-450)

```rust
// Clean up
// Make sure to flush the buffered writer before closing
let _ = socket_write.flush().await;
let _ = proxy_session.close().await;
let _ = socket_write.shutdown().await;
```

**Failure Modes**:
1. **Silent Errors**: All cleanup errors are ignored with `let _ =`
2. **Resource Leak Risk**: If shutdown fails, resources might not be fully released
3. **Stream Pool Notification** (Line 490):
   ```rust
   stream_pool.stream_closed(peer).await;
   ```
   If this fails, peer's active_count isn't decremented, blocking future connections.

---

## 4. RPC COMMUNICATION LAYER (`main.rs`)

### 4.1 TCP Server and Connection Handling

**RPC Server** (Lines 122-160)

```rust
const TCP_PORT: u16 = 9876;
async fn start_server(server_state: Arc<RwLock<ServerContainer>>) -> Result<()> {
    use remoc::ConnectExt;
    
    println!("Listening on port {}. Press Ctrl+C to exit.", TCP_PORT);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, TCP_PORT)).await?;

    loop {
        // Accept an incoming TCP connection.
        let (socket, addr) = listener.accept().await.unwrap();  // UNWRAP!
        let (socket_rx, socket_tx) = socket.into_split();
        println!("Accepted connection from {}", addr);
        let counter_obj = server_state.clone();

        // Spawn a task for each incoming connection.
        tokio::spawn(async move {
            let (server, client) =
                CounterServerSharedMut::<_, remoc::codec::Postcard>::new(counter_obj, 1);

            remoc::Connect::io(remoc::Cfg::default(), socket_rx, socket_tx)
                .provide(client)
                .await
                .unwrap();  // UNWRAP!

            tracing::info!("Serving database connection {}", addr);
            server.serve(true).await
        });
    }
}
```

**CRITICAL Issues**:
1. **Line 134**: `listener.accept().await.unwrap()` - WILL PANIC on accept error
2. **Line 152**: `.await.unwrap()` - WILL PANIC on remoc connection error
3. **No Timeout**: Accept loop has no timeout, can hang indefinitely
4. **No Error Recovery**: Single panic crashes entire RPC server

### 4.2 Main Task Coordination

**Main Event Loop** (Lines 85-107)

```rust
let mut join_set = JoinSet::new();

let mut proxy_future = ProxyNetwork::with_authentication()
    .await?
    .with_swarm(tx)
    .await?;

for server in CONFIG.servers.iter() {
    proxy_future.configure_server(server).await?;
}

let server_state = Arc::new(RwLock::new(ServerContainer::new(CONFIG.servers.clone())));
let _ = join_set.spawn(proxy_future.drive_network(server_state.clone()));
let _ = join_set.spawn(start_server(server_state.clone()));
let _ = join_set.spawn(handle_swarm_events(rx, server_state.clone()));

while let Some(result) = join_set.join_next().await {
    result??;  // Will propagate first task failure
}
```

**Failure Modes**:
1. **Any Task Panic**: If RPC server panics (unwrap), JoinSet detects panic and returns error
2. **Early Termination**: If any task finishes first, loop continues until all tasks complete
3. **No Restart Logic**: Failed tasks are not restarted

---

## 5. ERROR HANDLING PATTERNS

### 5.1 Unwrap/Expect Calls (Panic Points)

| File | Line | Expression | Context |
|------|------|-----------|---------|
| main.rs | 25 | `.expect("Failed to resolve GRPC Channel")` | GRPC channel creation (startup only) |
| main.rs | 44 | `.expect("Cannot initialise config")` | Config loading (startup only) |
| main.rs | 49-50 | `.unwrap()` | Panic hook cleanup (should never fail) |
| main.rs | 134 | `.unwrap()` on accept | **CRITICAL** - RPC server loop |
| main.rs | 154 | `.unwrap()` on remoc connect | **CRITICAL** - RPC server connection |
| swarm.rs | 157 | `.unwrap()` on try_into_ed25519 | Auth keypair conversion (assumes Ed25519) |
| swarm.rs | 486 | `.unwrap()` on Protocol::P2p | Destination peer protocol extraction |
| swarm.rs | 492 | `.unwrap()` on with_p2p | Multiaddr construction |
| socks.rs | 176 | `.unwrap()` on Auth::new | Session auth creation |
| socks.rs | 178 | `.unwrap()` on postcard encode | Session encoding |

**Risk Assessment**:
- **HIGH**: Main.rs unwraps in accept/remoc loops (can crash server)
- **MEDIUM**: Swarm.rs unwraps during connection (assumes valid keypair/protocol)
- **LOW**: Socks.rs unwraps (fallback available in socks_stream.rs)

### 5.2 Silent Error Handling

| Pattern | Location | Risk |
|---------|----------|------|
| `let _ = sender.send(...)` | Throughout | Dropped error messages if channel full |
| `let _ = socket.shutdown()` | socks_stream.rs:450 | Cleanup failures silently ignored |
| `let _ = proxy_session.close()` | socks_stream.rs:449 | Stream close failures ignored |
| `let _ = response.write_to_async_stream()` | socks.rs:129 | Auth response failure ignored |

### 5.3 Error Types and Propagation

Primary error type: `color_eyre::eyre::Result<T>` (uses eyre error handling)

```rust
pub fn get_grpc_channel(grpc_hub_url: String, grpc_hub_domain: String) -> Result<Channel> {
    // Returns Result with context wrapping
    let channel_config = if grpc_hub_url.starts_with("https://") {
        let tls = ClientTlsConfig::new()
            .with_enabled_roots()
            .domain_name(grpc_hub_domain);
        Channel::builder(grpc_hub_url.try_into()?)
            .tls_config(tls)
            .context("Error configuring TLS for GRPC")?  // Contextual error
    } else {
        Channel::builder(grpc_hub_url.try_into()?)
    };

    Ok(channel_config.connect_lazy())
}
```

---

## 6. CONNECTION STATE MANAGEMENT

### 6.1 Race Condition Scenarios

**Bootstrap State Flags** (Lines 137-142)

```rust
bootstrap_address: Multiaddr,
bootstrap_peer_id: Option<PeerId>,
bootstrap_connected: bool,      // Manually tracked
bootstrap_dialing: bool,        // Manually tracked
```

**Potential Race**:
```
Thread A: Detects ConnectionClosed, sets bootstrap_connected=false
Thread B: In try_dial_bootstrap, checks !bootstrap_connected && !bootstrap_dialing
Thread C: Sets bootstrap_dialing=true, initiates dial
Thread D: Dial completes immediately, sets bootstrap_dialing=false but bootstrap_connected=false
Result: Never sets bootstrap_connected=true because event handler hasn't run yet
```

**Actual Safety**: Safe because:
- Single tokio runtime thread (no true multithreading)
- Events processed sequentially in main loop
- But state is fragile and hard to reason about

### 6.2 Shared State via Arc<RwLock>

**ServerContainer Access** (Throughout swarm.rs)

```rust
pub async fn drive_network(mut self, server_state: Arc<RwLock<ServerContainer>>) -> Result<()> {
    // Spawned tasks write to server_state
    let event = Events::Connection(...);
    tokio::spawn(async move {
        server_state.write().await.handle_event(event).await;
    });
}
```

**Contention Risk**:
- Multiple tasks write to ServerContainer concurrently
- RPC clients read via `.read().await`
- High lock contention under heavy load
- No timeout on lock acquisition (can deadlock if holder panics)

### 6.3 Channel Sender/Receiver Patterns

**Event Channel** (Line 83)

```rust
let (tx, rx) = tokio::sync::mpsc::channel(100);
```

**Failure Mode**: If event handler task crashes, rx task hangs forever (channel never closes).

---

## 7. RESOURCE CLEANUP ISSUES

### 7.1 Stream Pool Active Count

**Potential Leak** (stream_pool.rs:159)

```rust
let semaphore = {
    let mut peers = self.peers.write().await;
    let peer_conn = peers.entry(peer)
        .or_insert_with(|| PeerConnection::new(peer, 
            self.config.max_concurrent_per_peer));
    peer_conn.stats.current_active += 1;  // Increment
    peer_conn.semaphore.clone()
};
```

**If Acquisition Fails Later**:
- current_active was incremented
- record_failure is called, which decrements
- But if stream_closed is never called (panic), count stays elevated
- Eventually all 30 slots get stuck

### 7.2 Orphaned Tasks

**tokio::spawn without Error Handling** (main.rs:140)

```rust
tokio::spawn(async move {
    // remoc connection handling
    // No error handling, panic will be logged but ignored
});
```

**Risk**: Crashes in spawned tasks are silently logged, main task continues.

### 7.3 Socket Resource Management

**socks_stream.rs:312-316**

```rust
let (mut socket_read, socket_write) = socket.split();
let mut socket_write = BufWriter::with_capacity(SOCKET_BUF_SIZE, socket_write);
let mut socket_buf = vec![0u8; SOCKET_BUF_SIZE];
```

**Cleanup** (Lines 447-450):

```rust
let _ = socket_write.flush().await;
let _ = proxy_session.close().await;
let _ = socket_write.shutdown().await;
```

**Risk**: If session holds reference to stream and panics, cleanup never happens. Sockets can leak.

---

## 8. SUMMARY: CRITICAL FAILURE POINTS

### Tier 1 (System Crash)
1. **RPC Server Accept Loop** (main.rs:134) - `.unwrap()` can panic
2. **RPC Server Connection Setup** (main.rs:154) - `.unwrap()` can panic
3. **Keypair Conversion** (swarm.rs:157) - Assumes Ed25519, will panic otherwise

### Tier 2 (Service Unavailability - 20+ seconds)
1. **Bootstrap Connection Retry** - Linear backoff, 10 max retries = 20+ seconds
2. **Peer Discovery & Connection Retry** - 20 retries × 1 second = 20+ seconds
3. **Stream Pool Timeout** - 20 seconds semaphore + 20 seconds open = 40 seconds total

### Tier 3 (Graceful Degradation)
1. **Peer Health Failover** - 15% error rate threshold takes effect
2. **Connection Cleanup** - Silent failure on shutdown (resources may leak)
3. **Session Error Handling** - Immediate termination on any I/O error

### Tier 4 (Observable Risks)
1. **No Exponential Backoff** - Bootstrap and peer discovery use linear backoff
2. **Race Conditions** - Bootstrap state flags are fragile
3. **No Lock Timeouts** - RwLock acquisition can deadlock
4. **Orphaned Tasks** - Panic in spawned tasks not handled

---

## 9. RECOMMENDATIONS

### High Priority
1. Remove unwraps from RPC server loop (main.rs:134, 154)
2. Implement exponential backoff for bootstrap connection
3. Add timeout on RwLock operations to prevent deadlock

### Medium Priority
1. Add health check endpoint instead of relying on error rate
2. Implement task panic recovery in JoinSet
3. Add timeout to accept() loop for graceful shutdown

### Low Priority
1. Use logging for silent error handling instead of `let _ =`
2. Add memory pool for session buffers to prevent allocation stalls
3. Document race conditions in bootstrap state management

