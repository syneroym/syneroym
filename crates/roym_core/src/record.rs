//! Roym's signed record vocabulary, and the envelope re-exports the
//! product reads it through.

pub use syneroym_signed_record::{
    DelegationCertificate, EmptyRevocations, Envelope, RecordDraft, RevocationCheck, RevocationSet,
    RevocationSource, RevocationStatus, SCOPE_RECORD_SIGNING, VerifiedRecord, VerifyError,
    VerifyOptions, content_digest, verify, verify_json,
};

/// Every record type this product produces, and the version each is
/// produced at today. Fixed, like the card table: a record of an unlisted
/// type, or a listed type at an unlisted version, is not understood
/// rather than guessed.
///
/// `profile` proves who published a person's card and the conversation
/// address they claim. It is how a stranger reached by direct link gets
/// an address they can attribute, with no directory involved.
pub const RECORD_TYPES: &[(&str, u32)] = &[
    ("profile", 1),
    ("listing", 1),
    ("membership-credential", 1),
    ("revocation", 1),
    ("request", 1),
    ("quote", 1),
    ("agreement-receipt", 1),
    ("payment-acknowledgement", 1),
    ("fulfilment-receipt", 1),
    ("moderation-decision", 1),
];

pub const RECORD_PROFILE: &str = "profile";

pub fn is_known_record(record_type: &str, version: u32) -> bool {
    RECORD_TYPES.iter().any(|&(t, v)| t == record_type && v == version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_record_types() {
        for &(t, v) in RECORD_TYPES {
            assert!(is_known_record(t, v));
            let draft = RecordDraft {
                version: v,
                record_type: t.to_string(),
                subject: "sub".to_string(),
                payload: serde_json::json!({"k": "v"}),
                expires_at_secs: None,
                supersedes: None,
            };
            assert!(draft.validate(0).is_ok(), "record type '{t}' must pass draft validation");
        }
        assert!(!is_known_record("unknown", 1));
        assert!(!is_known_record("profile", 2));
    }
}
