#![allow(clippy::unwrap_used, clippy::panic)]
use std::io::Cursor;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use syneroym_rpc::framing::{read_frame, write_frame};
use tokio::runtime::Builder;

fn bench_rpc_framing(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();

    let small_payload = vec![b'A'; 100];
    let large_payload = vec![b'A'; 10 * 1024];

    // Pre-calculate buffers for read_frame benchmark
    let mut small_read_buf = Vec::new();
    runtime.block_on(write_frame(&mut small_read_buf, &small_payload)).unwrap();

    let mut large_read_buf = Vec::new();
    runtime.block_on(write_frame(&mut large_read_buf, &large_payload)).unwrap();

    let mut group = c.benchmark_group("rpc_framing");

    // Small frame benchmarks
    group.bench_function("write_frame_100b", |b| {
        b.to_async(&runtime).iter(|| async {
            let mut out = Vec::with_capacity(128);
            write_frame(&mut out, black_box(&small_payload)).await.unwrap();
        });
    });

    group.bench_function("read_frame_100b", |b| {
        b.to_async(&runtime).iter(|| async {
            let mut cursor = Cursor::new(black_box(&small_read_buf));
            let _ = read_frame(&mut cursor).await.unwrap();
        });
    });

    // Large frame benchmarks
    group.bench_function("write_frame_10kb", |b| {
        b.to_async(&runtime).iter(|| async {
            let mut out = Vec::with_capacity(11 * 1024);
            write_frame(&mut out, black_box(&large_payload)).await.unwrap();
        });
    });

    group.bench_function("read_frame_10kb", |b| {
        b.to_async(&runtime).iter(|| async {
            let mut cursor = Cursor::new(black_box(&large_read_buf));
            let _ = read_frame(&mut cursor).await.unwrap();
        });
    });

    group.finish();
}

criterion_group!(benches, bench_rpc_framing);
criterion_main!(benches);
