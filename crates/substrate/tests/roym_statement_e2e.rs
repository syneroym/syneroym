#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! Two-node end-to-end tests for agreement close, event log, and statement
//! export.

use std::time::Duration;

use rustls::crypto::ring;
use serde_json::json;

mod common;

use common::{
    roym::{RoymNode as Node, roym_artifacts_present, wait_delivered, wait_until},
    roym_flow::{
        MixedFixture, accept_quote, setup_mixed_fixture, wait_and_verify_winner_scheduled,
    },
};

async fn wait_for_close_delivery(node: &Node, res: &serde_json::Value) {
    if let Some(records) = res["records"].as_array() {
        for r in records {
            if let Some(msg) = r["message_id"].as_str() {
                assert!(wait_delivered(node, msg).await, "record message delivered");
            }
        }
    }
}

async fn quote_three_commitments(node_y: &Node, req_x: &str) -> (String, String) {
    let quote_res = node_y
        .rpc_ok(
            "quote.set",
            json!({
                "enquiry_record_id": req_x,
                "expires_in_secs": 3600,
                "terms": {
                    "currency": "EUR",
                    "commitments": [
                        {
                            "giver": "quote-author",
                            "resource_kind": "goods-or-service",
                            "kind": "fulfilment",
                            "after": [],
                            "description": { "lang": "en", "text": "Gardening lesson" },
                            "declared_value_minor": 10000
                        },
                        {
                            "giver": "counterparty",
                            "resource_kind": "goods-or-service",
                            "kind": "fulfilment",
                            "after": [],
                            "description": { "lang": "en", "text": "Timber materials" },
                            "declared_value_minor": 6000
                        },
                        {
                            "giver": "counterparty",
                            "resource_kind": "currency",
                            "kind": "currency",
                            "after": [],
                            "description": { "lang": "en", "text": "EUR 40.00" },
                            "declared_value_minor": 4000,
                            "payee": "Bob",
                            "payment_methods": ["card"]
                        }
                    ],
                    "scope": { "lang": "en", "text": "Lesson in exchange for timber and cash" },
                    "cancellation_terms": { "lang": "en", "text": "Mutual consent" },
                    "refund_terms": { "lang": "en", "text": "Standard refund" },
                    "dispute_path": { "lang": "en", "text": "Mediation" }
                }
            }),
        )
        .await;
    let q_msg = quote_res["message_id"].as_str().unwrap().to_string();
    let quote_rec_id = quote_res["record_id"].as_str().unwrap().to_string();
    (quote_rec_id, q_msg)
}

async fn verify_statement_parity(node_x: &Node, node_y: &Node) {
    let x_stmt = node_x
        .rpc_ok("statement.export", json!({ "from_secs": 0, "to_secs": 2_000_000_000 }))
        .await;
    let y_stmt = node_y
        .rpc_ok("statement.export", json!({ "from_secs": 0, "to_secs": 2_000_000_000 }))
        .await;

    let x_rows = x_stmt["rows"].as_array().expect("Alice rows");
    let y_rows = y_stmt["rows"].as_array().expect("Bob rows");
    assert_eq!(x_rows.len(), 3);
    assert_eq!(y_rows.len(), 3);

    for cid in ["c1", "c2", "c3"] {
        let rx = x_rows.iter().find(|r| r["commitment_id"] == cid).unwrap();
        let ry = y_rows.iter().find(|r| r["commitment_id"] == cid).unwrap();
        assert_eq!(rx["settled_at_secs"], ry["settled_at_secs"]);
        assert_eq!(rx["declared_value_minor"], ry["declared_value_minor"]);
        assert_eq!(rx["currency"], ry["currency"]);

        let dx = rx["direction"].as_str().unwrap();
        let dy = ry["direction"].as_str().unwrap();
        match dx {
            "given" => assert_eq!(dy, "received"),
            "received" => assert_eq!(dy, "given"),
            other => panic!("unexpected direction {other}"),
        }
    }
}

