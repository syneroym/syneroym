#![allow(clippy::unwrap_used, clippy::panic)]
//! M03-sss (Slice 5) performance budgets: `put-blob`/`get-blob` against the
//! `object_store` local filesystem backend, unencrypted (encryption-at-rest
//! adds AEAD segment overhead on top -- see the SQLCipher A/B bench in
//! `crates/data-layer/benches/security_config_bench.rs` for the analogous
//! encrypted-vs-plaintext comparison on the data-layer side; blob content
//! encryption has no separate budget row in `task.md`).

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use syneroym_blob_store::{BlobProvider, ObjectStoreBlobProvider};
use tokio::runtime::Builder;

const ONE_MB: usize = 1024 * 1024;

/// Benchmark: `put-blob` (1 MB, `object_store` local backend).
fn bench_put_blob(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let payload = vec![0xab_u8; ONE_MB];

    c.bench_function("put_blob_1mb_local", |b| {
        b.to_async(&runtime).iter_custom(|iters| {
            let payload = payload.clone();
            async move {
                let mut total = std::time::Duration::ZERO;
                for _ in 0..iters {
                    let temp_dir = tempfile::tempdir().unwrap();
                    let provider = ObjectStoreBlobProvider::new_local(
                        temp_dir.path().to_path_buf(),
                        u64::MAX,
                        None,
                    )
                    .unwrap();

                    let start = std::time::Instant::now();
                    provider.put_blob("bench-svc", black_box(payload.clone()), None).await.unwrap();
                    total += start.elapsed();
                }
                total
            }
        });
    });
}

/// Benchmark: `get-blob` (1 MB, local cache hit -- i.e. the blob is already
/// on disk from a prior `put-blob`, and the OS page cache is warm from the
/// setup `put_blob` call each iteration).
fn bench_get_blob(c: &mut Criterion) {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let payload = vec![0xcd_u8; ONE_MB];
    let temp_dir = tempfile::tempdir().unwrap();
    let provider =
        ObjectStoreBlobProvider::new_local(temp_dir.path().to_path_buf(), u64::MAX, None).unwrap();
    let hash = runtime.block_on(provider.put_blob("bench-svc", payload, None)).unwrap();

    c.bench_function("get_blob_1mb_local_warm", |b| {
        b.to_async(&runtime).iter(|| async {
            let _ = provider.get_blob("bench-svc", black_box(&hash), None).await.unwrap();
        });
    });
}

criterion_group!(benches, bench_put_blob, bench_get_blob);
criterion_main!(benches);
