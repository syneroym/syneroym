#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end integration tests for record signing (M06C Slice C3).

use std::sync::Arc;

use serde_json::json;
use syneroym_core::{
    local_registry::EndpointRegistry,
    record_signer::{CallerBinding, NodeRecordSigner, SigningError, SigningPrincipal},
    storage::MockStorage,
};
use syneroym_identity::{
    Identity,
    delegation::{DelegationCertificate, SCOPE_RECORD_SIGNING},
    substrate::derive_did_key,
};
use syneroym_signed_record::{Envelope, RecordDraft, VerifyOptions, verify};

#[tokio::test]
async fn record_signing_e2e_service_and_delegated_flow() {
    let node_id = Arc::new(Identity::generate().unwrap());
    let owner_id = Identity::generate().unwrap();
    let owner_did = derive_did_key(&owner_id.public_key());

    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let service_id = "did:key:zService1";

    registry.set_owner(service_id.to_string(), owner_did.clone()).await.unwrap();

    let signer = NodeRecordSigner::new(node_id.clone(), registry);

    // 1. Check identity
    let id_info = signer.identity(service_id);
    assert_eq!(id_info.owner_did.as_deref(), Some(owner_did.as_str()));

    // 2. Sign as service
    let draft = RecordDraft {
        version: 1,
        record_type: "listing".to_string(),
        subject: "subj_123".to_string(),
        payload: json!({"price": 100}),
        expires_at_secs: Some(2_000_000_000),
        supersedes: None,
    };

    let signed_json = signer
        .sign_record(service_id, draft.clone(), &SigningPrincipal::Service, CallerBinding::Internal)
        .expect("sign_record service");

    let envelope: Envelope = serde_json::from_str(&signed_json).expect("parse envelope");
    let verified =
        verify(&envelope, &VerifyOptions::new(envelope.issued_at_secs)).expect("verify record");
    assert_eq!(verified.subject, "subj_123");
    assert_eq!(verified.issuer, id_info.signing_did);

    // 3. Sign as delegated
    let temp_key = node_id.derive_service_identity(&owner_did, service_id).public_key();
    let cert =
        DelegationCertificate::issue(&owner_id, temp_key, 3600, SCOPE_RECORD_SIGNING.to_string())
            .unwrap();
    let cert_json = serde_json::to_string(&cert).unwrap();

    let signed_del_json = signer
        .sign_record(
            service_id,
            draft,
            &SigningPrincipal::Delegated { delegation_json: cert_json },
            CallerBinding::Verified(&owner_did),
        )
        .expect("sign_record delegated");

    let del_envelope: Envelope =
        serde_json::from_str(&signed_del_json).expect("parse del envelope");
    let del_verified = verify(&del_envelope, &VerifyOptions::new(del_envelope.issued_at_secs))
        .expect("verify del record");
    assert_eq!(del_verified.issuer, owner_did);
    assert_eq!(del_verified.signer_did, id_info.signing_did);

    // 4. Refuse invalid float payload
    let float_draft = RecordDraft {
        version: 1,
        record_type: "listing".to_string(),
        subject: "subj_123".to_string(),
        payload: json!({"price": 100.5}),
        expires_at_secs: None,
        supersedes: None,
    };
    let err = signer
        .sign_record(service_id, float_draft, &SigningPrincipal::Service, CallerBinding::Internal)
        .unwrap_err();
    assert!(matches!(err, SigningError::InvalidRecord(_)));
}
