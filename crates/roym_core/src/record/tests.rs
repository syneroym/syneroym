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

mod wire_words;
