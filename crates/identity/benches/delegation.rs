#![allow(clippy::unwrap_used)]

use std::time::Duration;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use syneroym_identity::{DelegationCertificate, Identity};

fn bench_delegation(c: &mut Criterion) {
    let master = Identity::generate().unwrap();
    let temp = Identity::generate().unwrap();
    let temp_pubkey = temp.public_key();

    let cert =
        DelegationCertificate::issue(&master, temp_pubkey, 3600, "routing".to_string()).unwrap();

    c.bench_function("DelegationCertificate::issue", |b| {
        b.iter(|| {
            let _ = DelegationCertificate::issue(
                black_box(&master),
                black_box(temp_pubkey),
                black_box(3600),
                black_box("routing".to_string()),
            );
        })
    });

    c.bench_function("DelegationCertificate::verify", |b| {
        b.iter(|| {
            let _ = cert.verify(black_box(&cert.master_did));
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().measurement_time(Duration::from_secs(3));
    targets = bench_delegation
}
criterion_main!(benches);
