//! Roym signed record constants and re-exports.

pub use syneroym_signed_record::{
    Envelope, RecordDraft, VerifiedRecord, VerifyError, VerifyOptions, verify,
};

/// Known Roym record types (spec D-C3-14).
pub const RECORD_TYPES: &[&str] = &[
    "listing",
    "request",
    "quote",
    "agreement-receipt",
    "payment-acknowledgement",
    "fulfilment-receipt",
    "membership-credential",
    "revocation",
    "moderation-decision",
];

/// Returns true if `record_type` is one of the known Roym record types.
pub fn is_known_record(record_type: &str) -> bool {
    RECORD_TYPES.contains(&record_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_record_types() {
        for t in RECORD_TYPES {
            assert!(is_known_record(t));
            let draft = RecordDraft {
                version: 1,
                record_type: t.to_string(),
                subject: "sub".to_string(),
                payload: serde_json::json!({"k": "v"}),
                expires_at_secs: None,
                supersedes: None,
            };
            assert!(draft.validate(0).is_ok(), "record type '{t}' must pass draft validation");
        }
        assert!(!is_known_record("unknown"));
        assert!(!is_known_record("profile.v1"));
    }
}
