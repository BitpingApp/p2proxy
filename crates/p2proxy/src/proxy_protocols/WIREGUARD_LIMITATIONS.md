# WireGuard Implementation - Current Status and Limitations

## ⚠️ Important Notice

The current WireGuard implementation is a **FOUNDATION LAYER** that provides session-managed UDP packet forwarding through libp2p streams. It is **NOT** a complete WireGuard VPN implementation and should not be used as such without understanding its limitations.

## What This Implementation Provides

### ✅ Currently Implemented

1. **Session Management**
   - Persistent sessions per client address
   - Session affinity ensuring packets from the same client use the same libp2p stream
   - Automatic session timeout and cleanup (3-minute inactivity)
   - Stream pool integration with proper resource cleanup

2. **UDP Packet Forwarding**
   - Receives UDP packets on configured port (default: 51820)
   - Forwards packets through libp2p streams to destination peer
   - Receives responses from peer and sends back to original client
   - Non-blocking I/O to prevent throughput degradation

3. **WireGuard Protocol Recognition**
   - Identifies WireGuard message types:
     * Handshake Initiation (type 1)
     * Handshake Response (type 2)
     * Cookie Reply (type 3)
     * Data (type 4)
   - Logs message types for debugging

4. **Metrics and Monitoring**
   - Prometheus metrics for packets, bandwidth, sessions
   - Session lifecycle tracking
   - Error rate monitoring
   - Packet size histograms

5. **Event System**
   - Session initialization events
   - Data transfer events
   - Error events with context
   - Session finished events

## ❌ NOT Implemented (Critical Limitations)

### 1. **WireGuard Cryptography**
The implementation does NOT perform any WireGuard cryptographic operations:
- ❌ No Noise protocol handshake
- ❌ No ChaCha20-Poly1305 encryption/decryption
- ❌ No BLAKE2s hashing
- ❌ No Curve25519 key exchange
- ❌ No nonce management

**Impact**: Packets are forwarded as-is. If both ends of the libp2p stream don't have proper WireGuard endpoints, connections will fail.

### 2. **Key Management**
- ❌ No WireGuard keypair generation
- ❌ No peer public key management
- ❌ No pre-shared key support
- ❌ No key rotation

**Impact**: Cannot establish WireGuard connections independently. Requires external WireGuard configuration.

### 3. **TUN/TAP Interface**
- ❌ No virtual network interface creation
- ❌ No IP packet encapsulation/decapsulation
- ❌ No routing table manipulation
- ❌ No firewall integration

**Impact**: Cannot function as a standalone VPN. Requires actual WireGuard endpoints on both sides of the libp2p connection.

### 4. **WireGuard State Machine**
- ❌ No handshake state tracking
- ❌ No session key derivation
- ❌ No replay protection
- ❌ No rekey logic

**Impact**: Cannot maintain proper WireGuard security guarantees.

### 5. **Rate Limiting and DoS Protection**
- ❌ No per-client rate limiting
- ❌ No cookie-based DoS protection (WireGuard feature)
- ❌ No connection throttling

**Impact**: Vulnerable to UDP flood attacks.

## Current Use Cases

### ✅ Suitable For

1. **Tunneling Pre-Configured WireGuard**
   - Both client and server have WireGuard configured locally
   - This implementation tunnels WireGuard UDP packets through libp2p
   - Acts as a transport layer for WireGuard, not WireGuard itself

2. **Development and Testing**
   - Testing libp2p stream management
   - Prototyping UDP packet forwarding
   - Measuring performance characteristics

3. **Foundation for Full Implementation**
   - Session management is in place
   - Metrics infrastructure ready
   - Integration with stream pool working

### ❌ NOT Suitable For

1. **Direct VPN Usage**
   - Will not establish VPN connections on its own
   - Cannot replace WireGuard client/server

2. **Security-Critical Applications**
   - Missing cryptographic implementation
   - No authentication beyond libp2p
   - No rate limiting

3. **Production Deployments**
   - Incomplete implementation
   - Missing features for reliability
   - Needs extensive testing and hardening

## Architecture

```
┌─────────────┐                  ┌──────────────────┐                  ┌─────────────┐
│   WireGuard │                  │   This Impl      │                  │   WireGuard │
│   Client    │──UDP packets──>│  Session Mgr     │──libp2p stream──>│   Server    │
│  (external) │                  │  + Forwarding    │                  │  (external) │
└─────────────┘                  └──────────────────┘                  └─────────────┘
                                        ↓
                                  Metrics, Events
                                  Stream Pool Mgmt
```

## Performance Characteristics

### Strengths
- Session affinity maintains WireGuard state across packets
- Non-blocking I/O prevents blocking on slow responses
- Efficient buffer management (MTU-sized: 1500 bytes)
- Stream pooling reduces connection overhead

### Bottlenecks
- Polling-based receive loop has 1ms granularity
- Write lock contention on session map under high load
- No packet batching (processes packets individually)

### Optimization Opportunities
- Use `tokio::select!` for event-driven I/O instead of polling
- Implement lock-free session lookup for read path
- Add packet batching for bulk transfers
- Consider `io_uring` on Linux for zero-copy UDP

## Path to Full WireGuard Support

To make this a complete WireGuard implementation, the following components must be added:

### Phase 1: Basic Crypto (High Priority)
1. Integrate `boringtun` or `wireguard-rs` for crypto operations
2. Implement key management (generation, storage, exchange)
3. Add handshake state machine
4. Implement session key derivation

### Phase 2: Interface Support (High Priority)
1. Add TUN/TAP device creation and management
2. Implement IP packet encapsulation/decapsulation
3. Add routing table integration
4. Handle MTU and fragmentation

### Phase 3: Security (High Priority)
1. Add cookie-based DoS protection
2. Implement replay protection
3. Add per-client rate limiting
4. Implement proper authentication

### Phase 4: Reliability (Medium Priority)
1. Add keepalive mechanism
2. Implement rekey logic
3. Add connection recovery
4. Handle network roaming

### Phase 5: Performance (Low Priority)
1. Optimize hot paths
2. Add packet batching
3. Implement zero-copy where possible
4. Profile and optimize allocations

## Known Issues

1. **Polling-based receive** - The send task polls all sessions in a loop with 1ms sleep, which may not scale well
2. **Session map lock contention** - All session operations acquire write lock; consider RwLock or lock-free structures
3. **No connection limits** - Unlimited sessions can be created; add max sessions per IP
4. **Missing ICMP feedback** - Client never receives error responses for failed packets
5. **No MTU discovery** - Assumes 1500-byte MTU; should handle path MTU discovery

## Contributing

If you're interested in implementing full WireGuard support, please:

1. Review the existing `boringtun` and `wireguard-rs` crates
2. Create a design document for the crypto integration
3. Open an issue to discuss the approach
4. Submit PRs incrementally, starting with Phase 1

## References

- [WireGuard Protocol](https://www.wireguard.com/protocol/)
- [WireGuard Whitepaper](https://www.wireguard.com/papers/wireguard.pdf)
- [boringtun - WireGuard Rust Implementation](https://github.com/cloudflare/boringtun)
- [Noise Protocol Framework](https://noiseprotocol.org/)

## License

Same as parent project.
