#![allow(clippy::unwrap_used, clippy::panic)]
use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use p256::{EncodedPoint, PublicKey, ecdh::EphemeralSecret};
use syneroym_identity::Identity;

fn bench_ecdh_handshake(c: &mut Criterion) {
    let identity = Identity::generate().unwrap();

    // Simulate pre-computed client public key
    let client_secret = EphemeralSecret::random(&mut aes_gcm::aead::OsRng);
    let client_pub = EncodedPoint::from(client_secret.public_key());
    let client_pub_bytes = client_pub.as_bytes().to_vec();

    c.bench_function("ecdh_p256_server_handshake", |b| {
        b.iter(|| {
            // Server side handshake actions
            let public_key = PublicKey::from_sec1_bytes(black_box(&client_pub_bytes)).unwrap();
            let secret = EphemeralSecret::random(&mut aes_gcm::aead::OsRng);
            let server_pub_key = EncodedPoint::from(secret.public_key());

            let shared = secret.diffie_hellman(&public_key);
            let shared_bytes = shared.raw_secret_bytes();

            let mut key = [0u8; 32];
            key.copy_from_slice(&shared_bytes[..32]);
            let _cipher = Aes256Gcm::new(aes_gcm::Key::<Aes256Gcm>::from_slice(&key));

            // Server payload signing
            let mut payload = Vec::with_capacity(130);
            payload.extend_from_slice(server_pub_key.as_bytes());
            payload.extend_from_slice(&client_pub_bytes);

            let _signature = identity.sign(&payload);
        });
    });
}

fn bench_aes_gcm(c: &mut Criterion) {
    let key = [0u8; 32];
    let cipher = Aes256Gcm::new(aes_gcm::Key::<Aes256Gcm>::from_slice(&key));
    let nonce = aes_gcm::Nonce::from_slice(&[0u8; 12]);

    let data_1kb = vec![0u8; 1024];
    let ciphertext_1kb = cipher.encrypt(nonce, data_1kb.as_slice()).unwrap();

    let mut group_1kb = c.benchmark_group("aes_gcm_1kb");
    group_1kb.bench_function("encrypt", |b| {
        b.iter(|| {
            let _ = cipher.encrypt(black_box(nonce), black_box(data_1kb.as_slice())).unwrap();
        });
    });
    group_1kb.bench_function("decrypt", |b| {
        b.iter(|| {
            let _ = cipher.decrypt(black_box(nonce), black_box(ciphertext_1kb.as_slice())).unwrap();
        });
    });
    group_1kb.finish();

    let data_64kb = vec![0u8; 64 * 1024];
    let ciphertext_64kb = cipher.encrypt(nonce, data_64kb.as_slice()).unwrap();

    let mut group_64kb = c.benchmark_group("aes_gcm_64kb");
    group_64kb.bench_function("encrypt", |b| {
        b.iter(|| {
            let _ = cipher.encrypt(black_box(nonce), black_box(data_64kb.as_slice())).unwrap();
        });
    });
    group_64kb.bench_function("decrypt", |b| {
        b.iter(|| {
            let _ =
                cipher.decrypt(black_box(nonce), black_box(ciphertext_64kb.as_slice())).unwrap();
        });
    });
    group_64kb.finish();
}

criterion_group!(benches, bench_ecdh_handshake, bench_aes_gcm);
criterion_main!(benches);
