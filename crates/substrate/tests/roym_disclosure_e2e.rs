#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! Deal presentation tests across three independent substrate nodes:
//! verifying agreements and quotes with chosen disclosures to a third party.

use rustls::crypto::ring;
use serde_json::json;
use syneroym_identity::Identity;
use tempfile::TempDir;

mod common;

use common::{
    roym::{self, RoymNode},
    roym_flow,
};

struct ThreeNodeFixture {
    _dir_bob: TempDir,
    _dir_alice: TempDir,
    _dir_carol: TempDir,
    bob: RoymNode,
    alice: RoymNode,
    carol: RoymNode,
    conv_bob: String,
    conv_alice: String,
    req_rec_id: String,
}

async fn setup_three_nodes() -> Option<ThreeNodeFixture> {
    if !roym::roym_artifacts_present() {
        eprintln!("Skipping: Roym WASM components or UI bundle not built");
        return None;
    }
    let _ = ring::default_provider().install_default();

    let dir_bob = tempfile::tempdir().unwrap();
    let dir_alice = tempfile::tempdir().unwrap();
    let dir_carol = tempfile::tempdir().unwrap();

    let owner_bob = Identity::generate().unwrap();
    let owner_alice = Identity::generate().unwrap();
    let owner_carol = Identity::generate().unwrap();

    let bob = RoymNode::boot_ready(
        "node-bob",
        dir_bob.path().to_path_buf(),
        None,
        Identity::from_bytes(&owner_bob.to_bytes()),
        roym::fast_conversation_role(3600),
    )
    .await;

    let shared_registry = Some(bob.registry_url.clone());

    let alice = RoymNode::boot_ready(
        "node-alice",
        dir_alice.path().to_path_buf(),
        shared_registry.clone(),
        Identity::from_bytes(&owner_alice.to_bytes()),
        roym::fast_conversation_role(3600),
    )
    .await;

    let carol = RoymNode::boot_ready(
        "node-carol",
        dir_carol.path().to_path_buf(),
        shared_registry,
        Identity::from_bytes(&owner_carol.to_bytes()),
        roym::fast_conversation_role(3600),
    )
    .await;

    let bob_conv_did = bob.dids["conversation"].clone();
    let alice_conv_did = alice.dids["conversation"].clone();
    let carol_conv_did = carol.dids["conversation"].clone();

    bob.rpc_ok(
        "profile.set",
        json!({ "display_name": "Bob", "conversation_address": bob_conv_did, "lang": "en" }),
    )
    .await;
    alice
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "Alice", "conversation_address": alice_conv_did, "lang": "en" }),
        )
        .await;
    carol
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "Carol", "conversation_address": carol_conv_did, "lang": "en" }),
        )
        .await;

    let (conv_alice, _) =
        roym_flow::open_request_conv(&alice, &bob_conv_did, "Clear garden waste", &["gardening"])
            .await;
    let conv_bob = roym_flow::quote_author_conv_for(&bob, &alice_conv_did).await;
    let req_rec_id = roym_flow::request_record_on_quote_author(&bob, &conv_bob).await;

    Some(ThreeNodeFixture {
        _dir_bob: dir_bob,
        _dir_alice: dir_alice,
        _dir_carol: dir_carol,
        bob,
        alice,
        carol,
        conv_bob,
        conv_alice,
        req_rec_id,
    })
}

#[tokio::test]
async fn a_third_node_verifies_an_agreement_with_the_price_hidden() {
    let _guard = common::serial_guard().await;
    let Some(f) = setup_three_nodes().await else {
        return;
    };

    let terms = roym_flow::quote_terms("Bob Services", 50_000);
    let quote_params = json!({
        "enquiry_record_id": f.req_rec_id,
        "expires_in_secs": 3600,
        "terms": terms,
    });
    let quote_rec_id = roym_flow::send_quote_with(&f.bob, quote_params).await;

    roym_flow::accept_quote(&f.alice, &f.conv_alice, &quote_rec_id).await;

    f.bob.rpc_ok("transaction.sync", json!({ "conversation": f.conv_bob })).await;
    f.alice.rpc_ok("transaction.sync", json!({ "conversation": f.conv_alice })).await;

    let pres = f
        .bob
        .rpc_ok(
            "deal.present",
            json!({
                "record_id": quote_rec_id,
                "show": ["terms.scope"],
            }),
        )
        .await;

    let view = f.carol.rpc_ok("deal.verify-presentation", pres).await;

    let records = view["records"].as_array().expect("records array");
    assert_eq!(records.len(), 3, "must include quote and agreement halves");
    for r in records {
        assert_eq!(r["verified"], true, "every presentation record must verify");
    }

    assert_eq!(
        view["terms"]["scope"]["text"], "Clear garden waste and green bin collection",
        "scope must be shown"
    );

    let hidden = view["hidden"].as_array().expect("hidden array");
    assert!(
        hidden.iter().any(|h| h == "terms.commitments[c1].declared_value_minor"),
        "c1 declared value must be hidden"
    );
    assert!(
        hidden.iter().any(|h| h == "terms.commitments[c2].declared_value_minor"),
        "c2 declared value must be hidden"
    );
    if let Some(comms) = view["terms"].get("commitments").and_then(|v| v.as_array()) {
        for c in comms {
            assert!(
                c.get("declared_value_minor").is_none(),
                "declared value must not appear in shown terms"
            );
        }
    }
}

#[tokio::test]
async fn a_third_node_verifies_a_quote_before_any_agreement() {
    let _guard = common::serial_guard().await;
    let Some(f) = setup_three_nodes().await else {
        return;
    };

    let terms = roym_flow::quote_terms("Bob Services", 75_000);
    let quote_params = json!({
        "enquiry_record_id": f.req_rec_id,
        "expires_in_secs": 3600,
        "terms": terms,
    });
    let quote_rec_id = roym_flow::send_quote_with(&f.bob, quote_params).await;

    let pres = f
        .bob
        .rpc_ok(
            "deal.present",
            json!({
                "record_id": quote_rec_id,
                "show": ["terms.scope"],
            }),
        )
        .await;

    let view = f.carol.rpc_ok("deal.verify-presentation", pres).await;

    let records = view["records"].as_array().expect("records array");
    assert_eq!(records.len(), 1, "only the quote record is present");
    assert_eq!(records[0]["verified"], true, "quote record must verify");

    assert_eq!(
        view["terms"]["scope"]["text"], "Clear garden waste and green bin collection",
        "scope must be shown"
    );

    let hidden = view["hidden"].as_array().expect("hidden array");
    assert!(
        hidden.iter().any(|h| h == "terms.commitments[c1].declared_value_minor"),
        "c1 declared value must be hidden"
    );
    assert!(
        hidden.iter().any(|h| h == "terms.commitments[c2].declared_value_minor"),
        "c2 declared value must be hidden"
    );
}
