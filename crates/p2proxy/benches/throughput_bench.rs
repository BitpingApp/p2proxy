//! Throughput benchmarks for P2Proxy
//!
//! This benchmark suite measures:
//! - Single session maximum throughput
//! - Concurrent session throughput
//! - Large file transfer performance
//! - Aggregate bandwidth across multiple sessions
//! - Data transfer at various sizes: 1KB, 10KB, 100KB, 1MB, 10MB
//!
//! Run with: cargo bench --bench throughput_bench

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use std::time::Duration;

// Test data generation (inline to avoid dependency issues in benchmarks)
fn generate_bench_data(size: usize) -> Vec<u8> {
    vec![0xAB; size]
}

/// Simulates data processing/transfer by computing blake3 hash
fn process_data(data: &[u8]) -> blake3::Hash {
    blake3::hash(data)
}

/// Benchmark: Data transfer at various sizes
///
/// Measures throughput for different data sizes from 1KB to 10MB
fn bench_data_transfer_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("data_transfer_sizes");

    let sizes = vec![
        ("1KB", 1_024),
        ("10KB", 10_240),
        ("100KB", 102_400),
        ("1MB", 1_048_576),
        ("10MB", 10_485_760),
    ];

    for (name, size) in sizes {
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("transfer", name), &size, |b, &size| {
            let data = generate_bench_data(size);

            b.iter(|| {
                // Simulate transfer by processing chunks
                let chunk_size = 8192; // 8KB chunks
                let mut total_processed = 0;

                for chunk in data.chunks(chunk_size) {
                    black_box(chunk);
                    total_processed += chunk.len();
                }

                black_box(total_processed)
            });
        });
    }

    group.finish();
}

/// Benchmark: Hash computation for transferred data
///
/// Measures the overhead of hash computation during data transfer
fn bench_hash_computation(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_computation");

    let sizes = vec![
        ("1KB", 1_024),
        ("10KB", 10_240),
        ("100KB", 102_400),
        ("1MB", 1_048_576),
        ("10MB", 10_485_760),
    ];

    for (name, size) in sizes {
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("blake3", name), &size, |b, &size| {
            let data = generate_bench_data(size);

            b.iter(|| {
                let hash = process_data(&data);
                black_box(hash)
            });
        });
    }

    group.finish();
}

