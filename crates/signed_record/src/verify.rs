use serde_json::Value;
use syneroym_identity::{delegation::DelegationCertificate, substrate};

use crate::envelope::{DraftError, ENVELOPE_VERSION, Envelope, RecordDraft};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationCheck {
    Good,
    Revoked,
    Unknown,
}

pub trait RevocationSource {
    fn check_did(&self, did: &str) -> RevocationCheck;
    fn check_record(&self, record_id: &str) -> RevocationCheck;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyRevocations;

impl RevocationSource for EmptyRevocations {
    fn check_did(&self, _did: &str) -> RevocationCheck {
        RevocationCheck::Unknown
    }
    fn check_record(&self, _record_id: &str) -> RevocationCheck {
        RevocationCheck::Unknown
    }
}

#[derive(Debug, Clone, Default)]
pub struct RevocationSet {
    pub revoked_dids: std::collections::BTreeSet<String>,
    pub revoked_records: std::collections::BTreeSet<String>,
}

impl RevocationSource for RevocationSet {
    fn check_did(&self, did: &str) -> RevocationCheck {
        if self.revoked_dids.contains(did) {
            RevocationCheck::Revoked
        } else {
            RevocationCheck::Unknown
        }
    }
    fn check_record(&self, record_id: &str) -> RevocationCheck {
        if self.revoked_records.contains(record_id) {
            RevocationCheck::Revoked
        } else {
            RevocationCheck::Unknown
        }
    }
}

pub const DEFAULT_ACCEPTED_SCOPES: &[&str] = &[crate::SCOPE_RECORD_SIGNING];

pub struct VerifyOptions<'a> {
    pub now_secs: u64,
    pub expected_issuer: Option<&'a str>,
    pub accepted_scopes: &'a [&'a str],
    pub revoked: &'a dyn RevocationSource,
    pub max_clock_skew_secs: u64,
}

impl<'a> std::fmt::Debug for VerifyOptions<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifyOptions")
            .field("now_secs", &self.now_secs)
            .field("expected_issuer", &self.expected_issuer)
            .field("accepted_scopes", &self.accepted_scopes)
            .field("max_clock_skew_secs", &self.max_clock_skew_secs)
            .finish_non_exhaustive()
    }
}

const EMPTY_REVOCATIONS: EmptyRevocations = EmptyRevocations;

