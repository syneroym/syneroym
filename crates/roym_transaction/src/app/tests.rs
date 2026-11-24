//! Test fixtures and harness support for transaction service in-crate tests.

pub mod accept;
pub mod as_listed_accept;
pub mod as_listed_enquiry;
pub mod as_listed_owner;
pub mod cancel;
pub mod commitment_failure;
pub mod commitment_failure_backup;
pub mod commitment_failure_writer;
pub mod decide;
pub mod decline;
pub mod disclosure;
pub mod drive;
pub mod extension;
pub mod hooks;
pub mod hooks_fake;
pub mod present;
pub mod recover;
pub mod refuse;
pub mod restore;
pub mod restore_reservations;
pub mod settle;
pub mod step;
pub mod step_backup;
pub mod step_filing;
pub mod time;

use std::sync::Arc;

use syneroym_app_host::types::signing::{Principal, RecordDraft, SigningError, SigningIdentity};
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    clock,
    record::{Envelope, RecordDraft as CoreRecordDraft},
    signing::{
        CERTIFICATE_ID, CERTIFICATES, DelegationCertificate, MAX_CERTIFICATE_LIFETIME_SECS,
        SCOPE_RECORD_SIGNING, StoredCertificate,
    },
    testing::TestHost,
};

pub struct TestFixture {
    pub host: TestHost,
    pub(crate) scheduling: hooks_fake::FakeScheduling,
    pub master: Arc<Identity>,
    pub master_did: String,
    pub service_key: Arc<Identity>,
    pub service_did: String,
    pub cert_json: String,
}

impl std::fmt::Debug for TestFixture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestFixture")
            .field("host", &self.host)
            .field("master_did", &self.master_did)
            .field("service_did", &self.service_did)
            .finish()
    }
}

/// What the fixture's record signer does. A test that replaces the signer
/// calls it to keep signing the same way.
pub fn sign_record(
    service_key: &Identity,
    master_did: &str,
    service_did: &str,
    draft: RecordDraft,
    principal: Principal,
    now_secs: u64,
) -> Result<String, SigningError> {
    let (issuer, delegation) = match principal {
        Principal::Service => (service_did.to_string(), None),
        Principal::Delegated(delegation_json) => (master_did.to_string(), Some(delegation_json)),
    };

    let payload_val = serde_json::from_str(&draft.payload)
        .map_err(|e| SigningError::InvalidRecord(e.to_string()))?;
    let core_draft = CoreRecordDraft {
        version: draft.version,
        record_type: draft.record_type,
        subject: draft.subject,
        payload: payload_val,
        expires_at_secs: draft.expires_at_secs,
        supersedes: draft.supersedes,
    };

    let (mut env, bytes) = Envelope::unsigned(core_draft, issuer, delegation, now_secs)
        .map_err(|e| SigningError::InvalidRecord(e.to_string()))?;
    let sig = service_key.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig))
        .map_err(|e| SigningError::InvalidRecord(e.to_string()))?;
    env.to_json().map_err(|e| SigningError::InvalidRecord(e.to_string()))
}

pub fn fixture() -> TestFixture {
    let master = Arc::new(Identity::generate().expect("generate master key"));
    fixture_with_master(master)
}

pub fn fixture_with_master(master: Arc<Identity>) -> TestFixture {
    let master_did = substrate::derive_did_key(&master.public_key());

    let service_key = Arc::new(Identity::generate().expect("generate service key"));
    let service_did = substrate::derive_did_key(&service_key.public_key());

    let now = clock::now_secs();
    let cert = DelegationCertificate::issue(
        &master,
        service_key.public_key(),
        MAX_CERTIFICATE_LIFETIME_SECS,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .expect("issue delegation certificate");
    let cert_json = cert.to_json().expect("cert to json");

    let host = TestHost::default();
    host.set_now(now);

    let pubkey_hex: String =
        service_key.public_key().as_bytes().iter().map(|b| format!("{b:02x}")).collect();

    host.set_signing_id(SigningIdentity {
        signing_did: service_did.clone(),
        pubkey_hex,
        owner_did: Some(master_did.clone()),
    });

    let stored = StoredCertificate {
        signing_did: service_did.clone(),
        master_did: master_did.clone(),
        issued_at_secs: cert.issued_at_secs,
        expires_at_secs: cert.expires_at_secs,
        certificate: cert_json.clone(),
    };
    host.set_storage_row(CERTIFICATES, CERTIFICATE_ID, &stored);

    let key_clone = Arc::clone(&service_key);
    let master_did_clone = master_did.clone();
    let service_did_clone = service_did.clone();
    host.set_record_signer(move |draft, principal, now_secs| {
        sign_record(&key_clone, &master_did_clone, &service_did_clone, draft, principal, now_secs)
    });

    let scheduling = hooks_fake::FakeScheduling::install(&host);

    TestFixture { host, scheduling, master, master_did, service_key, service_did, cert_json }
}

#[test]
fn fixture_initializes_valid_signer_and_stubs() {
    let f = fixture();
    assert!(f.host.now() > 0);
    assert!(!f.master_did.is_empty());
    assert!(!f.service_did.is_empty());
}
