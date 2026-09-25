#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! The durability suite: a provider's transaction in flight is
//! carried across an encrypted backup and a restore onto a clean
//! installation, without shelling out to `roymctl` -- the same
//! `syneroym_identity::backup` calls and the same five `*.export`/`*.import`
//! verbs `roymctl roym backup` uses, called directly.

use roymctl::commands::roym::backup::{ARCHIVE_INFO, ARCHIVE_VERSION, RoymArchive, data_aad_bytes};
use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_identity::{Identity, backup as identity_backup, substrate};
use syneroym_roym_core::transaction::DEFAULT_DATA_USE_NOTICE;

mod common;

use common::roym::{
    RoymNode as Node, fast_conversation_role, roym_artifacts_present, wait_delivered,
};

const EXPORT_SERVICES: &[&str] =
    &["profile", "catalog", "conversation", "transaction", "directory"];
const IMPORT_ORDER: &[&str] = &["profile", "catalog", "conversation", "transaction", "directory"];

async fn export_bundles(node: &Node) -> serde_json::Map<String, Value> {
    let mut bundles = serde_json::Map::new();
    for svc in EXPORT_SERVICES {
        let bundle = node.rpc_ok(&format!("{svc}.export"), json!({})).await;
        bundles.insert((*svc).to_string(), bundle);
    }
    bundles
}

async fn import_bundles(node: &Node, bundles: &serde_json::Map<String, Value>) {
    for svc in IMPORT_ORDER {
        if let Some(bundle) = bundles.get(*svc) {
            let res = node.rpc(&format!("{svc}.import"), json!({ "bundle": bundle })).await;
            assert!(res.get("error").is_none(), "{svc} import: {res:?}");
        }
    }
}

/// Builds the encrypted archive exactly as `roymctl roym backup create`
/// does, without touching the filesystem or a subprocess.
fn seal_archive(
    owner: &Identity,
    subject_did: &str,
    bundles: serde_json::Map<String, Value>,
) -> (RoymArchive, [u8; 32]) {
    let recovery_key = identity_backup::generate_recovery_key().unwrap();
    let identity_part = identity_backup::export(owner, &recovery_key).unwrap();

    let payload = serde_json::to_vec(&json!({ "bundles": bundles })).unwrap();
    let now_secs = syneroym_roym_core::clock::now_secs();
    let aad = data_aad_bytes(ARCHIVE_VERSION, subject_did, now_secs);
    let sealed_data = identity_backup::seal(&payload, &recovery_key, ARCHIVE_INFO, &aad).unwrap();

    let archive = RoymArchive {
        archive_version: ARCHIVE_VERSION,
        subject_did: subject_did.to_string(),
        produced_at_secs: now_secs,
        identity: identity_part,
        data: sealed_data,
    };
    (archive, recovery_key)
}

fn open_archive_bundles(
    archive: &RoymArchive,
    recovery_key: &[u8; 32],
) -> anyhow::Result<serde_json::Map<String, Value>> {
    let aad =
        data_aad_bytes(archive.archive_version, &archive.subject_did, archive.produced_at_secs);
    let decrypted = identity_backup::open(&archive.data, recovery_key, ARCHIVE_INFO, &aad)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let data_val: Value = serde_json::from_slice(&decrypted)?;
    let bundles = data_val
        .get("bundles")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("archive missing 'bundles' mapping"))?
        .clone();
    Ok(bundles)
}

struct BootedRestoreCluster {
    node_x: Node,
    node_y: Node,
    shared_registry: Option<String>,
    listing_id: String,
    slot_id: String,
}