impl<'a> VerifyOptions<'a> {
    pub fn new(now_secs: u64) -> VerifyOptions<'static> {
        VerifyOptions {
            now_secs,
            expected_issuer: None,
            accepted_scopes: DEFAULT_ACCEPTED_SCOPES,
            revoked: &EMPTY_REVOCATIONS,
            max_clock_skew_secs: 300,
        }
    }

    pub fn expecting(mut self, issuer: &'a str) -> Self {
        self.expected_issuer = Some(issuer);
        self
    }

    pub fn with_revocations(mut self, src: &'a dyn RevocationSource) -> Self {
        self.revoked = src;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationStatus {
    Good,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRecord {
    pub record_id: String,
    pub issuer: String,
    pub signer_did: String,
    pub record_type: String,
    pub version: u32,
    pub subject: String,
    pub payload: Value,
    pub issued_at_secs: u64,
    pub expires_at_secs: Option<u64>,
    pub supersedes: Option<String>,
    pub revocation_status: RevocationStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("malformed record: {0}")]
    Malformed(String),
    #[error("envelope version {0} is not understood by this build")]
    UnknownEnvelopeVersion(u32),
    #[error("issuer mismatch: expected '{expected}', got '{actual}'")]
    IssuerMismatch { expected: String, actual: String },
    #[error("record expired at {expires_at_secs} (now {now_secs})")]
    Expired { now_secs: u64, expires_at_secs: u64 },
    #[error("record is dated {issued_at_secs}, in the future (now {now_secs})")]
    IssuedInFuture { now_secs: u64, issued_at_secs: u64 },
    #[error("delegation is invalid: {0}")]
    BadDelegation(String),
    #[error("signature is invalid: {0}")]
    BadSignature(String),
    #[error("signing key '{0}' is revoked")]
    RevokedKey(String),
    #[error("record '{0}' is revoked")]
    RevokedRecord(String),
}

pub fn verify(e: &Envelope, o: &VerifyOptions<'_>) -> Result<VerifiedRecord, VerifyError> {
    if e.envelope_version != ENVELOPE_VERSION {
        return Err(VerifyError::UnknownEnvelopeVersion(e.envelope_version));
    }

    let draft = RecordDraft {
        version: e.version,
        record_type: e.record_type.clone(),
        subject: e.subject.clone(),
        payload: e.payload.clone(),
        expires_at_secs: None,
        supersedes: e.supersedes.clone(),
    };
    if let Err(d_err) = draft.validate(0) {
        match d_err {
            DraftError::ExpiryInPast { .. } => (),
            other => return Err(VerifyError::Malformed(other.to_string())),
        }
    }

    if let Some(want) = o.expected_issuer
        && want != e.issuer
    {
        return Err(VerifyError::IssuerMismatch {
            expected: want.to_string(),
            actual: e.issuer.clone(),
        });
    }

    if e.issued_at_secs > o.now_secs + o.max_clock_skew_secs {
        return Err(VerifyError::IssuedInFuture {
            now_secs: o.now_secs,
            issued_at_secs: e.issued_at_secs,
        });
    }

    if let Some(exp) = e.expires_at_secs
        && o.now_secs >= exp
    {
        return Err(VerifyError::Expired { now_secs: o.now_secs, expires_at_secs: exp });
    }

    let signer_did = match &e.delegation {
        Some(json) => {
            let cert = DelegationCertificate::from_json(json)
                .map_err(|err| VerifyError::BadDelegation(err.to_string()))?;
            cert.verify_chain(&e.issuer, o.accepted_scopes)
                .map_err(|err| VerifyError::BadDelegation(err.to_string()))?;
            if e.issued_at_secs < cert.issued_at_secs || e.issued_at_secs >= cert.expires_at_secs {
                return Err(VerifyError::BadDelegation(format!(
                    "record issued at {}, outside the certificate's own window [{}, {})",
                    e.issued_at_secs, cert.issued_at_secs, cert.expires_at_secs
                )));
            }
            cert.temporary_did
        }
        None => e.issuer.clone(),
    };

    let signer_check = o.revoked.check_did(&signer_did);
    if signer_check == RevocationCheck::Revoked {
        return Err(VerifyError::RevokedKey(signer_did));
    }

    let issuer_check = o.revoked.check_did(&e.issuer);
    if issuer_check == RevocationCheck::Revoked {
        return Err(VerifyError::RevokedKey(e.issuer.clone()));
    }

    let signing_bytes = e.signing_bytes().map_err(|err| VerifyError::Malformed(err.to_string()))?;
    let json_val: Value = serde_json::from_slice(&signing_bytes)
        .map_err(|err| VerifyError::Malformed(err.to_string()))?;
    substrate::verify_json_signature(&signer_did, &json_val, &e.signature)
        .map_err(|err| VerifyError::BadSignature(err.to_string()))?;

    let record_id = e.record_id().map_err(|err| VerifyError::Malformed(err.to_string()))?;
    let record_check = o.revoked.check_record(&record_id);
    if record_check == RevocationCheck::Revoked {
        return Err(VerifyError::RevokedRecord(record_id));
    }

    let revocation_status = if signer_check == RevocationCheck::Unknown
        || issuer_check == RevocationCheck::Unknown
        || record_check == RevocationCheck::Unknown
    {
        RevocationStatus::Unknown
    } else {
        RevocationStatus::Good
    };

    Ok(VerifiedRecord {
        record_id,
        issuer: e.issuer.clone(),
        signer_did,
        record_type: e.record_type.clone(),
        version: e.version,
        subject: e.subject.clone(),
        payload: e.payload.clone(),
        issued_at_secs: e.issued_at_secs,
        expires_at_secs: e.expires_at_secs,
        supersedes: e.supersedes.clone(),
        revocation_status,
    })
}

pub fn verify_json(json: &str, o: &VerifyOptions<'_>) -> Result<VerifiedRecord, VerifyError> {
    let e = Envelope::from_json(json).map_err(|err| VerifyError::Malformed(err.to_string()))?;
    verify(&e, o)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use syneroym_identity::{
        Identity,
        delegation::{DelegationCertificate, SCOPE_RECORD_SIGNING, SCOPE_SERVICE_INSTANCE},
        substrate,
    };

    use super::*;

    fn sample_draft() -> RecordDraft {
        RecordDraft {
            version: 1,
            record_type: "listing".to_string(),
            subject: "sub_123".to_string(),
            payload: json!({"item": "book", "price": 42}),
            expires_at_secs: None,
            supersedes: None,
        }
    }

    fn sign_env(
        identity: &Identity,
        draft: RecordDraft,
        issuer: String,
        delegation: Option<String>,
        issued_at_secs: u64,
    ) -> Envelope {
        let (mut env, bytes) =
            Envelope::unsigned(draft, issuer, delegation, issued_at_secs).unwrap();
        let sig = z32::encode(&identity.sign(&bytes).to_bytes());
        env.attach_signature(sig).unwrap();
        env
    }

    #[test]
    fn sign_then_verify_round_trips() {
        let key = Identity::generate().unwrap();
        let issuer = substrate::derive_did_key(&key.public_key());
        let env = sign_env(&key, sample_draft(), issuer.clone(), None, 1000);

        let opts = VerifyOptions::new(1000);
        let res = verify(&env, &opts).unwrap();
        assert_eq!(res.issuer, issuer);
        assert_eq!(res.signer_did, issuer);
        assert_eq!(res.record_type, "listing");
        assert_eq!(res.version, 1);
        assert_eq!(res.revocation_status, RevocationStatus::Unknown);
    }

    #[test]
    fn a_tampered_payload_field_fails_the_signature() {
        let key = Identity::generate().unwrap();
        let issuer = substrate::derive_did_key(&key.public_key());
        let mut env = sign_env(&key, sample_draft(), issuer, None, 1000);
        env.payload = json!({"item": "book", "price": 43});

        let opts = VerifyOptions::new(1000);
        assert!(matches!(verify(&env, &opts), Err(VerifyError::BadSignature(_))));
    }

    #[test]
    fn a_swapped_delegation_fails_because_it_is_inside_the_signed_bytes() {
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());
        let service = Identity::generate().unwrap();

        let cert1 = DelegationCertificate::issue(
            &master,
            service.public_key(),
            3600,
            SCOPE_RECORD_SIGNING.to_string(),
        )
        .unwrap();

        let env = sign_env(
            &service,
            sample_draft(),
            master_did.clone(),
            Some(cert1.to_json().unwrap()),
            cert1.issued_at_secs + 10,
        );

        let cert2 = DelegationCertificate::issue(
            &master,
            service.public_key(),
            7200,
            SCOPE_RECORD_SIGNING.to_string(),
        )
        .unwrap();

        let mut tampered_env = env.clone();
        tampered_env.delegation = Some(cert2.to_json().unwrap());

        let opts = VerifyOptions::new(cert1.issued_at_secs + 10);
        assert!(matches!(verify(&tampered_env, &opts), Err(VerifyError::BadSignature(_))));

        // Also test verify_json
        let json_str = env.to_json().unwrap();
        assert!(verify_json(&json_str, &opts).is_ok());
    }

    #[test]
    fn a_service_instance_scoped_certificate_is_refused_as_a_record_delegation() {
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());
        let service = Identity::generate().unwrap();

        let cert = DelegationCertificate::issue(
            &master,
            service.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();

        let env = sign_env(
            &service,
            sample_draft(),
            master_did,
            Some(cert.to_json().unwrap()),
            cert.issued_at_secs + 10,
        );

        let opts = VerifyOptions::new(cert.issued_at_secs + 10);
        assert!(matches!(verify(&env, &opts), Err(VerifyError::BadDelegation(_))));
    }

    #[test]
    fn verify_error_variants_coverage() {
        let key = Identity::generate().unwrap();
        let issuer = substrate::derive_did_key(&key.public_key());
        let env = sign_env(&key, sample_draft(), issuer.clone(), None, 1000);

        // UnknownEnvelopeVersion
        let mut e = env.clone();
        e.envelope_version = 99;
        assert!(matches!(
            verify(&e, &VerifyOptions::new(1000)),
            Err(VerifyError::UnknownEnvelopeVersion(99))
        ));

        // IssuerMismatch
        assert!(matches!(
            verify(&env, &VerifyOptions::new(1000).expecting("did:key:wrong")),
            Err(VerifyError::IssuerMismatch { .. })
        ));

        // IssuedInFuture
        assert!(matches!(
            verify(&env, &VerifyOptions::new(100)),
            Err(VerifyError::IssuedInFuture { .. })
        ));

        // Expired
        let mut d = sample_draft();
        d.expires_at_secs = Some(1500);
        let env_exp = sign_env(&key, d, issuer.clone(), None, 1000);
        assert!(matches!(
            verify(&env_exp, &VerifyOptions::new(1500)),
            Err(VerifyError::Expired { .. })
        ));

        // Malformed
        let mut e_mal = env.clone();
        e_mal.version = 0;
        assert!(matches!(
            verify(&e_mal, &VerifyOptions::new(1000)),
            Err(VerifyError::Malformed(_))
        ));
    }

    #[test]
    fn a_record_signed_under_a_still_valid_certificate_verifies_after_that_certificate_has_since_wall_clock_expired()
     {
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());
        let service = Identity::generate().unwrap();

        let cert = DelegationCertificate::issue(
            &master,
            service.public_key(),
            3600,
            SCOPE_RECORD_SIGNING.to_string(),
        )
        .unwrap();

        let t0 = cert.issued_at_secs + 10;
        let env = sign_env(
            &service,
            sample_draft(),
            master_did.clone(),
            Some(cert.to_json().unwrap()),
            t0,
        );

        // Verify far in the future (after cert expired)
        let opts = VerifyOptions::new(t0 + 100_000);
        let res = verify(&env, &opts).unwrap();
        assert_eq!(res.issuer, master_did);
        assert_eq!(res.signer_did, substrate::derive_did_key(&service.public_key()));
    }

    #[test]
    fn a_record_dated_before_the_certificate_was_issued_is_refused() {
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());
        let service = Identity::generate().unwrap();

        let cert = DelegationCertificate::issue(
            &master,
            service.public_key(),
            3600,
            SCOPE_RECORD_SIGNING.to_string(),
        )
        .unwrap();

        let env = sign_env(
            &service,
            sample_draft(),
            master_did,
            Some(cert.to_json().unwrap()),
            cert.issued_at_secs - 10,
        );

        let opts = VerifyOptions::new(cert.issued_at_secs);
        assert!(matches!(verify(&env, &opts), Err(VerifyError::BadDelegation(_))));
    }

    #[test]
    fn a_record_dated_at_or_after_the_certificate_expires_is_refused() {
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());
        let service = Identity::generate().unwrap();

        let cert = DelegationCertificate::issue(
            &master,
            service.public_key(),
            3600,
            SCOPE_RECORD_SIGNING.to_string(),
        )
        .unwrap();

        let env = sign_env(
            &service,
            sample_draft(),
            master_did,
            Some(cert.to_json().unwrap()),
            cert.expires_at_secs,
        );

        let opts = VerifyOptions::new(cert.expires_at_secs);
        assert!(matches!(verify(&env, &opts), Err(VerifyError::BadDelegation(_))));
    }

    struct CleanRevocationSource {
        clean_dids: std::collections::BTreeSet<String>,
        clean_records: std::collections::BTreeSet<String>,
    }

    impl RevocationSource for CleanRevocationSource {
        fn check_did(&self, did: &str) -> RevocationCheck {
            if self.clean_dids.contains(did) {
                RevocationCheck::Good
            } else {
                RevocationCheck::Unknown
            }
        }
        fn check_record(&self, record_id: &str) -> RevocationCheck {
            if self.clean_records.contains(record_id) {
                RevocationCheck::Good
            } else {
                RevocationCheck::Unknown
            }
        }
    }

    #[test]
    fn an_unknown_did_and_an_unknown_record_verify_as_good_with_unknown_revocation_status() {
        let key = Identity::generate().unwrap();
        let issuer = substrate::derive_did_key(&key.public_key());
        let env = sign_env(&key, sample_draft(), issuer, None, 1000);

        let opts = VerifyOptions::new(1000);
        let res = verify(&env, &opts).unwrap();
        assert_eq!(res.revocation_status, RevocationStatus::Unknown);
    }

    #[test]
    fn a_revoked_signing_key_is_a_hard_verify_error() {
        let key = Identity::generate().unwrap();
        let issuer = substrate::derive_did_key(&key.public_key());
        let env = sign_env(&key, sample_draft(), issuer.clone(), None, 1000);

        let mut rset = RevocationSet::default();
        rset.revoked_dids.insert(issuer.clone());

        let opts = VerifyOptions::new(1000).with_revocations(&rset);
        assert!(matches!(
            verify(&env, &opts),
            Err(VerifyError::RevokedKey(k)) if k == issuer
        ));
    }

    #[test]
    fn a_revoked_issuer_did_is_a_hard_verify_error() {
        let master = Identity::generate().unwrap();
        let master_did = substrate::derive_did_key(&master.public_key());
        let service = Identity::generate().unwrap();

        let cert = DelegationCertificate::issue(
            &master,
            service.public_key(),
            3600,
            SCOPE_RECORD_SIGNING.to_string(),
        )
        .unwrap();

        let env = sign_env(
            &service,
            sample_draft(),
            master_did.clone(),
            Some(cert.to_json().unwrap()),
            cert.issued_at_secs + 10,
        );

        let mut rset = RevocationSet::default();
        rset.revoked_dids.insert(master_did.clone());

        let opts = VerifyOptions::new(cert.issued_at_secs + 10).with_revocations(&rset);
        assert!(matches!(
            verify(&env, &opts),
            Err(VerifyError::RevokedKey(k)) if k == master_did
        ));
    }

    #[test]
    fn a_revoked_record_id_is_a_hard_verify_error() {
        let key = Identity::generate().unwrap();
        let issuer = substrate::derive_did_key(&key.public_key());
        let env = sign_env(&key, sample_draft(), issuer, None, 1000);
        let rec_id = env.record_id().unwrap();

        let mut rset = RevocationSet::default();
        rset.revoked_records.insert(rec_id.clone());

        let opts = VerifyOptions::new(1000).with_revocations(&rset);
        assert!(matches!(
            verify(&env, &opts),
            Err(VerifyError::RevokedRecord(r)) if r == rec_id
        ));
    }

    #[test]
    fn a_known_clean_did_and_record_verify_as_good_with_good_revocation_status() {
        let key = Identity::generate().unwrap();
        let issuer = substrate::derive_did_key(&key.public_key());
        let env = sign_env(&key, sample_draft(), issuer.clone(), None, 1000);
        let rec_id = env.record_id().unwrap();

        let mut src = CleanRevocationSource {
            clean_dids: std::collections::BTreeSet::new(),
            clean_records: std::collections::BTreeSet::new(),
        };
        src.clean_dids.insert(issuer);
        src.clean_records.insert(rec_id);

        let opts = VerifyOptions::new(1000).with_revocations(&src);
        let res = verify(&env, &opts).unwrap();
        assert_eq!(res.revocation_status, RevocationStatus::Good);
    }
}
