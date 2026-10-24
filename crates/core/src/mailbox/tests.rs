use std::{collections::HashSet, str::FromStr};

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_mailbox_format::{DepositId, Receipt, SealedBytes};

use super::*;
use crate::dht_registry::{EndpointInfo, EndpointType};

#[test]
fn wire_shapes_are_fixed() {
    let id_str = "0123456789abcdef0123456789abcdef";
    let deposit_id = DepositId::from_str(id_str).unwrap();
    let encoded_sealed = "aGVsbG8gd29ybGQ=".to_string();
    let sealed = SealedBytes::try_from(encoded_sealed.clone()).unwrap();

    let claim_answer = ClaimAnswer {
        messages: vec![ClaimedMessage {
            sender: "did:key:z6Mksender".to_string(),
            deposit_id,
            sealed,
            deposited_at_secs: 100,
            expires_at_secs: 200,
            claim_count: 1,
        }],
        visible_again_at_secs: 160,
    };
    let claim_json = serde_json::to_value(&claim_answer).unwrap();
    assert_eq!(
        claim_json,
        json!({
            "messages": [{
                "sender": "did:key:z6Mksender",
                "deposit_id": id_str,
                "sealed": encoded_sealed,
                "deposited_at_secs": 100,
                "expires_at_secs": 200,
                "claim_count": 1
            }],
            "visible_again_at_secs": 160
        })
    );

    let ack_params = AckParams {
        acks: vec![AckEntry {
            sender: "did:key:z6Mksender".to_string(),
            deposit_id,
            receipt: Receipt::Stored { at_secs: 100 },
        }],
    };
    let ack_params_json = serde_json::to_value(&ack_params).unwrap();
    assert_eq!(
        ack_params_json,
        json!({
            "acks": [{
                "sender": "did:key:z6Mksender",
                "deposit_id": id_str,
                "receipt": {
                    "state": "stored",
                    "at_secs": 100
                }
            }]
        })
    );

    let ack_answer =
        AckAnswer { results: vec![AckOutcome::Stored, AckOutcome::NotFound, AckOutcome::Refused] };
    let ack_answer_json = serde_json::to_value(&ack_answer).unwrap();
    assert_eq!(
        ack_answer_json,
        json!({
            "results": ["stored", "not_found", "refused"]
        })
    );

    let status_answer = StatusAnswer {
        receipts: vec![StatusEntry { deposit_id, receipt: Receipt::Expired { at_secs: 300 } }],
    };
    let status_json = serde_json::to_value(&status_answer).unwrap();
    assert_eq!(
        status_json,
        json!({
            "receipts": [{
                "deposit_id": id_str,
                "receipt": {
                    "state": "expired",
                    "at_secs": 300
                }
            }]
        })
    );

    let register_answer = RegisterAnswer {
        limits: QueueLimits { max_bytes: 5_000_000, max_messages: 500, max_ttl_secs: 604_800 },
        queue_ends_at_secs: 2_592_000,
    };
    let register_json = serde_json::to_value(&register_answer).unwrap();
    assert_eq!(
        register_json,
        json!({
            "limits": {
                "max_bytes": 5000000,
                "max_messages": 500,
                "max_ttl_secs": 604800
            },
            "queue_ends_at_secs": 2592000
        })
    );
}

#[test]
fn error_codes_are_distinct() {
    let all_codes = [
        codes::UNKNOWN_DESTINATION,
        codes::TOO_LARGE,
        codes::QUEUE_FULL,
        codes::RATE_LIMITED,
        codes::MAILBOX_FULL,
        codes::NOT_AN_INSTANCE,
        codes::STALE_INSTANCE,
        codes::BAD_RECORD,
        codes::DUPLICATE_DEPOSIT,
        codes::UNAUTHENTICATED,
        codes::TTL_TOO_LONG,
    ];
    let unique: HashSet<i32> = all_codes.iter().copied().collect();
    assert_eq!(unique.len(), 11);
}

#[test]
fn unknown_fields_are_ignored() {
    let claim_raw = json!({
        "max": 10,
        "unexpected_future_field": "ignored"
    });
    let claim_params: ClaimParams = serde_json::from_value(claim_raw).unwrap();
    assert_eq!(claim_params.max, Some(10));

    let identity = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&identity.public_key());
    let info = EndpointInfo {
        service_id: did.clone(),
        substrate_id: did.clone(),
        endpoint_type: EndpointType::Service,
        mechanisms: Vec::new(),
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: 100_000,
        generation: 0,
        mailbox: None,
    };
    let signed_record = info.sign(&identity).unwrap();

    let register_raw = json!({
        "record": signed_record,
        "unknown_extra_param": 42
    });
    let reg_params: RegisterParams = serde_json::from_value(register_raw).unwrap();
    assert_eq!(reg_params.record.info.service_id, did);
}

fn hosted(methods: &[&str]) -> HostedMailbox {
    HostedMailbox {
        mailbox: "did:key:zBox".to_string(),
        max_ttl_secs: 100,
        methods: methods.iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn the_conversation_methods_are_always_accepted() {
    let none = hosted(&[]);
    assert!(none.accepts("conversation", "deliver"));
    assert!(none.accepts("conversation", "group-push"));
    assert!(!none.accepts("conversation", "send"));
}

#[test]
fn only_a_listed_method_is_accepted() {
    let listed = hosted(&["greeter#greet"]);
    assert!(listed.accepts("greeter", "greet"));
    assert!(!listed.accepts("greeter", "other"));
    assert!(!listed.accepts("other", "greet"));
}

#[test]
fn an_entry_that_names_the_interface_by_its_short_hash_is_accepted() {
    let entry = format!("{}#greet", util::short_hash("greeter"));
    let listed = hosted(&[entry.as_str()]);
    assert!(listed.accepts("greeter", "greet"));
    assert!(!listed.accepts("greeter", "other"));
}

#[test]
fn node_and_platform_interfaces_are_reserved_by_name_and_by_short_hash() {
    let reserved =
        ["orchestrator", "security", "supervisor", "mailbox", "data-layer", "conversation"];
    for name in reserved {
        assert!(is_reserved_mailbox_interface(name), "{name}");
        assert!(is_reserved_mailbox_interface(&util::short_hash(name)), "{name} hash");
    }
    for name in NATIVE_CAPABILITY_INTERFACES {
        assert!(is_reserved_mailbox_interface(name), "{name}");
    }
    assert!(!is_reserved_mailbox_interface("greeter"));
}

#[test]
fn a_hosted_mailbox_without_methods_reads_as_empty() {
    let parsed: HostedMailbox =
        serde_json::from_value(json!({"mailbox": "did:key:zBox", "max_ttl_secs": 100})).unwrap();
    assert_eq!(parsed, hosted(&[]));
}