#[tokio::test]
async fn both_sides_close_a_three_commitment_deal() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let MixedFixture { _dir_x, _dir_y, node_x, node_y, conv_x, conv_y, req_x, .. } =
        setup_mixed_fixture().await;

    let (quote_rec_id, q_msg) = quote_three_commitments(&node_y, &req_x).await;
    assert!(wait_delivered(&node_y, &q_msg).await, "quote delivered");

    accept_quote(&node_x, &conv_x, &quote_rec_id).await;
    node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;

    // Both parties close their sides of the agreement
    let y_close_giver = node_y
        .rpc_ok(
            "agreement.close",
            json!({ "agreement": quote_rec_id, "as": "giver", "commitments": ["c1"] }),
        )
        .await;
    wait_for_close_delivery(&node_y, &y_close_giver).await;

    let y_close_receiver = node_y
        .rpc_ok(
            "agreement.close",
            json!({ "agreement": quote_rec_id, "as": "receiver", "commitments": ["c2", "c3"] }),
        )
        .await;
    wait_for_close_delivery(&node_y, &y_close_receiver).await;

    let x_close_receiver = node_x
        .rpc_ok(
            "agreement.close",
            json!({ "agreement": quote_rec_id, "as": "receiver", "commitments": ["c1"] }),
        )
        .await;
    wait_for_close_delivery(&node_x, &x_close_receiver).await;

    let x_close_giver = node_x
        .rpc_ok(
            "agreement.close",
            json!({ "agreement": quote_rec_id, "as": "giver", "commitments": ["c2", "c3"] }),
        )
        .await;
    wait_for_close_delivery(&node_x, &x_close_giver).await;

    // Two-way sync to complete agreement
    let x_closed = wait_until(Duration::from_secs(30), || async {
        node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;
        let a = node_x.rpc_ok("agreement.get", json!({ "quote_record_id": quote_rec_id })).await;
        a["state"] == "closed"
    })
    .await;
    assert!(x_closed, "Alice agreement reaches closed");

    let y_closed = wait_until(Duration::from_secs(30), || async {
        node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;
        let a = node_y.rpc_ok("agreement.get", json!({ "quote_record_id": quote_rec_id })).await;
        a["state"] == "closed"
    })
    .await;
    assert!(y_closed, "Bob agreement reaches closed");

    verify_statement_parity(&node_x, &node_y).await;
}

#[tokio::test]
async fn the_driver_logs_with_no_hub() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let MixedFixture { _dir_x, _dir_y, node_x, node_y, conv_x, conv_y, req_x, owner_y_did } =
        setup_mixed_fixture().await;

    let quote_res = node_y
        .rpc_ok(
            "quote.set",
            json!({
                "enquiry_record_id": req_x,
                "expires_in_secs": 3600,
                "terms": {
                    "currency": "EUR",
                    "commitments": [
                        {
                            "giver": "counterparty",
                            "resource_kind": "goods-or-service",
                            "kind": "fulfilment",
                            "after": [],
                            "description": { "lang": "en", "text": "Task A" },
                            "declared_value_minor": 10000
                        }
                    ],
                    "scope": { "lang": "en", "text": "Single task" },
                    "cancellation_terms": { "lang": "en", "text": "Mutual consent" },
                    "refund_terms": { "lang": "en", "text": "Standard refund" },
                    "dispute_path": { "lang": "en", "text": "Mediation" }
                }
            }),
        )
        .await;

    let q_msg = quote_res["message_id"].as_str().unwrap();
    let quote_rec_id = quote_res["record_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(&node_y, q_msg).await, "quote delivered");

    accept_quote(&node_x, &conv_x, &quote_rec_id).await;
    node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;
    wait_and_verify_winner_scheduled(
        &node_y,
        &owner_y_did,
        &node_x,
        &quote_rec_id,
        &conv_x,
        &conv_y,
    )
    .await;

    // Alice signs fulfilment as giver
    let x_sign = node_x
        .rpc_ok("fulfilment.sign", json!({ "agreement": quote_rec_id, "commitment_id": "c1" }))
        .await;
    let x_sign_msg = x_sign["message_id"].as_str().unwrap();
    assert!(wait_delivered(&node_x, x_sign_msg).await, "Alice fulfilment delivered");

    // Bob signs fulfilment as receiver
    let y_sign = node_y
        .rpc_ok("fulfilment.sign", json!({ "agreement": quote_rec_id, "commitment_id": "c1" }))
        .await;
    let y_sign_msg = y_sign["message_id"].as_str().unwrap();
    assert!(wait_delivered(&node_y, y_sign_msg).await, "fulfilment card delivered");

    // Without calling transaction.sync on node_x, verify the driver wakes up and
    // logs commitment-settled
    let settled_logged = wait_until(Duration::from_secs(30), || async {
        let el = node_x.rpc_ok("eventlog.list", json!({})).await;
        let entries = el["entries"].as_array();
        entries.is_some_and(|list| {
            list.iter().any(|e| {
                e["event"] == "commitment-settled"
                    && e["commitment_id"] == "c1"
                    && e["agreement"] == quote_rec_id
            })
        })
    })
    .await;

    assert!(settled_logged, "driver logged commitment-settled without manual transaction.sync");
}
