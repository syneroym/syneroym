use aes_gcm::aead::OsRng;
use p256::{EncodedPoint, ecdh::EphemeralSecret, elliptic_curve::sec1::ToEncodedPoint};
use syneroym_identity::Identity;
use tokio::io::{AsyncReadExt, duplex, split};

use super::*;
use crate::{RoutePreamble, routing::EncryptionStage};

#[tokio::test]
async fn test_encryption_handshake_missing_epk_fails() {
    let identity = Identity::generate().unwrap();
    let mut preamble = RoutePreamble::binary_json_rpc("svc", "api");
    preamble.enc = Some("ecdh-p256".to_string());
    preamble.pubkey = Some("123456".to_string());

    let (client_stream, server_stream) = duplex(1024);
    let (server_r, server_w) = split(server_stream);
    drop(client_stream);

    let err = match apply_encryption_stage(
        server_r,
        server_w,
        &EncryptionStage::TerminateEcdhP256,
        &preamble,
        &identity,
    )
    .await
    {
        Ok(_) => panic!("expected encryption stage to fail due to missing epk"),
        Err(e) => e,
    };

    assert!(err.to_string().contains("epk"), "{err}");
}

#[tokio::test]
async fn test_encryption_handshake_with_epk_succeeds() {
    let identity = Identity::generate().unwrap();
    let expected_pubkey = identity.public_key();
    let client_secret = EphemeralSecret::random(&mut OsRng);
    let client_pub = EncodedPoint::from(client_secret.public_key());
    let client_pub_bytes = client_pub.as_bytes();

    let mut preamble = RoutePreamble::binary_json_rpc("svc", "api");
    preamble.enc = Some("ecdh-p256".to_string());
    preamble.epk = Some(hex::encode(client_pub_bytes));

    let (mut client_stream, server_stream) = tokio::io::duplex(1024);
    let (server_r, server_w) = tokio::io::split(server_stream);

    let handshake_task = tokio::spawn(async move {
        apply_encryption_stage(
            server_r,
            server_w,
            &EncryptionStage::TerminateEcdhP256,
            &preamble,
            &identity,
        )
        .await
    });

    // Client reads 65 bytes of server public key + 64 bytes of Ed25519 signature
    let mut server_pub_buf = [0u8; 65];
    client_stream.read_exact(&mut server_pub_buf).await.unwrap();

    let mut sig_buf = [0u8; 64];
    client_stream.read_exact(&mut sig_buf).await.unwrap();

    let mut signed_payload = Vec::with_capacity(130);
    signed_payload.extend_from_slice(&server_pub_buf);
    signed_payload.extend_from_slice(client_pub_bytes);

    let signature = ed25519_dalek::Signature::from_bytes(&sig_buf);
    let verifying_key =
        ed25519_dalek::VerifyingKey::from_bytes(&expected_pubkey.to_bytes()).unwrap();
    verifying_key
        .verify_strict(&signed_payload, &signature)
        .expect("handshake signature must verify");

    let stage_res = handshake_task.await.unwrap();
    assert!(stage_res.is_ok());
}

#[tokio::test]
async fn epk_compressed_point_is_refused() {
    let identity = Identity::generate().unwrap();
    let client_secret = EphemeralSecret::random(&mut OsRng);
    let compressed_point = client_secret.public_key().to_encoded_point(true);
    assert_eq!(compressed_point.as_bytes().len(), 33);

    let mut preamble = RoutePreamble::binary_json_rpc("svc", "api");
    preamble.enc = Some("ecdh-p256".to_string());
    preamble.epk = Some(hex::encode(compressed_point.as_bytes()));

    let (client_stream, server_stream) = tokio::io::duplex(1024);
    let (server_r, server_w) = tokio::io::split(server_stream);
    drop(client_stream);

    let err = apply_encryption_stage(
        server_r,
        server_w,
        &EncryptionStage::TerminateEcdhP256,
        &preamble,
        &identity,
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("epk"), "{err}");
}
