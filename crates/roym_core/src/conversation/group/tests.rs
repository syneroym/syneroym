use std::{fs, path::PathBuf};

use super::*;
use crate::conversation::{BodyEncoding, Direction, StoredState};

fn make_test_row(id: &str, ts: i64, author: &str) -> MessageRow {
    MessageRow {
        id: id.to_string(),
        conversation: "conv-1".to_string(),
        author: author.to_string(),
        direction: Direction::Incoming,
        sender_timestamp_ms: ts,
        content_type: "text/plain".to_string(),
        body_encoding: BodyEncoding::Utf8,
        body: Some("hello".to_string()),
        state: StoredState::Delivered,
        last_error: None,
        deleted_at_secs: None,
        stored_at_secs: 100,
    }
}

#[test]
fn name_validation_rules() {
    assert_eq!(validate_group_name(""), Err(GroupNameError::Empty));
    assert_eq!(validate_group_name("   "), Err(GroupNameError::Empty));
    assert_eq!(validate_group_name("  trimmed  ").unwrap(), "trimmed");

    let exact_80 = "a".repeat(80);
    assert_eq!(validate_group_name(&exact_80).unwrap(), exact_80);

    let too_long = "a".repeat(81);
    assert_eq!(validate_group_name(&too_long), Err(GroupNameError::TooLong));

    assert_eq!(validate_group_name("bell\u{0007}"), Err(GroupNameError::ControlCharacter));
    assert_eq!(validate_group_name("new\nline"), Err(GroupNameError::ControlCharacter));
    assert_eq!(validate_group_name("tab\tchar"), Err(GroupNameError::ControlCharacter));

    // Multi-byte Unicode: each emoji counts as one char.
    let emojis_80 = "🎉".repeat(80);
    assert_eq!(validate_group_name(&emojis_80).unwrap(), emojis_80);
    let emojis_81 = "🎉".repeat(81);
    assert_eq!(validate_group_name(&emojis_81), Err(GroupNameError::TooLong));
}

#[test]
fn strict_group_profile_parse() {
    let body = group_profile_body("Garden Group");
    assert_eq!(parse_group_profile(&body).unwrap(), "Garden Group");

    // Invalid JSON
    assert!(matches!(parse_group_profile(b"not json"), Err(GroupProfileError::Json(_))));

    // Extra key
    let extra = br#"{"name":"Group","version":1,"extra":"nope"}"#;
    assert_eq!(parse_group_profile(extra).unwrap_err(), GroupProfileError::Shape);

    // Missing key
    let missing = br#"{"name":"Group"}"#;
    assert_eq!(parse_group_profile(missing).unwrap_err(), GroupProfileError::Shape);

    // Wrong version
    let v2 = br#"{"name":"Group","version":2}"#;
    assert_eq!(parse_group_profile(v2).unwrap_err(), GroupProfileError::Shape);

    // Number name
    let num_name = br#"{"name":123,"version":1}"#;
    assert_eq!(parse_group_profile(num_name).unwrap_err(), GroupProfileError::Shape);

    // Empty name
    let empty_name = br#"{"name":"","version":1}"#;
    assert_eq!(
        parse_group_profile(empty_name).unwrap_err(),
        GroupProfileError::Name(GroupNameError::Empty)
    );
}

#[test]
fn apply_group_profile_order_independence_and_tie_breaking() {
    let mut meta = GroupMeta {
        name: None,
        name_source: None,
        admission: GroupAdmission::Shown,
        membership_events_copied: 0,
    };

    let src_a = NameSource {
        sender_timestamp_ms: 100,
        author: "did:key:zA".to_string(),
        message_id: "m-1".to_string(),
    };
    let src_b = NameSource {
        sender_timestamp_ms: 200,
        author: "did:key:zB".to_string(),
        message_id: "m-2".to_string(),
    };

    assert!(apply_group_profile(&mut meta, "Group A".to_string(), src_a.clone()));
    assert_eq!(meta.name.as_deref(), Some("Group A"));

    // Applying older profile B is ignored
    let mut meta_newer = meta.clone();
    let src_older = NameSource {
        sender_timestamp_ms: 50,
        author: "did:key:zC".to_string(),
        message_id: "m-0".to_string(),
    };
    assert!(!apply_group_profile(&mut meta_newer, "Older Group".to_string(), src_older));
    assert_eq!(meta_newer.name.as_deref(), Some("Group A"));

    // Applying newer profile B succeeds
    assert!(apply_group_profile(&mut meta, "Group B".to_string(), src_b.clone()));
    assert_eq!(meta.name.as_deref(), Some("Group B"));

    // Applying in reverse order (B then A) produces identical final state
    let mut meta_reverse = GroupMeta {
        name: None,
        name_source: None,
        admission: GroupAdmission::Shown,
        membership_events_copied: 0,
    };
    assert!(apply_group_profile(&mut meta_reverse, "Group B".to_string(), src_b.clone()));
    assert!(!apply_group_profile(&mut meta_reverse, "Group A".to_string(), src_a));
    assert_eq!(meta.name, meta_reverse.name);
    assert_eq!(meta.name_source, meta_reverse.name_source);

    // Ties break by author then message_id
    let tie_author_a = NameSource {
        sender_timestamp_ms: 300,
        author: "did:key:zA".to_string(),
        message_id: "m-3".to_string(),
    };
    let tie_author_b = NameSource {
        sender_timestamp_ms: 300,
        author: "did:key:zB".to_string(),
        message_id: "m-3".to_string(),
    };
    let mut tie_meta = meta.clone();
    assert!(apply_group_profile(&mut tie_meta, "Tie A".to_string(), tie_author_a));
    assert!(apply_group_profile(&mut tie_meta, "Tie B".to_string(), tie_author_b));
    assert_eq!(tie_meta.name.as_deref(), Some("Tie B"));
}

