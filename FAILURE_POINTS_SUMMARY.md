# P2Proxy Connection Failure Points - Quick Reference

## Location Map

### P2P Networking (`crates/p2proxy/src/swarm.rs`)
- **Lines 228-304**: Bootstrap connection with retry logic
- **Lines 382-467**: Peer discovery and connection flow
- **Lines 469-581**: Query protocol implementation
- **Lines 584-597**: Bootstrap reconnection mechanism
- **Lines 599-624**: Main event loop

### Stream Pool (`crates/p2proxy/src/stream_pool.rs`)
- **Lines 31-42**: Default configuration
- **Lines 140-210**: Stream acquisition with dual timeouts
- **Lines 232-260**: Peer failure recording and health check
- **Lines 329-336**: Health status queries

### SOCKS5 Proxy (`crates/p2proxy/src/proxy_protocols/socks_stream.rs`)
- **Lines 107-170**: Proxy server initialization
- **Lines 172-181**: Connection handler setup
- **Lines 188-228**: SOCKS5 handshake protocol
- **Lines 262-279**: Stream pool acquisition
- **Lines 323-443**: Data transfer loop with select!
- **Lines 446-450**: Cleanup and resource release

### RPC Communication (`crates/p2proxy/src/main.rs`)
- **Lines 122-160**: TCP server with critical unwraps
- **Lines 85-107**: Main task coordination

---

## Timeout Configuration

### Currently Configured Timeouts

```
QUIC Handshake:        120 seconds (swarm.rs:190)
Bootstrap Identify:    10 seconds (swarm.rs:283)
Peer Connection:       10 seconds (swarm.rs:451)
Peer Discovery Query:  5 seconds (swarm.rs:559)
Stream Semaphore:      20 seconds (stream_pool.rs:167)
Stream Open:           20 seconds (stream_pool.rs:187)
Bootstrap Retry:       5 seconds (swarm.rs:604)
```

### Configured in Config.yaml

```yaml
pool:
  open_timeout_secs: 20      # Stream open (doubled from 10)
  health_check_timeout_secs: 5
  max_error_rate: 0.15       # 15% failover threshold
  max_retries: 3             # Not currently used in code
```

---

## Connection Failure Scenarios

### Scenario 1: Bootstrap Connection Failure
**Time to Failure**: 20+ seconds

```
1. Dial bootstrap (fail) → sleep 2s
2. Retry up to 10 times
3. Total: 10 retries × 2s = 20+ seconds before bail
```

**Issues**:
- Linear backoff (not exponential)
- No jitter (could cause thundering herd)

### Scenario 2: Peer Discovery Failure
**Time to Failure**: 20+ seconds

```
1. Query relay (5s timeout)
2. Dial discovered peers
3. Wait for ConnectionEstablished (10s timeout)
4. Retry up to 20 times with 1s sleep
5. Total: 20 retries × 1s = 20+ seconds
```

**Issues**:
- Query timeout very short (5s)
- Linear backoff, not exponential
- Waits for ANY peer to connect

### Scenario 3: Stream Acquisition Timeout
**Time to Failure**: 40+ seconds

```
1. Wait for semaphore slot (20s timeout)
2. Open stream to peer (20s timeout)
3. Total: 40+ seconds possible
```

**Issues**:
- Two sequential timeouts
- No indication which phase failed
- Client blocks entire time

### Scenario 4: Peer Health Degradation
**Time to Failover**: Variable (15% error rate threshold)

```
1. Peer starts failing (varies)
2. Error rate tracked in sliding 100-attempt window
3. Once 15+ failures in window → failover triggered
4. Minimum: 15 failures + time for evaluation
```

**Issues**:
- Sliding window delayed detection
- 15% is relatively high threshold
- No active health checks

---

## Critical Code Sections

### Section 1: Unwraps in RPC Server (PANIC RISK)

**File**: `main.rs:134, 154`

```rust
let (socket, addr) = listener.accept().await.unwrap();  // Line 134
remoc::Connect::io(...).await.unwrap();                  // Line 154
```

**Impact**: Any accept/connect error crashes RPC server
**Severity**: HIGH - System crash

---

### Section 2: Bootstrap State Machine (RACE CONDITION RISK)

**File**: `swarm.rs:137-142, 584-597, 749-783`

```rust
bootstrap_connected: bool,      // Manually tracked
bootstrap_dialing: bool,        // Manually tracked

// Updated by event handler and try_dial_bootstrap()
// Single-threaded but fragile state management
```

**Impact**: Possible stuck states (always dialing, never connected)
**Severity**: MEDIUM - Can hang indefinitely

---

