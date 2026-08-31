//! Roym signed record constants and re-exports.

pub use syneroym_signed_record::{
    Envelope, RecordDraft, VerifiedRecord, VerifyError, VerifyOptions, verify,
};

/// Known Roym record types.
pub const RECORD_TYPES: &[&str] = &[
    "profile.v1",
    "catalog.listing.v1",
    "catalog.review.v1",
    "transaction.receipt.v1",
    "directory.attestation.v1",
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
        assert!(is_known_record("profile.v1"));
        assert!(is_known_record("catalog.listing.v1"));
        assert!(is_known_record("catalog.review.v1"));
        assert!(is_known_record("transaction.receipt.v1"));
        assert!(is_known_record("directory.attestation.v1"));
        assert!(!is_known_record("unknown.v1"));
    }
}