#[test]
fn admission_for_new_group_table() {
    assert_eq!(admission_for_new_group(true, None), GroupAdmission::Shown);
    assert_eq!(admission_for_new_group(true, Some("blocked")), GroupAdmission::Shown);
    assert_eq!(admission_for_new_group(false, Some("allow")), GroupAdmission::Shown);
    assert_eq!(
        admission_for_new_group(false, Some("blocked")),
        GroupAdmission::Refused { reason: "blocked".to_string() }
    );
    assert_eq!(
        admission_for_new_group(false, Some("rate-limited")),
        GroupAdmission::Refused { reason: "rate-limited".to_string() }
    );
    assert_eq!(
        admission_for_new_group(false, None),
        GroupAdmission::Refused { reason: "rate-limited".to_string() }
    );
}

#[test]
fn transcript_digest_properties() {
    let r1 = make_test_row("msg-1", 100, "did:key:z1");
    let r2 = make_test_row("msg-2", 200, "did:key:z2");
    let r3 = make_test_row("msg-3", 300, "did:key:z3");

    // Order independence
    let digest_forward = transcript_digest(&[r1.clone(), r2.clone(), r3.clone()]).unwrap();
    let digest_backward = transcript_digest(&[r3.clone(), r1.clone(), r2.clone()]).unwrap();
    assert_eq!(digest_forward, digest_backward);

    // Changes when a row is added
    let digest_two = transcript_digest(&[r1.clone(), r2.clone()]).unwrap();
    assert_ne!(digest_two, digest_forward);

    // Does not change when a row is tombstoned
    let mut r1_tombstoned = r1.clone();
    r1_tombstoned.tombstone(999);
    let digest_tombstoned = transcript_digest(&[r1_tombstoned, r2.clone(), r3.clone()]).unwrap();
    assert_eq!(digest_forward, digest_tombstoned);
}

#[test]
fn notice_constants_match_hub_ui_verbatim() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir.join("../roym_web/ui/src/groups/words.ts");
    assert!(path.exists(), "missing ../roym_web/ui/src/groups/words.ts");
    let content = fs::read_to_string(&path).unwrap();

    assert!(content.contains(OWNER_CAN_READ_NOTICE), "OWNER_CAN_READ_NOTICE missing");
    assert!(content.contains(GROUP_KEY_TRUST_NOTICE), "GROUP_KEY_TRUST_NOTICE missing");
    assert!(content.contains(GROUP_DELIVERY_NOTICE), "GROUP_DELIVERY_NOTICE missing");
    assert!(content.contains(GROUP_JOIN_BOUNDARY_NOTICE), "GROUP_JOIN_BOUNDARY_NOTICE missing");
    assert!(content.contains(GROUP_REMOVED_NOTICE), "GROUP_REMOVED_NOTICE missing");
    assert!(content.contains(GROUP_RESTORED_NOTICE), "GROUP_RESTORED_NOTICE missing");
    assert!(
        content.contains(GROUP_ADD_UNREACHABLE_MESSAGE),
        "GROUP_ADD_UNREACHABLE_MESSAGE missing"
    );
    assert!(content.contains(GROUP_HIDDEN_NOTICE), "GROUP_HIDDEN_NOTICE missing");
    assert!(content.contains(TRANSCRIPT_CHECK_NOTICE), "TRANSCRIPT_CHECK_NOTICE missing");
    assert!(content.contains(CARDS_NOT_IN_GROUPS_MESSAGE), "CARDS_NOT_IN_GROUPS_MESSAGE missing");
}

#[test]
fn system_type_predicate() {
    assert!(is_group_system_type(GROUP_PROFILE_CONTENT_TYPE));
    assert!(is_group_system_type(MEMBERSHIP_EVENT_CONTENT_TYPE));
    assert!(!is_group_system_type("text/plain"));
    assert!(!is_group_system_type("application/vnd.roym.card+json"));
}
