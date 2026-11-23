//! Tests for node-local transaction driver (ADR-0031).

use serde_json::{Value, json};
use syneroym_app_host::types::{events::ReadResult, proxy::ProxyError};
use syneroym_roym_core::{
    deal::{DealTerms, LocalText},
    lang::LangTag,
    progress::AgreementState,
    transaction::SYNC_WINDOW,
};

use super::fixture;
use crate::app::{
    AGREEMENTS, AgreementRow, DRIVE_PENDING, DRIVE_STATE, LEDGER,
    decide::{DecisionRow, DecisionState, decision_id},
    drive::{
        DRIVE_RESCAN_PAGE, DriveInput, DriveState, MAX_DRIVE_CONVERSATIONS, PendingRow, drive,
        drive_at,
    },
    get_row, put_row,
    sync::{SyncInput, sync_at, sync_direct},
};

fn mock_empty_changes(host: &syneroym_roym_core::testing::TestHost) {
    host.on_call("conversation", "conversation.changes", |_| {
        Ok(json!({
            "result": {
                "messages": [],
                "last_seq": 0,
                "kind": "direct"
            }
        })
        .to_string())
    });
    host.on_call("conversation", "conversation.list", |_| {
        Ok(json!({
            "result": {
                "conversations": []
            }
        })
        .to_string())
    });
}

async fn seed_drive_cursor(host: &syneroym_roym_core::testing::TestHost) {
    let state = DriveState {
        event_cursor: Some("0000000000000000-0".to_string()),
        rescan_offset: None,
        timed_cursor: 0,
    };
    put_row(host, DRIVE_STATE, "drive", &state).await.unwrap();
}

fn dummy_agreement_row(conv: &str, state: Option<AgreementState>, now: u64) -> AgreementRow {
    AgreementRow {
        quote_record_id: "rec_q_1".to_string(),
        quote_id: "q_1".to_string(),
        conversation: conv.to_string(),
        counterparty_did: "did:key:cp".to_string(),
        quote_author_did: "did:key:author".to_string(),
        terms: DealTerms {
            currency: "EUR".to_string(),
            commitments: vec![],
            home_exchange: None,
            listing_id: None,
            scope: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Test"),
            cancellation_terms: LocalText::plain(
                LangTag::parse_canonical("en").expect("lang"),
                "None",
            ),
            refund_terms: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "None"),
            dispute_path: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "None"),
            quote_expires_at_secs: now + 3600,
        },
        agreed_at_secs: Some(now),
        counterparty: None,
        quote_author: None,
        state,
        state_reason: None,
        cancel_reason: None,
        seq: Some(1),
        commitments: vec![],
        updated_at_secs: now,
    }
}

#[tokio::test]
async fn drive_syncs_each_conversation_named_by_a_message_event() {
    let f = fixture();
    mock_empty_changes(&f.host);
    seed_drive_cursor(&f.host).await;

    f.host.push_event("conversation.message", "conv_1");
    f.host.push_event("conversation.message", "conv_2");

    let out = drive(&f.host, DriveInput {}).await.expect("drive");
    assert_eq!(out.synced, 2);
    assert_eq!(out.pending, 0);
    assert!(!out.again);

    let p1: Option<PendingRow> = get_row(&f.host, DRIVE_PENDING, "conv_1").await.unwrap();
    let p2: Option<PendingRow> = get_row(&f.host, DRIVE_PENDING, "conv_2").await.unwrap();
    assert!(p1.is_none());
    assert!(p2.is_none());
}

#[tokio::test]
async fn drive_syncs_at_most_eight_conversations_and_keeps_the_rest() {
    let f = fixture();
    mock_empty_changes(&f.host);
    seed_drive_cursor(&f.host).await;

    for i in 1..=10 {
        f.host.push_event("conversation.message", format!("conv_{i}"));
    }

    let out1 = drive(&f.host, DriveInput {}).await.expect("drive 1");
    assert_eq!(out1.synced, MAX_DRIVE_CONVERSATIONS as u32);
    assert_eq!(out1.pending, 2);
    assert!(out1.again);

    let out2 = drive(&f.host, DriveInput {}).await.expect("drive 2");
    assert_eq!(out2.synced, 2);
    assert_eq!(out2.pending, 0);
    assert!(!out2.again);
}

#[tokio::test]
async fn drive_skips_a_group_conversation_without_an_error() {
    let f = fixture();
    seed_drive_cursor(&f.host).await;
    f.host.on_call("conversation", "conversation.changes", |_| {
        Ok(json!({
            "result": {
                "messages": [],
                "last_seq": 0,
                "kind": "group"
            }
        })
        .to_string())
    });

    f.host.push_event("conversation.message", "grp_conv");

    let out = drive(&f.host, DriveInput {}).await.expect("drive");
    assert_eq!(out.synced, 0);
    assert_eq!(out.pending, 0);
    assert!(out.warnings.is_empty());

    let p: Option<PendingRow> = get_row(&f.host, DRIVE_PENDING, "grp_conv").await.unwrap();
    assert!(p.is_none());
}