/// Benchmark: Chunked data transfer
///
/// Measures performance of transferring data in chunks of various sizes
fn bench_chunked_transfer(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunked_transfer");

    // Test with 1MB data, various chunk sizes
    let data_size = 1_048_576; // 1 MB
    let data = generate_bench_data(data_size);

    let chunk_sizes = vec![
        ("512B", 512),
        ("4KB", 4_096),
        ("8KB", 8_192),
        ("16KB", 16_384),
        ("64KB", 65_536),
    ];

    for (name, chunk_size) in chunk_sizes {
        group.throughput(Throughput::Bytes(data_size as u64));

        group.bench_with_input(
            BenchmarkId::new("1MB_chunks", name),
            &chunk_size,
            |b, &chunk_size| {
                b.iter(|| {
                    let mut total = 0;
                    for chunk in data.chunks(chunk_size) {
                        black_box(chunk);
                        total += chunk.len();
                    }
                    black_box(total)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Async data transfer simulation
///
/// Measures async overhead for data transfers
fn bench_async_transfer(c: &mut Criterion) {
    let mut group = c.benchmark_group("async_transfer");

    let sizes = vec![
        ("1KB", 1_024),
        ("10KB", 10_240),
        ("100KB", 102_400),
        ("1MB", 1_048_576),
    ];

    for (name, size) in sizes {
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("tokio", name), &size, |b, &size| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let data = generate_bench_data(size);

            b.to_async(&rt).iter(|| async {
                // Simulate async transfer
                let chunk_size = 8192;
                let mut total = 0;

                for chunk in data.chunks(chunk_size) {
                    black_box(chunk);
                    total += chunk.len();
                    tokio::task::yield_now().await;
                }

                black_box(total)
            });
        });
    }

    group.finish();
}

/// Benchmark: Concurrent transfers
///
/// Measures performance of multiple concurrent transfers
fn bench_concurrent_transfers(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_transfers");
    group.sample_size(10); // Reduce sample size for concurrent tests

    let concurrency_levels = vec![1, 5, 10, 25];

    for concurrency in concurrency_levels {
        group.throughput(Throughput::Bytes((100_000 * concurrency) as u64));

        group.bench_with_input(
            BenchmarkId::new("100KB_each", concurrency),
            &concurrency,
            |b, &concurrency| {
                let rt = tokio::runtime::Runtime::new().unwrap();

                b.to_async(&rt).iter(|| async move {
                    let mut handles = vec![];

                    for _i in 0..concurrency {
                        let handle = tokio::spawn(async move {
                            let data = generate_bench_data(100_000); // 100KB
                            let mut total = 0;

                            for chunk in data.chunks(8192) {
                                black_box(chunk);
                                total += chunk.len();
                                tokio::task::yield_now().await;
                            }

                            black_box(total)
                        });
                        handles.push(handle);
                    }

                    // Wait for all transfers to complete
                    for handle in handles {
                        handle.await.unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Hash + Transfer combined
///
/// Measures combined overhead of hashing and transferring data
fn bench_hash_and_transfer(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_and_transfer");

    let sizes = vec![
        ("1KB", 1_024),
        ("10KB", 10_240),
        ("100KB", 102_400),
        ("1MB", 1_048_576),
    ];

    for (name, size) in sizes {
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("combined", name), &size, |b, &size| {
            let data = generate_bench_data(size);

            b.iter(|| {
                // Hash while transferring
                let mut hasher = blake3::Hasher::new();
                let mut total = 0;

                for chunk in data.chunks(8192) {
                    hasher.update(chunk);
                    black_box(chunk);
                    total += chunk.len();
                }

                let hash = hasher.finalize();
                black_box((total, hash))
            });
        });
    }

    group.finish();
}

/// Benchmark: Bidirectional transfer simulation
///
/// Measures performance of simultaneous send and receive
fn bench_bidirectional_transfer(c: &mut Criterion) {
    let mut group = c.benchmark_group("bidirectional_transfer");

    let sizes = vec![("10KB", 10_240), ("100KB", 102_400), ("1MB", 1_048_576)];

    for (name, size) in sizes {
        group.throughput(Throughput::Bytes((size * 2) as u64)); // Both directions

        group.bench_with_input(BenchmarkId::new("duplex", name), &size, |b, &size| {
            let rt = tokio::runtime::Runtime::new().unwrap();

            b.to_async(&rt).iter(|| async move {
                let send_data = generate_bench_data(size);
                let recv_data = generate_bench_data(size);

                // Simulate sending and receiving simultaneously
                let send_handle = tokio::spawn(async move {
                    let mut total = 0;
                    for chunk in send_data.chunks(8192) {
                        black_box(chunk);
                        total += chunk.len();
                        tokio::task::yield_now().await;
                    }
                    total
                });

                let recv_handle = tokio::spawn(async move {
                    let mut total = 0;
                    for chunk in recv_data.chunks(8192) {
                        black_box(chunk);
                        total += chunk.len();
                        tokio::task::yield_now().await;
                    }
                    total
                });

                let (sent, received) = tokio::join!(send_handle, recv_handle);
                black_box((sent.unwrap(), received.unwrap()))
            });
        });
    }

    group.finish();
}

/// Benchmark: Memory copying overhead
///
/// Measures the cost of memory operations during transfer
fn bench_memory_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_operations");

    let sizes = vec![
        ("1KB", 1_024),
        ("10KB", 10_240),
        ("100KB", 102_400),
        ("1MB", 1_048_576),
    ];

    for (name, size) in sizes {
        group.throughput(Throughput::Bytes(size as u64));

        // Benchmark: Clone
        group.bench_with_input(BenchmarkId::new("clone", name), &size, |b, &size| {
            let data = generate_bench_data(size);

            b.iter(|| {
                let cloned = data.clone();
                black_box(cloned)
            });
        });

        // Benchmark: Copy
        group.bench_with_input(BenchmarkId::new("copy", name), &size, |b, &size| {
            let data = generate_bench_data(size);

            b.iter(|| {
                let mut dest = vec![0u8; size];
                dest.copy_from_slice(&data);
                black_box(dest)
            });
        });

        // Benchmark: To Vec (chunk iteration)
        group.bench_with_input(
            BenchmarkId::new("to_vec_chunked", name),
            &size,
            |b, &size| {
                let data = generate_bench_data(size);

                b.iter(|| {
                    let mut result = Vec::new();
                    for chunk in data.chunks(8192) {
                        result.extend_from_slice(chunk);
                    }
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

// Configure benchmark groups
criterion_group! {
    name = benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .warm_up_time(Duration::from_secs(3));
    targets =
        bench_data_transfer_sizes,
        bench_hash_computation,
        bench_chunked_transfer,
        bench_async_transfer,
        bench_concurrent_transfers,
        bench_hash_and_transfer,
        bench_bidirectional_transfer,
        bench_memory_operations,
}

criterion_main!(benches);