async fn boot_restore_cluster(
    dir_x: &std::path::Path,
    dir_y: &std::path::Path,
    owner_x: &Identity,
    owner_y: &Identity,
) -> BootedRestoreCluster {
    let mut node_x = Node::boot(
        "node-x",
        dir_x.to_path_buf(),
        None,
        Identity::from_bytes(&owner_x.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    let shared_registry = node_x.registry_url.clone();
    node_x.full_bring_up().await;

    let mut node_y = Node::boot(
        "node-y",
        dir_y.to_path_buf(),
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_y.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_y.full_bring_up().await;

    let x_conv_did = node_x.dids["conversation"].clone();
    let y_conv_did = node_y.dids["conversation"].clone();
    node_x
        .rpc_ok("profile.set", json!({ "display_name": "X", "conversation_address": x_conv_did }))
        .await;
    node_y
        .rpc_ok("profile.set", json!({ "display_name": "Y", "conversation_address": y_conv_did }))
        .await;

    let listing = node_y
        .rpc_ok(
            "listing.set",
            json!({
                "title": "Deep clean",
                "summary": "One-off deep clean",
                "categories": ["cleaning"],
                "payment": {
                    "currency": "EUR", "model": "fixed", "amount_minor": 8000,
                    "tax_included": true, "payee": "Y Cleaning"
                }
            }),
        )
        .await;
    let listing_id = listing["listing_id"].as_str().unwrap().to_string();
    let slot_start = 1_800_000_000_u64;
    let slot_end = slot_start + 3600;
    let avail = node_y
        .rpc_ok(
            "availability.set",
            json!({
                "listing_id": listing_id,
                "slots": [{ "start_secs": slot_start, "end_secs": slot_end, "capacity": 1 }],
            }),
        )
        .await;
    let slot_id = avail["slot_ids"][0].as_str().unwrap().to_string();

    BootedRestoreCluster {
        node_x,
        node_y,
        shared_registry: Some(shared_registry),
        listing_id,
        slot_id,
    }
}

struct ActiveTransaction {
    quote_record_id: String,
    req_id: String,
    quote_id: String,
    y_conv_id: String,
}

async fn setup_active_transaction(
    node_x: &Node,
    node_y: &Node,
    listing_id: &str,
    slot_id: &str,
) -> ActiveTransaction {
    let y_conv_did = node_y.dids["conversation"].clone();
    let opened = node_x.rpc_ok("conversation.open", json!({ "address": y_conv_did })).await;
    let x_conv_id = opened["conversation_id"].as_str().unwrap().to_string();
    let req = node_x
        .rpc_ok(
            "request.set",
            json!({
                "conversation": x_conv_id,
                "description": "Deep clean the kitchen",
                "categories": ["cleaning"],
                "data_use_notice": DEFAULT_DATA_USE_NOTICE,
            }),
        )
        .await;
    let req_id = req["request_id"].as_str().unwrap().to_string();
    let req_msg_id = req["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_x, &req_msg_id).await, "request delivered");

    let y_conv_id = {
        let list = node_y.rpc_ok("conversation.list", json!({})).await;
        list["conversations"][0]["id"].as_str().unwrap().to_string()
    };
    node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv_id })).await;
    let y_thread = node_y.rpc_ok("transaction.thread", json!({ "conversation": y_conv_id })).await;
    let req_record_id =
        y_thread["cards"].as_array().unwrap().iter().find(|c| c["card_type"] == "request").unwrap()
            ["record_id"]
            .as_str()
            .unwrap()
            .to_string();

    let quote = node_y
        .rpc_ok(
            "quote.set",
            json!({
                "request_record_id": req_record_id,
                "listing_id": listing_id,
                "slot_id": slot_id,
                "expires_in_secs": 3600,
                "terms": {
                    "scope": "Deep clean the kitchen, floor to ceiling",
                    "currency": "EUR", "amount_minor": 8000, "tax_minor": 0, "fees_minor": 0,
                    "payment_methods": ["cash"], "payee": "Y Cleaning",
                    "payment_timing": "after-work",
                    "location": { "where": "at-customer", "address": "1 Kitchen Way" },
                    "cancellation_terms": "24 hours notice required",
                    "refund_terms": "Full refund if work not completed",
                    "dispute_path": "Informal mediation",
                },
            }),
        )
        .await;
    let quote_msg_id = quote["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_y, &quote_msg_id).await, "quote delivered");
    let quote_record_id = quote["record_id"].as_str().unwrap().to_string();
    let quote_id = quote["quote_id"].as_str().unwrap().to_string();

    node_x.rpc_ok("transaction.sync", json!({ "conversation": x_conv_id })).await;
    let accept =
        node_x.rpc_ok("agreement.accept", json!({ "quote_record_id": quote_record_id })).await;
    let accept_msg_id = accept["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_x, &accept_msg_id).await, "accept delivered");
    node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv_id })).await;

    let booking = node_y.rpc_ok("booking.get", json!({ "agreement": quote_record_id })).await;
    assert_eq!(booking["state"], "scheduled");

    let pay_req = node_y.rpc_ok("payment.request", json!({ "agreement": quote_record_id })).await;
    let pay_req_msg = pay_req["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_y, &pay_req_msg).await, "payment request delivered");
    node_x.rpc_ok("transaction.sync", json!({ "conversation": x_conv_id })).await;

    let x_ack = node_x.rpc_ok("payment.acknowledge", json!({ "agreement": quote_record_id })).await;
    let x_ack_msg = x_ack["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_x, &x_ack_msg).await, "consumer payment ack delivered");
    node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv_id })).await;

    let y_ack1 = node_y
        .rpc_ok(
            "payment.acknowledge",
            json!({ "agreement": quote_record_id, "method": "cash", "reference": "first" }),
        )
        .await;
    let y_ack1_id = y_ack1["record_id"].as_str().unwrap().to_string();
    let y_ack_corrected = node_y
        .rpc_ok(
            "payment.acknowledge",
            json!({
                "agreement": quote_record_id, "method": "cash", "reference": "corrected",
                "supersedes": y_ack1_id,
            }),
        )
        .await;
    assert_ne!(y_ack_corrected["record_id"], y_ack1_id);

    let y_fulfil = node_y.rpc_ok("fulfilment.sign", json!({ "agreement": quote_record_id })).await;
    assert!(
        wait_delivered(node_y, y_fulfil["message_id"].as_str().unwrap()).await,
        "fulfilment sign delivered"
    );

    ActiveTransaction { quote_record_id, req_id, quote_id, y_conv_id }
}

