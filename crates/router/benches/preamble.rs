#![allow(clippy::unwrap_used, clippy::panic)]
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use syneroym_router::RoutePreamble;

fn bench_preamble_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("preamble_parsing");

    let canon = "json-rpc://health|substrate-123";
    group.bench_function("binary_json_rpc", |b| {
        b.iter(|| {
            let _ = RoutePreamble::parse(black_box(canon)).unwrap();
        });
    });

    let http_preamble = "http://health|substrate-123";
    group.bench_function("http_json_rpc", |b| {
        b.iter(|| {
            let _ = RoutePreamble::parse(black_box(http_preamble)).unwrap();
        });
    });

    let composable = "http-wrpc://health|substrate-123";
    group.bench_function("composable", |b| {
        b.iter(|| {
            let _ = RoutePreamble::parse(black_box(composable)).unwrap();
        });
    });

    let encrypted = "raw://health|substrate-123?enc=ecdh-p256&pubkey=62734fde81";
    group.bench_function("encrypted_query_params", |b| {
        b.iter(|| {
            let _ = RoutePreamble::parse(black_box(encrypted)).unwrap();
        });
    });

    group.finish();
}

criterion_group!(benches, bench_preamble_parsing);
criterion_main!(benches);
