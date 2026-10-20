use std::sync::RwLock;

use syneroym_core::dht_registry::MasterAnchorPayload;
use syneroym_identity::{DelegationCertificate, Identity};

use super::MockResolver;
use crate::{
    RoutePreamble,
    handshake::{HandshakeVerifier, MasterAnchorResolver},
    preamble::proof_bytes,
};

#[derive(Debug)]
struct PanickingResolver;

#[async_trait::async_trait]
impl MasterAnchorResolver for PanickingResolver {
    async fn resolve_master_anchor(&self, _master_id: &str) -> anyhow::Result<MasterAnchorPayload> {
        panic!("resolver must not be called when signature verification fails");
    }
}

#[tokio::test]
async fn test_proof_rejections_and_success_direct_and_delegated() {
    let now = crate::handshake::now_secs();
    let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

    let master = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let other = Identity::generate().unwrap();
    let cert =
        DelegationCertificate::issue(&master, client.public_key(), 3600, "routing".to_string())
            .unwrap();

    // 1. Direct (no cert) cases
    // Valid proof
    let mut p = RoutePreamble::binary_json_rpc("svc", "api");
    p.sign(&client, now);
    assert!(HandshakeVerifier::verify_preamble_at(&p, &resolver, now).await.is_ok());

    // Missing sig
    let mut p_no_sig = p.clone();
    p_no_sig.sig = None;
    let err = HandshakeVerifier::verify_preamble_at(&p_no_sig, &resolver, now).await.unwrap_err();
    assert!(err.to_string().contains("has no sig"), "{err}");

    // Missing ts
    let mut p_no_ts = p.clone();
    p_no_ts.ts = None;
    let err = HandshakeVerifier::verify_preamble_at(&p_no_ts, &resolver, now).await.unwrap_err();
    assert!(err.to_string().contains("has no ts"), "{err}");

    // Signed by another key
    let mut p_other_key = p.clone();
    p_other_key.sig = Some(hex::encode(other.sign(&proof_bytes("svc", "api", now)).to_bytes()));
    let err =
        HandshakeVerifier::verify_preamble_at(&p_other_key, &resolver, now).await.unwrap_err();
    assert!(err.to_string().contains("signature verification failed"), "{err}");

    // Signed over another service_id
    let mut p_other_svc = p.clone();
    p_other_svc.sig =
        Some(hex::encode(client.sign(&proof_bytes("other_svc", "api", now)).to_bytes()));
    let err =
        HandshakeVerifier::verify_preamble_at(&p_other_svc, &resolver, now).await.unwrap_err();
    assert!(err.to_string().contains("signature verification failed"), "{err}");

    // Signed over another interface
    let mut p_other_iface = p.clone();
    p_other_iface.sig =
        Some(hex::encode(client.sign(&proof_bytes("svc", "other_iface", now)).to_bytes()));
    let err =
        HandshakeVerifier::verify_preamble_at(&p_other_iface, &resolver, now).await.unwrap_err();
    assert!(err.to_string().contains("signature verification failed"), "{err}");

    // 2. Delegated cases
    let mut p_del = RoutePreamble::binary_json_rpc("svc", "api");
    p_del.delegation = Some(cert.clone());
    p_del.sign(&client, now);
    assert!(HandshakeVerifier::verify_preamble_at(&p_del, &resolver, now).await.is_ok());

    let mut p_del_other_key = p_del.clone();
    p_del_other_key.sig = Some(hex::encode(other.sign(&proof_bytes("svc", "api", now)).to_bytes()));
    assert!(HandshakeVerifier::verify_preamble_at(&p_del_other_key, &resolver, now).await.is_err());
}

#[tokio::test]
async fn test_timestamp_window_bounds() {
    let now = 1_700_000_000u64;
    let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };
    let client = Identity::generate().unwrap();

    // Accepted: now - 899, now + 899
    let mut p1 = RoutePreamble::binary_json_rpc("svc", "api");
    p1.sign(&client, now - 899);
    assert!(HandshakeVerifier::verify_preamble_at(&p1, &resolver, now).await.is_ok());

    let mut p2 = RoutePreamble::binary_json_rpc("svc", "api");
    p2.sign(&client, now + 899);
    assert!(HandshakeVerifier::verify_preamble_at(&p2, &resolver, now).await.is_ok());

    // Refused: now - 901, now + 901
    let mut p3 = RoutePreamble::binary_json_rpc("svc", "api");
    p3.sign(&client, now - 901);
    let err3 = HandshakeVerifier::verify_preamble_at(&p3, &resolver, now).await.unwrap_err();
    assert!(err3.to_string().contains("is more than 900 s from this node's clock"), "{err3}");

    let mut p4 = RoutePreamble::binary_json_rpc("svc", "api");
    p4.sign(&client, now + 901);
    let err4 = HandshakeVerifier::verify_preamble_at(&p4, &resolver, now).await.unwrap_err();
    assert!(err4.to_string().contains("is more than 900 s from this node's clock"), "{err4}");
}

#[tokio::test]
async fn test_forged_signature_refused_without_calling_resolver() {
    let now = 1_700_000_000u64;
    let resolver = PanickingResolver;

    let master = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let other = Identity::generate().unwrap();
    let cert =
        DelegationCertificate::issue(&master, client.public_key(), 3600, "routing".to_string())
            .unwrap();

    let mut p = RoutePreamble::binary_json_rpc("svc", "api");
    p.delegation = Some(cert);
    p.sign(&client, now);
    p.sig = Some(hex::encode(other.sign(&proof_bytes("svc", "api", now)).to_bytes()));

    let err = HandshakeVerifier::verify_preamble_at(&p, &resolver, now).await.unwrap_err();
    assert!(err.to_string().contains("signature verification failed"), "{err}");
}