struct BeforeSnapshot {
    booking: Value,
    history: Value,
    payment: Value,
    fulfilment: Value,
    request_history: Value,
    quote_history: Value,
    thread: Value,
    conv_history: Value,
}

async fn capture_before_snapshot(node_y: &Node, tx: &ActiveTransaction) -> BeforeSnapshot {
    let booking = node_y.rpc_ok("booking.get", json!({ "agreement": tx.quote_record_id })).await;
    let history =
        node_y.rpc_ok("booking.history", json!({ "agreement": tx.quote_record_id })).await;
    let payment = node_y.rpc_ok("payment.get", json!({ "agreement": tx.quote_record_id })).await;
    assert_eq!(payment["track"], "acknowledged");
    let fulfilment =
        node_y.rpc_ok("fulfilment.get", json!({ "agreement": tx.quote_record_id })).await;
    assert_eq!(fulfilment["track"], "claimed");
    let request_history =
        node_y.rpc_ok("request.history", json!({ "request_id": tx.req_id })).await;
    let quote_history = node_y.rpc_ok("quote.history", json!({ "quote_id": tx.quote_id })).await;
    let thread = node_y.rpc_ok("transaction.thread", json!({ "conversation": tx.y_conv_id })).await;
    let conv_history =
        node_y.rpc_ok("conversation.history", json!({ "conversation": tx.y_conv_id })).await;
    BeforeSnapshot {
        booking,
        history,
        payment,
        fulfilment,
        request_history,
        quote_history,
        thread,
        conv_history,
    }
}