#[tokio::test]
async fn drive_after_reload_walks_every_conversation_in_pages() {
    let f = fixture();
    mock_empty_changes(&f.host);

    f.host.on_call("conversation", "conversation.list", |params| {
        let p: Value = serde_json::from_str(&params).unwrap_or(Value::Null);
        let offset = p.get("offset").and_then(Value::as_u64).unwrap_or(0);
        let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(16);

        let convs = if offset == 0 {
            (0..limit)
                .map(|i| json!({ "id": format!("c_{i}"), "kind": "direct" }))
                .collect::<Vec<_>>()
        } else if offset == 16 {
            (16..18)
                .map(|i| json!({ "id": format!("c_{i}"), "kind": "direct" }))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        Ok(json!({ "result": { "conversations": convs } }).to_string())
    });

    f.host.set_read_events_override(Ok(ReadResult::Reload("head_1".to_string())));

    let out1 = drive(&f.host, DriveInput {}).await.expect("drive 1");
    assert!(out1.rescanning);
    assert_eq!(out1.synced, MAX_DRIVE_CONVERSATIONS as u32);
    assert!(out1.again);

    let state: Option<DriveState> = get_row(&f.host, DRIVE_STATE, "drive").await.unwrap();
    assert_eq!(state.unwrap().rescan_offset, Some(DRIVE_RESCAN_PAGE));

    let out2 = drive(&f.host, DriveInput {}).await.expect("drive 2");
    assert!(!out2.rescanning);
    assert_eq!(out2.synced, MAX_DRIVE_CONVERSATIONS as u32);

    let out3 = drive(&f.host, DriveInput {}).await.expect("drive 3");
    assert_eq!(out3.synced, 2);
    assert_eq!(out3.pending, 0);
    assert!(!out3.again);
}

#[tokio::test]
async fn drive_ignores_events_of_transaction_itself() {
    let f = fixture();
    mock_empty_changes(&f.host);
    seed_drive_cursor(&f.host).await;

    f.host.push_event("transaction.card", "conv_tx_1");
    f.host.push_event("transaction.progress", "conv_tx_2");
    f.host.push_event("conversation.message", "conv_real");

    let out = drive(&f.host, DriveInput {}).await.expect("drive");
    assert_eq!(out.synced, 1);
    assert_eq!(out.pending, 0);

    let p_tx: Option<PendingRow> = get_row(&f.host, DRIVE_PENDING, "conv_tx_1").await.unwrap();
    assert!(p_tx.is_none());
}

#[tokio::test]
async fn drive_gives_timed_work_a_sync_with_no_new_message() {
    let f = fixture();
    mock_empty_changes(&f.host);
    seed_drive_cursor(&f.host).await;

    let row = dummy_agreement_row("conv_timed", Some(AgreementState::Agreed), f.host.now());
    put_row(&f.host, AGREEMENTS, "agr_1", &row).await.unwrap();

    let out = drive(&f.host, DriveInput {}).await.expect("drive");
    assert_eq!(out.synced, 1);
    assert_eq!(out.pending, 0);
}

#[tokio::test]
async fn drive_keeps_conversation_pending_when_more_messages_in_window() {
    let f = fixture();
    seed_drive_cursor(&f.host).await;
    let mut msgs = Vec::new();
    for i in 1..=(SYNC_WINDOW as u64) {
        msgs.push(json!({
            "id": format!("m_{i}"),
            "conversation": "conv_more",
            "author": "did:key:other",
            "direction": "incoming",
            "sender_timestamp_ms": 1000,
            "content_type": "text/plain",
            "languages": ["en"],
            "body_encoding": "utf8",
            "restored": false,
            "state": "delivered",
            "stored_at_secs": 1,
            "visible_seq": i,
            "verified": true,
            "body": "hello",
        }));
    }

    f.host.on_call("conversation", "conversation.changes", move |_| {
        Ok(json!({
            "result": {
                "messages": msgs,
                "last_seq": SYNC_WINDOW as u64,
                "kind": "direct"
            }
        })
        .to_string())
    });

    f.host.push_event("conversation.message", "conv_more");

    let out = drive(&f.host, DriveInput {}).await.expect("drive");
    assert_eq!(out.synced, 1);
    assert_eq!(out.pending, 1);

    let p: Option<PendingRow> = get_row(&f.host, DRIVE_PENDING, "conv_more").await.unwrap();
    assert!(p.is_some());
}

#[tokio::test]
async fn drive_compare_and_delete_preserves_concurrent_event_mark() {
    let f = fixture();
    seed_drive_cursor(&f.host).await;
    let now = f.host.now();

    put_row(&f.host, DRIVE_PENDING, "conv_race", &PendingRow { since: now, failures: 0 })
        .await
        .unwrap();

    let host_clone = f.host.clone();
    f.host.on_call("conversation", "conversation.changes", move |_| {
        host_clone.set_storage_row(
            DRIVE_PENDING,
            "conv_race",
            &PendingRow { since: now + 50, failures: 0 },
        );
        Ok(json!({
            "result": {
                "messages": [],
                "last_seq": 0,
                "kind": "direct"
            }
        })
        .to_string())
    });

    let out = drive(&f.host, DriveInput {}).await.expect("drive");
    assert_eq!(out.synced, 1);

    let p: Option<PendingRow> = get_row(&f.host, DRIVE_PENDING, "conv_race").await.unwrap();
    assert!(p.is_some(), "row marked concurrently must be preserved");
    assert_eq!(p.unwrap().since, now + 50);
}

#[tokio::test]
async fn drive_runs_recovery_once_per_call() {
    let f = fixture();
    mock_empty_changes(&f.host);
    seed_drive_cursor(&f.host).await;

    // Seed an expired deciding agreement
    let mut agr = dummy_agreement_row("conv_dec", None, f.host.now() - 7200);
    agr.quote_record_id = "rec_q_dec".to_string();
    agr.quote_id = "q_dec".to_string();
    agr.terms.quote_expires_at_secs = f.host.now() - 3600;
    put_row(&f.host, AGREEMENTS, "rec_q_dec", &agr).await.unwrap();

    let dec = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::Deciding,
        agreement: "rec_q_dec".to_string(),
        quote_id: "q_dec".to_string(),
        reason: None,
        created_at_secs: f.host.now() - 3600,
    };
    put_row(&f.host, LEDGER, &decision_id("q_dec"), &dec).await.unwrap();

    f.host.push_event("conversation.message", "conv_1");
    f.host.push_event("conversation.message", "conv_2");

    let out = drive(&f.host, DriveInput {}).await.expect("drive");
    assert_eq!(out.synced, 2);

    // Verify recovery ran once: deciding row transitioned to NotFormed
    let dec_after: DecisionRow =
        get_row(&f.host, LEDGER, &decision_id("q_dec")).await.unwrap().unwrap();
    assert_eq!(dec_after.state, DecisionState::NotFormed);

    // Verify exactly one recovery query for deciding rows occurred
    let recovery_queries = f
        .host
        .queries()
        .iter()
        .filter(|(col, opts)| {
            col == LEDGER && opts.filter.as_deref().unwrap_or("").contains("decision")
        })
        .count();
    assert_eq!(recovery_queries, 1);
}

