//! Latency benchmarks for P2Proxy
//!
//! This benchmark suite measures:
//! - Connection establishment latency
//! - SOCKS5 handshake latency
//! - Small message round-trip time (100 bytes)
//!
//! These benchmarks use the criterion framework with async tokio support
//! and mock components configured with realistic latencies.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use libp2p::PeerId;
use std::time::Duration;

// Include the test modules directly in the benchmark
// This is necessary because benches can't easily import from tests/ directory
#[path = "../tests/common/mod.rs"]
mod common;

use common::mock_peer::{MockPeer, MockPeerConfig};
use common::mock_swarm::{MockSwarm, MockSwarmConfig};

/// Benchmark connection establishment latency
///
/// Measures the time to establish a P2P connection with various latency configurations.
fn bench_connection_establishment(c: &mut Criterion) {
    let mut group = c.benchmark_group("connection_establishment");

    // Test with different latency configurations
    for latency_ms in [10, 50, 100].iter() {
        let config = MockSwarmConfig {
            latency: Duration::from_millis(*latency_ms),
            success_rate: 1.0,
            seed: Some(42),
            ..Default::default()
        };

        group.bench_with_input(
            BenchmarkId::new("latency_ms", latency_ms),
            latency_ms,
            |b, _| {
                b.to_async(tokio::runtime::Runtime::new().unwrap()).iter(|| async {
                    let mut swarm = MockSwarm::new(config.clone());
                    let peer_id = PeerId::random();

                    // Measure connection establishment
                    let _ = swarm.connect_to_peer(peer_id).await;
                });
            },
        );
    }

    group.finish();
}

/// Benchmark SOCKS5 handshake latency
///
/// Measures the complete SOCKS5 handshake process including:
/// 1. Client greeting and method selection
/// 2. Connection request and response
fn bench_socks5_handshake(c: &mut Criterion) {
    let mut group = c.benchmark_group("socks5_handshake");

    // Test with realistic latency configurations
    for latency_ms in [10, 25, 50].iter() {
        let config = MockPeerConfig {
            latency: Duration::from_millis(*latency_ms),
            failure_rate: 0.0,
            seed: Some(100),
            jitter: Duration::from_millis(2),
            ..Default::default()
        };

        group.bench_with_input(
            BenchmarkId::new("latency_ms", latency_ms),
            latency_ms,
            |b, _| {
                b.to_async(tokio::runtime::Runtime::new().unwrap()).iter(|| async {
                    let mut peer = MockPeer::new(config.clone());

                    // Simulate SOCKS5 handshake (2 round-trips)
                    let _ = peer.respond_to_query(b"socks5_greeting").await;
                    let _ = peer.respond_to_query(b"socks5_connect").await;
                });
            },
        );
    }

    group.finish();
}

/// Benchmark small message round-trip time (100 bytes)
///
/// Measures the latency for sending and receiving small messages,
/// which is typical for control plane operations.
fn bench_small_message_rtt(c: &mut Criterion) {
    let mut group = c.benchmark_group("small_message_rtt");

    // Small message payload (100 bytes)
    let message = vec![0u8; 100];

    // Test with different latency scenarios
    for latency_ms in [10, 30, 50].iter() {
        let config = MockPeerConfig {
            latency: Duration::from_millis(*latency_ms),
            failure_rate: 0.0,
            seed: Some(200),
            jitter: Duration::from_millis(1),
            ..Default::default()
        };

        group.bench_with_input(
            BenchmarkId::new("latency_ms", latency_ms),
            latency_ms,
            |b, _| {
                b.to_async(tokio::runtime::Runtime::new().unwrap()).iter(|| async {
                    let mut peer = MockPeer::new(config.clone());

                    // Send message and wait for response
                    let _ = peer.respond_to_query(&message).await;
                });
            },
        );
    }

    group.finish();
}

/// Benchmark comparison of direct vs relay connections
///
/// Compares the latency difference between direct P2P connections
/// and relay-mediated connections.
fn bench_direct_vs_relay(c: &mut Criterion) {
    let mut group = c.benchmark_group("direct_vs_relay");

    // Direct connection (no relay)
    let direct_config = MockSwarmConfig {
        latency: Duration::from_millis(50),
        success_rate: 1.0,
        seed: Some(300),
        use_relay: false,
        ..Default::default()
    };

    group.bench_function("direct_connection", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap()).iter(|| async {
            let mut swarm = MockSwarm::new(direct_config.clone());
            let peer_id = PeerId::random();
            let _ = swarm.connect_to_peer(peer_id).await;
        });
    });

    // Relay connection (adds relay overhead)
    let relay_config = MockSwarmConfig {
        latency: Duration::from_millis(50),
        success_rate: 1.0,
        seed: Some(301),
        use_relay: true,
        ..Default::default()
    };

    group.bench_function("relay_connection", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap()).iter(|| async {
            let mut swarm = MockSwarm::new(relay_config.clone());
            let peer_id = PeerId::random();
            let _ = swarm.connect_to_peer(peer_id).await;
        });
    });

    group.finish();
}

/// Benchmark peer query operations
///
/// Measures latency for various peer query types including ping,
/// peer info, and node discovery queries.
fn bench_peer_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("peer_queries");

    let config = MockPeerConfig {
        latency: Duration::from_millis(30),
        failure_rate: 0.0,
        seed: Some(400),
        jitter: Duration::from_millis(3),
        ..Default::default()
    };

    // Benchmark ping query
    group.bench_function("ping", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap()).iter(|| async {
            let mut peer = MockPeer::new(config.clone());
            let _ = peer.respond_to_query(b"ping").await;
        });
    });

    // Benchmark peer info query
    group.bench_function("peer_info", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap()).iter(|| async {
            let mut peer = MockPeer::new(config.clone());
            let _ = peer.respond_to_query(b"peer_info").await;
        });
    });

    // Benchmark find nodes query
    group.bench_function("find_nodes", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap()).iter(|| async {
            let mut peer = MockPeer::new(config.clone());
            let _ = peer.respond_to_query(b"find_nodes").await;
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_connection_establishment,
    bench_socks5_handshake,
    bench_small_message_rtt,
    bench_direct_vs_relay,
    bench_peer_queries
);
criterion_main!(benches);