async fn assert_durability_parity(node_y2: &Node, before: &BeforeSnapshot, tx: &ActiveTransaction) {
    let booking_after =
        node_y2.rpc_ok("booking.get", json!({ "agreement": tx.quote_record_id })).await;
    assert_eq!(booking_after["state"], before.booking["state"]);
    assert_eq!(booking_after["payment"], before.booking["payment"]);
    assert_eq!(booking_after["fulfilment"], before.booking["fulfilment"]);
    assert_eq!(booking_after["seq"], before.booking["seq"]);

    let history_after =
        node_y2.rpc_ok("booking.history", json!({ "agreement": tx.quote_record_id })).await;
    assert_eq!(history_after["history"], before.history["history"]);

    let payment_after =
        node_y2.rpc_ok("payment.get", json!({ "agreement": tx.quote_record_id })).await;
    assert_eq!(payment_after["track"], before.payment["track"]);
    assert_eq!(payment_after["provider"], before.payment["provider"]);
    assert_eq!(payment_after["consumer"], before.payment["consumer"]);

    let fulfilment_after =
        node_y2.rpc_ok("fulfilment.get", json!({ "agreement": tx.quote_record_id })).await;
    assert_eq!(fulfilment_after["track"], before.fulfilment["track"]);
    assert_eq!(fulfilment_after["provider"], before.fulfilment["provider"]);

    let request_history_after =
        node_y2.rpc_ok("request.history", json!({ "request_id": tx.req_id })).await;
    assert_eq!(request_history_after["history"], before.request_history["history"]);
    let quote_history_after =
        node_y2.rpc_ok("quote.history", json!({ "quote_id": tx.quote_id })).await;
    assert_eq!(quote_history_after["history"], before.quote_history["history"]);

    let thread_after =
        node_y2.rpc_ok("transaction.thread", json!({ "conversation": tx.y_conv_id })).await;
    let cards_before = before.thread["cards"].as_array().unwrap();
    let cards_after = thread_after["cards"].as_array().unwrap();
    assert_eq!(cards_after.len(), cards_before.len());
    for (b, a) in cards_before.iter().zip(cards_after.iter()) {
        assert_eq!(a["card_type"], b["card_type"]);
        assert_eq!(a["record_id"], b["record_id"]);
        assert_eq!(a["verified"], b["verified"]);
        assert_eq!(a["reason"], b["reason"]);
    }

    let conv_history_after =
        node_y2.rpc_ok("conversation.history", json!({ "conversation": tx.y_conv_id })).await;
    let bodies_before: Vec<&Value> =
        before.conv_history["messages"].as_array().unwrap().iter().map(|m| &m["body"]).collect();
    let bodies_after: Vec<&Value> =
        conv_history_after["messages"].as_array().unwrap().iter().map(|m| &m["body"]).collect();
    assert_eq!(bodies_after, bodies_before);

    let req_verify = node_y2
        .rpc_ok("request.verify", json!({ "envelope": request_history_after["history"][0] }))
        .await;
    assert_eq!(req_verify["verified"], true);
    let quote_verify = node_y2
        .rpc_ok("quote.verify", json!({ "envelope": quote_history_after["history"][0] }))
        .await;
    assert_eq!(quote_verify["verified"], true);
    let agreement_after =
        node_y2.rpc_ok("agreement.get", json!({ "quote_record_id": tx.quote_record_id })).await;
    let agreement_verify = node_y2
        .rpc_ok("agreement.verify", json!({ "envelope": agreement_after["provider"]["envelope"] }))
        .await;
    assert_eq!(agreement_verify["verified"], true);
}

async fn assert_post_restore_operations(
    node_y2: &Node,
    tx: &ActiveTransaction,
    initial_seq: &Value,
    archive: &RoymArchive,
    recovery_key: &[u8; 32],
    bundles: &serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    let tick_view = node_y2.rpc_ok("booking.get", json!({ "agreement": tx.quote_record_id })).await;
    assert_eq!(&tick_view["seq"], initial_seq);

    let start_view =
        node_y2.rpc_ok("booking.start", json!({ "agreement": tx.quote_record_id })).await;
    assert_eq!(start_view["state"], "in-progress");

    let payment_after =
        node_y2.rpc_ok("payment.get", json!({ "agreement": tx.quote_record_id })).await;
    let latest_provider_ack = payment_after["provider"].as_array().unwrap().last().unwrap();
    let post_restore_correction = node_y2
        .rpc_ok(
            "payment.acknowledge",
            json!({
                "agreement": tx.quote_record_id, "method": "cash", "reference": "post-restore",
                "supersedes": latest_provider_ack["record_id"],
            }),
        )
        .await;
    assert_ne!(post_restore_correction["record_id"], latest_provider_ack["record_id"]);
    let post_restore_envelope =
        post_restore_correction_envelope(node_y2, &tx.quote_record_id).await;
    let post_restore_verify =
        node_y2.rpc_ok("payment.verify", json!({ "envelope": post_restore_envelope })).await;
    assert_eq!(post_restore_verify["verified"], true);

    let second_export = export_bundles(node_y2).await;
    for svc in ["profile", "catalog", "transaction"] {
        let before_bundle: Value = bundles.get(svc).cloned().unwrap();
        let after_bundle: Value = second_export.get(svc).cloned().unwrap();
        let before_digests: Vec<Value> =
            before_bundle["manifest"]["sections"].as_object().unwrap().values().cloned().collect();
        let after_digests: Vec<Value> =
            after_bundle["manifest"]["sections"].as_object().unwrap().values().cloned().collect();
        assert_eq!(before_digests.len(), after_digests.len(), "{svc} section count differs");
    }

    let mut tampered = archive.clone();
    let mut ct = tampered.data.ciphertext_z32.clone().into_bytes();
    let flip_at = ct.len() / 2;
    ct[flip_at] ^= 0x01;
    tampered.data.ciphertext_z32 = String::from_utf8_lossy(&ct).to_string();
    let tampered_open = open_archive_bundles(&tampered, recovery_key);
    assert!(tampered_open.is_err(), "a flipped ciphertext byte must not decrypt");

    let unchanged = node_y2.rpc_ok("booking.get", json!({ "agreement": tx.quote_record_id })).await;
    assert_eq!(unchanged["seq"], start_view["seq"]);
    second_export
}