#[tokio::test]
async fn sync_still_runs_recovery_once() {
    let f = fixture();
    mock_empty_changes(&f.host);

    // Seed an expired deciding agreement
    let mut agr = dummy_agreement_row("conv_dec", None, f.host.now() - 7200);
    agr.quote_record_id = "rec_q_dec".to_string();
    agr.quote_id = "q_dec".to_string();
    agr.terms.quote_expires_at_secs = f.host.now() - 3600;
    put_row(&f.host, AGREEMENTS, "rec_q_dec", &agr).await.unwrap();

    let dec = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::Deciding,
        agreement: "rec_q_dec".to_string(),
        quote_id: "q_dec".to_string(),
        reason: None,
        created_at_secs: f.host.now() - 3600,
    };
    put_row(&f.host, LEDGER, &decision_id("q_dec"), &dec).await.unwrap();

    let res = sync_at(
        &f.host,
        SyncInput { conversation: "conv_single".to_string(), full: false },
        f.host.now(),
    )
    .await
    .expect("sync_at");
    assert_eq!(res.scanned, 0);

    // Verify recovery ran once: deciding row transitioned to NotFormed
    let dec_after: DecisionRow =
        get_row(&f.host, LEDGER, &decision_id("q_dec")).await.unwrap().unwrap();
    assert_eq!(dec_after.state, DecisionState::NotFormed);

    let recovery_queries = f
        .host
        .queries()
        .iter()
        .filter(|(col, opts)| {
            col == LEDGER && opts.filter.as_deref().unwrap_or("").contains("decision")
        })
        .count();
    assert_eq!(recovery_queries, 1);
}