### Section 3: Stream Pool Active Count (LEAK RISK)

**File**: `stream_pool.rs:159, 232-260`

```rust
peer_conn.stats.current_active += 1;  // Incremented
// Later decremented in record_failure() or stream_closed()
// If panic occurs between, count stays elevated
```

**Impact**: Gradual exhaustion of 30-stream limit
**Severity**: MEDIUM - Degrades performance over time

---

### Section 4: Session Cleanup Silent Failures

**File**: `socks_stream.rs:446-450, 489-490`

```rust
let _ = socket_write.flush().await;
let _ = proxy_session.close().await;
let _ = socket_write.shutdown().await;
stream_pool.stream_closed(peer).await;  // If this fails?
```

**Impact**: Resources might not be fully released
**Severity**: LOW - Cleanup is best-effort

---

## Error Metrics Emitted

P2Proxy emits comprehensive metrics for monitoring:

### Connection Metrics
```
p2proxy_peer_connections_total        # Total successful connections
p2proxy_peers_connected               # Current connected peer gauge
p2proxy_peer_failover_total           # Failover events
p2proxy_peer_identified_total         # Identify protocol completions
```

### Stream Pool Metrics
```
p2proxy_stream_acquire_timeout_total  # Semaphore/stream open timeouts
p2proxy_stream_opened_total           # Successful stream opens
p2proxy_stream_opened_success_total   # Per-peer success count
p2proxy_stream_opened_failed_total    # Per-peer failure count
p2proxy_stream_pool_active_total      # Current active streams per peer
p2proxy_peer_error_rate               # Current error rate per peer
```

### SOCKS5 Metrics
```
p2proxy_socks_handshake_errors_total         # Protocol errors
p2proxy_socks_connections_total              # Client connections
p2proxy_socks_connections_active             # Current active SOCKS connections
p2proxy_socket_read_errors_total             # Client read failures
p2proxy_socket_write_errors_total            # Client write failures
p2proxy_peer_read_errors_total               # Peer read failures
p2proxy_data_send_errors_total               # Data transmission errors
```

### Session Metrics
```
p2proxy_sessions_initialized_total    # Created sessions
p2proxy_sessions_completed_total      # Completed sessions
p2proxy_session_errors_total          # Failed sessions
p2proxy_bandwidth_reports_sent_total  # Reports sent to relay
```

---

## Recommended Testing

### High Priority
1. **Bootstrap Failure Resilience**: Simulate bootstrap unavailability for 30+ seconds
2. **RPC Server Stability**: Send malformed connections to TCP server
3. **Stream Pool Exhaustion**: Open 30+ concurrent streams, measure response time

### Medium Priority
1. **Peer Health Detection**: Introduce 16% error rate, verify failover
2. **Bootstrap Reconnection**: Kill bootstrap, verify recovery timing
3. **Lock Contention**: Many concurrent SOCKS connections, measure latency

### Low Priority
1. **Resource Leak Detection**: Run for hours, monitor memory growth
2. **Race Condition Detection**: High load with frequent disconnects
3. **Panic Recovery**: Inject panics in spawned tasks, verify graceful shutdown

---

## Configuration Recommendations

### Current Config (Config.yaml)
```yaml
port: 45445
log_level: debug

servers:
  - protocol: Socks5
    port: 1080
    min_bandwidth: 50Mbps
    pool:
      enabled: true
      max_total: 30
      open_timeout_secs: 20
      max_retries: 3
      health_check_timeout_secs: 5
      max_error_rate: 0.15
```

### Recommended Tuning

**For Slow Networks** (high latency):
```yaml
open_timeout_secs: 30         # Increase from 20
max_error_rate: 0.20          # Increase to 20%
```

**For Unreliable Networks** (frequent failures):
```yaml
open_timeout_secs: 15         # Decrease for faster failover
max_error_rate: 0.10          # Lower threshold, faster recovery
```

**For Performance** (low-latency local):
```yaml
open_timeout_secs: 10         # Tight timeout
max_total: 50                 # More concurrent streams
```

---

## Files to Monitor for Changes

1. **crates/p2proxy/src/swarm.rs** - P2P connection logic
2. **crates/p2proxy/src/stream_pool.rs** - Connection pooling
3. **crates/p2proxy/src/proxy_protocols/socks_stream.rs** - SOCKS5 protocol
4. **crates/p2proxy/src/main.rs** - RPC server and task coordination

---

## Related Documentation

- Full analysis: `CONNECTION_FAILURE_ANALYSIS.md` (971 lines)
- Test suite: `crates/p2proxy/tests/README.md`
- Configuration: `Config.yaml`
- Model definitions: `crates/models/src/config.rs`

