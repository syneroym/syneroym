//! The one place a product record is signed. Holds the node identity and
//! the endpoint registry, derives the per-service signing key the same way
//! `SynSvcNativeService` and `resolve-instance-identity` already do, and
//! applies every refusal rule before it touches the key.
//!
//! Concrete and held `Arc`, not `Weak<dyn …>`: unlike `ConversationHost`
//! and `ServiceProxy` this holds no reference back to the sandbox engine or
//! the native factory, so there is no cycle to guard against.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use syneroym_identity::{
    Identity,
    delegation::{DelegationCertificate, SCOPE_RECORD_SIGNING},
    substrate::derive_did_key,
};
use syneroym_signed_record::{DraftError, Envelope, RecordDraft};

use crate::local_registry::EndpointRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigningPrincipal {
    Service,
    Delegated { delegation_json: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerBinding<'a> {
    Verified(&'a str),
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SigningError {
    #[error("no usable delegation: {0}")]
    NoDelegation(String),
    #[error("refused: {0}")]
    InvalidRecord(String),
    #[error("permission denied")]
    PermissionDenied,
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigningIdentity {
    pub signing_did: String,
    pub pubkey_hex: String,
    pub owner_did: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordClock {
    System,
    Fixed(u64),
}

#[derive(Debug)]
pub struct NodeRecordSigner {
    node_identity: Arc<Identity>,
    registry: EndpointRegistry,
    clock: RecordClock,
}

impl NodeRecordSigner {
    pub fn new(node_identity: Arc<Identity>, registry: EndpointRegistry) -> Arc<Self> {
        Arc::new(Self { node_identity, registry, clock: RecordClock::System })
    }

    pub fn with_clock(
        node_identity: Arc<Identity>,
        registry: EndpointRegistry,
        clock: RecordClock,
    ) -> Arc<Self> {
        Arc::new(Self { node_identity, registry, clock })
    }

    fn service_identity(&self, service_id: &str) -> Identity {
        let owner = self
            .registry
            .owner_of(service_id)
            .unwrap_or_else(|| derive_did_key(&self.node_identity.public_key()));
        self.node_identity.derive_service_identity(&owner, service_id)
    }

    pub fn identity(&self, service_id: &str) -> SigningIdentity {
        let key = self.service_identity(service_id);
        SigningIdentity {
            signing_did: derive_did_key(&key.public_key()),
            pubkey_hex: hex::encode(key.public_key().to_bytes()),
            owner_did: self.registry.owner_of(service_id),
        }
    }

    pub fn sign_record(
        &self,
        service_id: &str,
        draft: RecordDraft,
        principal: &SigningPrincipal,
        caller: CallerBinding<'_>,
    ) -> Result<String, SigningError> {
        let now = match self.clock {
            RecordClock::System => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| SigningError::Internal(e.to_string()))?
                .as_secs(),
            RecordClock::Fixed(t) => t,
        };

        let key = self.service_identity(service_id);
        let key_did = derive_did_key(&key.public_key());

        let (issuer, delegation) = match principal {
            SigningPrincipal::Service => (key_did, None),
            SigningPrincipal::Delegated { delegation_json } => {
                let cert = DelegationCertificate::from_json(delegation_json).map_err(|e| {
                    SigningError::NoDelegation(format!("not a delegation certificate: {e}"))
                })?;

                if cert.temporary_did != key_did {
                    return Err(SigningError::NoDelegation(format!(
                        "certificate certifies '{}', not this service's signing key '{key_did}'",
                        cert.temporary_did
                    )));
                }

                cert.verify(&cert.master_did, &[SCOPE_RECORD_SIGNING])
                    .map_err(|e| SigningError::NoDelegation(e.to_string()))?;

                if let CallerBinding::Verified(caller_did) = caller
                    && caller_did != cert.master_did
                {
                    return Err(SigningError::NoDelegation(format!(
                        "certificate names master '{}', which does not match the calling \
                         session's subject '{caller_did}'",
                        cert.master_did
                    )));
                }

                (cert.master_did.clone(), Some(delegation_json.clone()))
            }
        };

        let (mut env, bytes) = Envelope::unsigned(draft, issuer, delegation, now)
            .map_err(|e: DraftError| SigningError::InvalidRecord(e.to_string()))?;
        let sig = key.sign(&bytes).to_bytes();
        env.attach_signature(z32::encode(&sig))
            .map_err(|e| SigningError::Internal(e.to_string()))?;
        env.to_json().map_err(|e| SigningError::Internal(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use syneroym_identity::{delegation::SCOPE_SERVICE_INSTANCE, substrate::resolve_did_key};
    use syneroym_signed_record::{VerifyOptions, verify_json};

    use super::*;
    use crate::storage::MockStorage;

    fn sample_draft() -> RecordDraft {
        RecordDraft {
            version: 1,
            record_type: "listing".to_string(),
            subject: "sub_1".to_string(),
            payload: json!({"key": "val"}),
            expires_at_secs: None,
            supersedes: None,
        }
    }

    async fn make_registry() -> EndpointRegistry {
        EndpointRegistry::new(Arc::new(MockStorage::new())).await.unwrap()
    }

    #[tokio::test]
    async fn a_service_principal_signs_under_the_derived_key_and_verifies() {
        let node_id = Arc::new(Identity::generate().unwrap());
        let registry = make_registry().await;
        let signer = NodeRecordSigner::new(node_id, registry);

        let json_str = signer
            .sign_record(
                "svc1",
                sample_draft(),
                &SigningPrincipal::Service,
                CallerBinding::Internal,
            )
            .unwrap();

        let id = signer.identity("svc1");
        let opts =
            VerifyOptions::new(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());
        let verified = verify_json(&json_str, &opts.expecting(&id.signing_did)).unwrap();
        assert_eq!(verified.issuer, id.signing_did);
    }

    #[tokio::test]
    async fn the_derived_key_matches_what_resolve_instance_identity_reports() {
        let node_id = Arc::new(Identity::generate().unwrap());
        let owner = Identity::generate().unwrap();
        let owner_did = derive_did_key(&owner.public_key());
        let registry = make_registry().await;
        registry.set_owner("svc1".to_string(), owner_did.clone()).await.unwrap();

        let signer = NodeRecordSigner::new(node_id.clone(), registry);
        let id_signer = signer.identity("svc1");

        let expected_key = node_id.derive_service_identity(&owner_did, "svc1");
        let expected_did = derive_did_key(&expected_key.public_key());
        let expected_hex = hex::encode(expected_key.public_key().to_bytes());

        assert_eq!(id_signer.signing_did, expected_did);
        assert_eq!(id_signer.pubkey_hex, expected_hex);
        assert_eq!(id_signer.owner_did, Some(owner_did));
    }

    #[tokio::test]
    async fn a_delegated_certificate_over_another_key_is_refused() {
        let node_id = Arc::new(Identity::generate().unwrap());
        let master = Identity::generate().unwrap();
        let other_key = Identity::generate().unwrap();
        let registry = make_registry().await;
        let signer = NodeRecordSigner::new(node_id, registry);

        let cert = DelegationCertificate::issue(
            &master,
            other_key.public_key(),
            3600,
            SCOPE_RECORD_SIGNING.to_string(),
        )
        .unwrap();

        let err = signer
            .sign_record(
                "svc1",
                sample_draft(),
                &SigningPrincipal::Delegated { delegation_json: cert.to_json().unwrap() },
                CallerBinding::Internal,
            )
            .unwrap_err();

        assert!(matches!(err, SigningError::NoDelegation(_)));
    }

    #[tokio::test]
    async fn a_service_instance_scoped_certificate_is_refused_as_a_delegation() {
        let node_id = Arc::new(Identity::generate().unwrap());
        let master = Identity::generate().unwrap();
        let registry = make_registry().await;
        let signer = NodeRecordSigner::new(node_id, registry);

        let id = signer.identity("svc1");
        let pubkey = resolve_did_key(&id.signing_did).unwrap();

        let cert =
            DelegationCertificate::issue(&master, pubkey, 3600, SCOPE_SERVICE_INSTANCE.to_string())
                .unwrap();

        let err = signer
            .sign_record(
                "svc1",
                sample_draft(),
                &SigningPrincipal::Delegated { delegation_json: cert.to_json().unwrap() },
                CallerBinding::Internal,
            )
            .unwrap_err();

        assert!(matches!(err, SigningError::NoDelegation(_)));
    }

    #[tokio::test]
    async fn an_already_expired_delegated_certificate_is_refused_at_signing_time() {
        let node_id = Arc::new(Identity::generate().unwrap());
        let master = Identity::generate().unwrap();
        let registry = make_registry().await;
        let signer = NodeRecordSigner::new(node_id, registry);

        let id = signer.identity("svc1");
        let pubkey = resolve_did_key(&id.signing_did).unwrap();

        let cert =
            DelegationCertificate::issue(&master, pubkey, 0, SCOPE_RECORD_SIGNING.to_string())
                .unwrap();

        let err = signer
            .sign_record(
                "svc1",
                sample_draft(),
                &SigningPrincipal::Delegated { delegation_json: cert.to_json().unwrap() },
                CallerBinding::Internal,
            )
            .unwrap_err();

        assert!(matches!(err, SigningError::NoDelegation(_)));
    }

    #[tokio::test]
    async fn a_verified_caller_whose_did_does_not_match_the_certificates_master_is_refused() {
        let node_id = Arc::new(Identity::generate().unwrap());
        let master = Identity::generate().unwrap();
        let master_did = derive_did_key(&master.public_key());
        let registry = make_registry().await;
        let signer = NodeRecordSigner::new(node_id, registry);

        let id = signer.identity("svc1");
        let pubkey = resolve_did_key(&id.signing_did).unwrap();

        let cert =
            DelegationCertificate::issue(&master, pubkey, 3600, SCOPE_RECORD_SIGNING.to_string())
                .unwrap();

        let err = signer
            .sign_record(
                "svc1",
                sample_draft(),
                &SigningPrincipal::Delegated { delegation_json: cert.to_json().unwrap() },
                CallerBinding::Verified("did:key:z6M_wrong_caller"),
            )
            .unwrap_err();

        assert!(matches!(err, SigningError::NoDelegation(msg) if msg.contains("does not match")));

        // Correct caller succeeds
        let res = signer.sign_record(
            "svc1",
            sample_draft(),
            &SigningPrincipal::Delegated { delegation_json: cert.to_json().unwrap() },
            CallerBinding::Verified(&master_did),
        );
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn an_internal_caller_presenting_any_valid_certificate_is_not_checked_against_a_caller_did()
     {
        let node_id = Arc::new(Identity::generate().unwrap());
        let master = Identity::generate().unwrap();
        let registry = make_registry().await;
        let signer = NodeRecordSigner::new(node_id, registry);

        let id = signer.identity("svc1");
        let pubkey = resolve_did_key(&id.signing_did).unwrap();

        let cert =
            DelegationCertificate::issue(&master, pubkey, 3600, SCOPE_RECORD_SIGNING.to_string())
                .unwrap();

        let res = signer.sign_record(
            "svc1",
            sample_draft(),
            &SigningPrincipal::Delegated { delegation_json: cert.to_json().unwrap() },
            CallerBinding::Internal,
        );
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn a_float_payload_is_refused_as_invalid_record() {
        let node_id = Arc::new(Identity::generate().unwrap());
        let registry = make_registry().await;
        let signer = NodeRecordSigner::new(node_id, registry);

        let mut draft = sample_draft();
        draft.payload = json!({"amount": 10.5});

        let err = signer
            .sign_record("svc1", draft, &SigningPrincipal::Service, CallerBinding::Internal)
            .unwrap_err();

        assert!(matches!(err, SigningError::InvalidRecord(_)));
    }

    #[tokio::test]
    async fn two_signers_with_the_same_node_identity_and_owner_produce_identical_bytes() {
        let node_id = Arc::new(Identity::generate().unwrap());
        let owner = Identity::generate().unwrap();
        let owner_did = derive_did_key(&owner.public_key());

        let reg1 = make_registry().await;
        reg1.set_owner("svc1".to_string(), owner_did.clone()).await.unwrap();
        let signer1 =
            NodeRecordSigner::with_clock(node_id.clone(), reg1, RecordClock::Fixed(1_800_000_000));

        let reg2 = make_registry().await;
        reg2.set_owner("svc1".to_string(), owner_did).await.unwrap();
        let signer2 =
            NodeRecordSigner::with_clock(node_id, reg2, RecordClock::Fixed(1_800_000_000));

        let json1 = signer1
            .sign_record(
                "svc1",
                sample_draft(),
                &SigningPrincipal::Service,
                CallerBinding::Internal,
            )
            .unwrap();

        let json2 = signer2
            .sign_record(
                "svc1",
                sample_draft(),
                &SigningPrincipal::Service,
                CallerBinding::Internal,
            )
            .unwrap();

        assert_eq!(json1, json2);
    }
}