#[tokio::test]
async fn a_failing_conversation_does_not_starve_the_others() {
    let f = fixture();
    seed_drive_cursor(&f.host).await;
    f.host.on_call("conversation", "conversation.changes", |params| {
        let p: Value = serde_json::from_str(&params).unwrap_or(Value::Null);
        let conv = p.get("conversation").and_then(Value::as_str).unwrap_or("");
        if conv == "conv_bad" {
            Err(ProxyError::Transport("conv deleted".to_string()))
        } else {
            Ok(json!({
                "result": {
                    "messages": [],
                    "last_seq": 0,
                    "kind": "direct"
                }
            })
            .to_string())
        }
    });

    put_row(&f.host, DRIVE_PENDING, "conv_bad", &PendingRow { since: 100, failures: 0 })
        .await
        .unwrap();
    put_row(&f.host, DRIVE_PENDING, "conv_good", &PendingRow { since: 101, failures: 0 })
        .await
        .unwrap();

    let out = drive_at(&f.host, DriveInput {}, 200).await.expect("drive");
    assert_eq!(out.synced, 1);
    assert_eq!(out.pending, 1);

    let bad_row: Option<PendingRow> = get_row(&f.host, DRIVE_PENDING, "conv_bad").await.unwrap();
    assert_eq!(bad_row.as_ref().unwrap().failures, 1);
    assert_eq!(bad_row.as_ref().unwrap().since, 200);

    let good_row: Option<PendingRow> = get_row(&f.host, DRIVE_PENDING, "conv_good").await.unwrap();
    assert!(good_row.is_none());
}

#[tokio::test]
async fn a_failing_row_is_dropped_after_five_failures_with_a_warning() {
    let f = fixture();
    seed_drive_cursor(&f.host).await;
    f.host.on_call("conversation", "conversation.changes", |_| {
        Err(ProxyError::Transport("fail".to_string()))
    });

    put_row(&f.host, DRIVE_PENDING, "conv_drop", &PendingRow { since: 100, failures: 4 })
        .await
        .unwrap();

    let out = drive_at(&f.host, DriveInput {}, 200).await.expect("drive");
    assert_eq!(out.synced, 0);
    assert_eq!(out.pending, 0);
    assert_eq!(out.warnings.len(), 1);
    assert!(out.warnings[0].contains("gave up on conv_drop"));

    let row: Option<PendingRow> = get_row(&f.host, DRIVE_PENDING, "conv_drop").await.unwrap();
    assert!(row.is_none());
}

#[tokio::test]
async fn a_failing_row_does_not_set_again() {
    let f = fixture();
    seed_drive_cursor(&f.host).await;
    f.host.on_call("conversation", "conversation.changes", |_| {
        Err(ProxyError::Transport("fail".to_string()))
    });

    for i in 1..=10 {
        put_row(
            &f.host,
            DRIVE_PENDING,
            &format!("conv_f_{i}"),
            &PendingRow { since: i as u64, failures: 0 },
        )
        .await
        .unwrap();
    }

    let out = drive(&f.host, DriveInput {}).await.expect("drive");
    assert_eq!(out.synced, 0);
    assert_eq!(out.pending, 10);
    assert!(!out.again);
}

#[tokio::test]
async fn drive_initial_poll_without_cursor_captures_head_and_starts_rescan() {
    let f = fixture();
    mock_empty_changes(&f.host);

    f.host.push_event("conversation.message", "conv_early");

    let out = drive(&f.host, DriveInput {}).await.expect("drive");
    assert_eq!(out.synced, 0);
    assert_eq!(out.pending, 0);
    assert!(!out.rescanning);

    let state: Option<DriveState> = get_row(&f.host, DRIVE_STATE, "drive").await.unwrap();
    let state = state.expect("drive state saved");
    assert_eq!(state.event_cursor, Some("0000000000000000-1".to_string()));
    assert_eq!(state.rescan_offset, None);
}

#[tokio::test]
async fn more_is_false_when_last_seq_does_not_move() {
    let f = fixture();
    let mut msgs = Vec::new();
    for i in 1..=(SYNC_WINDOW as u64) {
        msgs.push(json!({
            "id": format!("m_{i}"),
            "conversation": "conv_stuck",
            "author": "did:key:other",
            "direction": "incoming",
            "sender_timestamp_ms": 1000,
            "content_type": "text/plain",
            "languages": ["en"],
            "body_encoding": "utf8",
            "restored": false,
            "state": "delivered",
            "stored_at_secs": 1,
            "visible_seq": i,
            "verified": false,
            "body": "invalid",
        }));
    }

    f.host.on_call("conversation", "conversation.changes", move |_| {
        Ok(json!({
            "result": {
                "messages": msgs,
                "last_seq": 0,
                "kind": "direct"
            }
        })
        .to_string())
    });

    let res = sync_direct(&f.host, "conv_stuck", false, f.host.now()).await.expect("sync_direct");
    assert_eq!(res.scanned, SYNC_WINDOW);
    assert!(!res.more);
}
