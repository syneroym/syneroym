use std::sync::RwLock;

use syneroym_core::dht_registry::MasterAnchorPayload;
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_rpc::OnBehalfProof;

use super::MockResolver;
use crate::{handshake::HandshakeVerifier, preamble::proof_bytes};

#[tokio::test]
async fn test_verify_on_behalf_cases() {
    let now = crate::handshake::now_secs();
    let master = Identity::generate().unwrap();
    let p_client = Identity::generate().unwrap();
    let other = Identity::generate().unwrap();

    let cert =
        DelegationCertificate::issue(&master, p_client.public_key(), 3600, "routing".to_string())
            .unwrap();
    let p_did = substrate::derive_did_key(&p_client.public_key());
    let master_did = substrate::derive_did_key(&master.public_key());

    let bytes = proof_bytes("orig_svc", "orig_iface", now);
    let sig = hex::encode(p_client.sign(&bytes).to_bytes());

    let valid_proof = OnBehalfProof {
        pubkey: hex::encode(p_client.public_key().to_bytes()),
        delegation: Some(cert.to_json().unwrap()),
        service_id: "orig_svc".to_string(),
        interface: "orig_iface".to_string(),
        ts: now,
        sig: sig.clone(),
    };

    let resolver = MockResolver { anchor: RwLock::new(MasterAnchorPayload::default()) };

    // 1. Valid P proof
    let verified =
        HandshakeVerifier::verify_on_behalf_at(&valid_proof, &resolver, now).await.unwrap();
    assert_eq!(verified.master_did, master_did);
    assert_eq!(verified.temporary_did, p_did);

    // 2. Forged sig
    let mut forged_proof = valid_proof.clone();
    forged_proof.sig = hex::encode(other.sign(&bytes).to_bytes());
    let err =
        HandshakeVerifier::verify_on_behalf_at(&forged_proof, &resolver, now).await.unwrap_err();
    assert!(err.to_string().contains("signature verification failed"), "{err}");

    // 3. Old ts (outside 15-minute window)
    let mut old_proof = valid_proof.clone();
    old_proof.ts = now - 901;
    let old_bytes = proof_bytes("orig_svc", "orig_iface", now - 901);
    old_proof.sig = hex::encode(p_client.sign(&old_bytes).to_bytes());
    let err = HandshakeVerifier::verify_on_behalf_at(&old_proof, &resolver, now).await.unwrap_err();
    assert!(err.to_string().contains("is more than 900 s from this node's clock"), "{err}");

    // 4. Revoked P key
    let mut revoked_anchor = MasterAnchorPayload::default();
    revoked_anchor.revoked_keys.push(p_did);
    let revoked_resolver = MockResolver { anchor: RwLock::new(revoked_anchor) };
    let err = HandshakeVerifier::verify_on_behalf_at(&valid_proof, &revoked_resolver, now)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("has been revoked"), "{err}");

    // 5. Unreadable delegation
    let mut bad_del_proof = valid_proof.clone();
    bad_del_proof.delegation = Some("not valid json".to_string());
    let err =
        HandshakeVerifier::verify_on_behalf_at(&bad_del_proof, &resolver, now).await.unwrap_err();
    assert!(err.to_string().contains("unreadable on_behalf certificate"), "{err}");
}
