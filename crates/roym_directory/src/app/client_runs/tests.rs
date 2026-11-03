use syneroym_roym_core::directory::SourceError;
use syneroym_roym_federated_query::SourceReport;

use super::errors_to_record;

#[test]
fn errors_to_record_skips_not_started() {
    let reports = vec![
        SourceReport {
            source: "did:key:sOk".to_string(),
            verified: 5,
            refused: 0,
            truncated: false,
            error: None,
        },
        SourceReport {
            source: "did:key:sNotStarted".to_string(),
            verified: 0,
            refused: 0,
            truncated: false,
            error: Some(SourceError::NotStarted),
        },
        SourceReport {
            source: "did:key:sNotFound".to_string(),
            verified: 0,
            refused: 0,
            truncated: false,
            error: Some(SourceError::NotFound),
        },
        SourceReport {
            source: "did:key:sTimedOut".to_string(),
            verified: 0,
            refused: 0,
            truncated: false,
            error: Some(SourceError::TimedOut),
        },
    ];

    let to_record = errors_to_record(&reports);
    assert_eq!(to_record.len(), 2);
    assert_eq!(to_record[0], ("did:key:sNotFound", &SourceError::NotFound));
    assert_eq!(to_record[1], ("did:key:sTimedOut", &SourceError::TimedOut));
}