#[tokio::test]
async fn a_provider_transaction_survives_an_encrypted_backup_and_restore() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let dir_y2 = tempfile::tempdir().unwrap();
    let owner_x = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();
    let owner_y_did = substrate::derive_did_key(&owner_y.public_key());

    let cluster = boot_restore_cluster(dir_x.path(), dir_y.path(), &owner_x, &owner_y).await;
    let (node_x, node_y) = (cluster.node_x, cluster.node_y);

    let tx =
        setup_active_transaction(&node_x, &node_y, &cluster.listing_id, &cluster.slot_id).await;
    let before = capture_before_snapshot(&node_y, &tx).await;

    // Step 2: Y builds an encrypted backup archive.
    let bundles = export_bundles(&node_y).await;
    let (archive, recovery_key) = seal_archive(&node_y.owner, &owner_y_did, bundles.clone());

    // Step 3: a clean Y' restores identity, deploys, enrols, restores data.
    let restored_identity = identity_backup::import(&archive.identity, &recovery_key).unwrap();
    assert_eq!(restored_identity.to_bytes(), node_y.owner.to_bytes());

    let mut node_y2 = Node::boot(
        "node-y2",
        dir_y2.path().to_path_buf(),
        cluster.shared_registry.clone(),
        restored_identity,
        fast_conversation_role(3600),
    )
    .await;
    node_y2.full_bring_up().await;

    let restore_bundles = open_archive_bundles(&archive, &recovery_key).unwrap();
    import_bundles(&node_y2, &restore_bundles).await;

    // Step 4: the durability suite.
    assert_durability_parity(&node_y2, &before, &tx).await;
    let second_export = assert_post_restore_operations(
        &node_y2,
        &tx,
        &before.booking["seq"],
        &archive,
        &recovery_key,
        &bundles,
    )
    .await;

    // Step 5: export from Y2 and verify import onto a clean node Y3.
    let (archive_y2, recovery_key_y2) = seal_archive(&node_y2.owner, &owner_y_did, second_export);
    let restored_identity_y3 =
        identity_backup::import(&archive_y2.identity, &recovery_key_y2).unwrap();
    let dir_y3 = tempfile::tempdir().unwrap();
    let mut node_y3 = Node::boot(
        "node-y3",
        dir_y3.path().to_path_buf(),
        cluster.shared_registry,
        restored_identity_y3,
        fast_conversation_role(3600),
    )
    .await;
    node_y3.full_bring_up().await;
    let restore_bundles_y3 = open_archive_bundles(&archive_y2, &recovery_key_y2).unwrap();
    import_bundles(&node_y3, &restore_bundles_y3).await;
    let y3_booking =
        node_y3.rpc_ok("booking.get", json!({ "agreement": tx.quote_record_id })).await;
    assert_eq!(y3_booking["state"], "in-progress");

    node_x.teardown().await;
    node_y.teardown().await;
    node_y2.teardown().await;
    node_y3.teardown().await;
}

async fn post_restore_correction_envelope(node: &Node, agreement: &str) -> String {
    let payment = node.rpc_ok("payment.get", json!({ "agreement": agreement })).await;
    let last = payment["provider"].as_array().unwrap().last().unwrap();
    last["envelope"].as_str().unwrap().to_string()
}
