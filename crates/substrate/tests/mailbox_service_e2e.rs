#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end integration tests for the service mailbox role against a live
//! substrate.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::SubstrateNode;
use rustls::crypto::ring;
use serde_json::{Value, from_value, json};
use syneroym_core::{
    config::MailboxRole,
    dht_registry::{
        DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointMailbox, EndpointType,
        RegistryClient, SignedEndpointInfo,
    },
    mailbox::{
        AckAnswer, AckOutcome, ClaimAnswer, MAILBOX_INTERFACE, METHOD_ACK, METHOD_CLAIM,
        METHOD_DEPOSIT, METHOD_REGISTER, METHOD_STATUS, StatusAnswer, codes,
    },
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_mailbox_format::{
    DepositId, MailboxSecretKey, MessageContent, OpenPolicy, Receipt, Recipient, Sealed, Sender,
    SignedOutcome, open, seal,
};
use syneroym_rpc::JsonRpcError;
use syneroym_sdk::SyneroymClient;
use tokio::time;

mod common;

fn did_of(id: &Identity) -> String {
    substrate::derive_did_key(&id.public_key())
}

fn not_after() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS)
}

struct Member {
    master: Identity,
    instance: Identity,
    cert: DelegationCertificate,
    mailbox_key: MailboxSecretKey,
}

impl Member {
    async fn new(node: &SubstrateNode) -> Self {
        let master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        Self::with_master_and_instance(node, master, instance).await
    }

    async fn with_master_and_instance(
        node: &SubstrateNode,
        master: Identity,
        instance: Identity,
    ) -> Self {
        let master_did = did_of(&master);
        let cert = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let mailbox_key = MailboxSecretKey::derive_from_master(&master);
        common::conversation_fixture::publish_master_anchor(
            &master_did,
            &master,
            node.registry_url(),
        )
        .await;
        Self { master, instance, cert, mailbox_key }
    }
}

async fn publish_anchors(node: &SubstrateNode, members: &[&Member]) {
    for m in members {
        let master_did = did_of(&m.master);
        common::conversation_fixture::publish_master_anchor(
            &master_did,
            &m.master,
            node.registry_url(),
        )
        .await;
    }
}

async fn instance_client(node: &SubstrateNode, member: &Member) -> SyneroymClient {
    let mut client = node
        .client_as(Identity::from_bytes(&member.instance.to_bytes()))
        .with_delegation(member.cert.clone());
    client.connect().await.expect("failed to connect instance client");
    client
}

async fn sender_client(node: &SubstrateNode, member: &Member) -> SyneroymClient {
    instance_client(node, member).await
}

fn record(member: &Member, node_did: &str) -> SignedEndpointInfo {
    EndpointInfo {
        service_id: did_of(&member.master),
        substrate_id: node_did.to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: not_after(),
        generation: 0,
        mailbox: Some(EndpointMailbox {
            service_id: node_did.to_string(),
            key: member.mailbox_key.public_key(),
        }),
    }
    .sign(&member.master)
    .unwrap()
}

fn make_sealed_deposit(
    sender: &Member,
    receiver: &Member,
    ttl: u64,
    now: u64,
    content: MessageContent,
) -> Sealed {
    let s = Sender { instance: &sender.instance, certificate: &sender.cert };
    let r_id = did_of(&receiver.master);
    let r = Recipient { service_id: &r_id, mailbox_key: &receiver.mailbox_key.public_key() };
    seal(&content, r, s, ttl, now).unwrap()
}

async fn call(client: &SyneroymClient, method: &str, params: Value) -> Result<Value, (i32, Value)> {
    match client.request(MAILBOX_INTERFACE, method, params).await {
        Ok(res) => Ok(res.result),
        Err(e) => {
            if let Some(rpc_err) = e.downcast_ref::<JsonRpcError>() {
                Err((rpc_err.code, rpc_err.data.clone().unwrap_or(Value::Null)))
            } else {
                panic!("unexpected transport or parse error: {e:?}");
            }
        }
    }
}

async fn boot_mailbox_node() -> SubstrateNode {
    let _ = ring::default_provider().install_default();
    let owner = Identity::generate().unwrap();
    SubstrateNode::builder()
        .owner(&owner)
        .configure(|c| {
            c.roles.mailbox = Some(MailboxRole {
                visibility_secs: 2,
                sweep_interval_secs: 1,
                republish_interval_secs: 2,
                ..Default::default()
            });
        })
        .boot()
        .await
}

#[tokio::test]
async fn a_registered_record_is_published_by_the_mailbox() {
    let _guard = common::serial_guard().await;
    let node = boot_mailbox_node().await;
    let receiver = Member::new(&node).await;
    let r_client = instance_client(&node, &receiver).await;

    let rec = record(&receiver, node.did());
    let reg_res = call(&r_client, METHOD_REGISTER, json!({ "record": rec })).await;
    assert!(reg_res.is_ok());

    let registry = RegistryClient::new(false, Some(node.registry_url().to_string()));
    let master_did = did_of(&receiver.master);
    let mut lookup_result = None;
    for _ in 0..20 {
        if let Ok(signed) = registry.lookup(&master_did, false).await {
            lookup_result = Some(signed);
            break;
        }
        time::sleep(Duration::from_millis(500)).await;
    }
    let looked_up = lookup_result.expect("record was not published to the registry");
    assert_eq!(looked_up.info.service_id, master_did);
    assert_eq!(looked_up.info.mailbox, rec.info.mailbox);
}

#[tokio::test]
async fn a_stale_republish_never_replaces_a_newer_record() {
    let _guard = common::serial_guard().await;
    let node = boot_mailbox_node().await;
    let receiver = Member::new(&node).await;
    let r_client = instance_client(&node, &receiver).await;

    let rec1 = record(&receiver, node.did());
    call(&r_client, METHOD_REGISTER, json!({ "record": rec1 })).await.unwrap();

    let registry = RegistryClient::new(false, Some(node.registry_url().to_string()));
    let master_did = did_of(&receiver.master);

    // Sleep briefly so the second record's signed_at timestamp is strictly newer
    time::sleep(Duration::from_millis(100)).await;
    let mut info2 = rec1.info.clone();
    info2.nickname = Some("newer-nickname".to_string());
    let rec2 = info2.sign(&receiver.master).unwrap();
    registry.register(&rec2, false).await.expect("direct register failed");

    // Wait for republish: the mailbox worker attempts to republish rec1,
    // which the registry refuses with 409 Conflict because rec2 is newer.
    time::sleep(Duration::from_secs(3)).await;

    let current = registry.lookup(&master_did, false).await.expect("lookup failed");
    assert_eq!(current.info.nickname, Some("newer-nickname".to_string()));

    // Now register rec3 (newest) with the mailbox, which immediately republishes it
    time::sleep(Duration::from_millis(100)).await;
    let mut info3 = rec1.info.clone();
    info3.nickname = Some("newest-nickname".to_string());
    let rec3 = info3.sign(&receiver.master).unwrap();
    call(&r_client, METHOD_REGISTER, json!({ "record": rec3 })).await.unwrap();

    let mut updated = None;
    for _ in 0..20 {
        if let Ok(signed) = registry.lookup(&master_did, false).await
            && signed.info.nickname.as_deref() == Some("newest-nickname")
        {
            updated = Some(signed);
            break;
        }
        time::sleep(Duration::from_millis(200)).await;
    }
    assert!(updated.is_some(), "registering a newer record must republish to registry");
}

#[tokio::test]
async fn a_deposit_reaches_the_receiver_and_its_delivery_receipt_reaches_the_sender() {
    let _guard = common::serial_guard().await;
    let node = boot_mailbox_node().await;
    let sender = Member::new(&node).await;
    let receiver = Member::new(&node).await;
    let s_client = sender_client(&node, &sender).await;
    let r_client = instance_client(&node, &receiver).await;

    let rec = record(&receiver, node.did());
    call(&r_client, METHOD_REGISTER, json!({ "record": rec })).await.unwrap();

    let now = sender.cert.issued_at_secs + 10;
    let content = MessageContent {
        interface: "test-interface".to_string(),
        method: "test-method".to_string(),
        params: json!({"msg": "hello"}),
        idempotency_key: "idem-deposit-1".to_string(),
    };
    let sealed = make_sealed_deposit(&sender, &receiver, 600, now, content);
    let dep_id = sealed.deposit.deposit_id;

    let deposit_val = call(
        &s_client,
        METHOD_DEPOSIT,
        json!({
            "destination": did_of(&receiver.master),
            "deposit_id": dep_id,
            "ttl_secs": 600,
            "sealed": String::from(sealed.deposit.sealed.clone()),
        }),
    )
    .await
    .unwrap();
    let deposit_receipt: Receipt = from_value(deposit_val).unwrap();
    assert!(matches!(deposit_receipt, Receipt::Stored { .. }));

    let claim_val = call(&r_client, METHOD_CLAIM, json!({})).await.unwrap();
    let claim_ans: ClaimAnswer = from_value(claim_val).unwrap();
    assert_eq!(claim_ans.messages.len(), 1);
    let msg = &claim_ans.messages[0];
    assert_eq!(msg.deposit_id, dep_id);

    let opened = open(
        &receiver.mailbox_key,
        &did_of(&receiver.master),
        &msg.deposit_id,
        &msg.sealed,
        OpenPolicy::default(),
        now,
    )
    .unwrap();
    assert_eq!(opened.content.interface, "test-interface");
    assert_eq!(opened.content.method, "test-method");

    let del_receipt = opened.sign_delivered(&receiver.instance, &receiver.cert, now).unwrap();
    let ack_val = call(
        &r_client,
        METHOD_ACK,
        json!({
            "acks": [{
                "sender": did_of(&sender.master),
                "deposit_id": dep_id,
                "receipt": del_receipt,
            }]
        }),
    )
    .await
    .unwrap();
    let ack_ans: AckAnswer = from_value(ack_val).unwrap();
    assert_eq!(ack_ans.results, vec![AckOutcome::Stored]);

    let status_val =
        call(&s_client, METHOD_STATUS, json!({ "deposit_ids": [dep_id] })).await.unwrap();
    let status_ans: StatusAnswer = from_value(status_val).unwrap();
    assert_eq!(status_ans.receipts.len(), 1);

    let verified = status_ans.receipts[0].receipt.verify_signed(&sealed.sent, 300, now).unwrap();
    assert_eq!(verified, SignedOutcome::Delivered);
}

#[tokio::test]
async fn a_moved_service_takes_the_queue_from_its_old_instance() {
    let _guard = common::serial_guard().await;
    let node = boot_mailbox_node().await;
    let receiver_a = Member::new(&node).await;
    let r_client_a = instance_client(&node, &receiver_a).await;

    let rec_a = record(&receiver_a, node.did());
    call(&r_client_a, METHOD_REGISTER, json!({ "record": rec_a })).await.unwrap();

    // Sleep so certificate issued_at is strictly in the next second
    time::sleep(Duration::from_millis(1100)).await;

    let receiver_b = Member::with_master_and_instance(
        &node,
        Identity::from_bytes(&receiver_a.master.to_bytes()),
        Identity::generate().unwrap(),
    )
    .await;
    let r_client_b = instance_client(&node, &receiver_b).await;

    let rec_b = record(&receiver_b, node.did());
    call(&r_client_b, METHOD_REGISTER, json!({ "record": rec_b })).await.unwrap();

    // Old instance A's claim is stale
    let err_a = call(&r_client_a, METHOD_CLAIM, json!({})).await.unwrap_err();
    assert_eq!(err_a.0, codes::STALE_INSTANCE);

    // New instance B's claim succeeds
    let ok_b = call(&r_client_b, METHOD_CLAIM, json!({})).await;
    assert!(ok_b.is_ok());
}

#[tokio::test]
async fn refusals_have_their_own_codes() {
    let _guard = common::serial_guard().await;
    let node = boot_mailbox_node().await;
    let sender = Member::new(&node).await;
    let receiver = Member::new(&node).await;
    let s_client = sender_client(&node, &sender).await;
    let r_client = instance_client(&node, &receiver).await;

    let now = sender.cert.issued_at_secs + 10;
    let content = MessageContent {
        interface: "iface".to_string(),
        method: "method".to_string(),
        params: json!({}),
        idempotency_key: "idem-dup".to_string(),
    };
    let sealed1 = make_sealed_deposit(&sender, &receiver, 600, now, content);

    // 1. Deposit to unknown destination
    let unknown_dest = did_of(&Identity::generate().unwrap());
    let err1 = call(
        &s_client,
        METHOD_DEPOSIT,
        json!({
            "destination": unknown_dest,
            "deposit_id": DepositId::random().unwrap(),
            "ttl_secs": 600,
            "sealed": String::from(sealed1.deposit.sealed.clone()),
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err1.0, codes::UNKNOWN_DESTINATION);

    // 2. Register with uncertificated / non-delegated client
    let uncert_client = {
        let mut c = node.client_as(Identity::generate().unwrap());
        c.connect().await.unwrap();
        c
    };
    let rec = record(&receiver, node.did());
    let err2 = call(&uncert_client, METHOD_REGISTER, json!({ "record": rec })).await.unwrap_err();
    assert_eq!(err2.0, codes::NOT_AN_INSTANCE);

    // Register receiver properly
    call(&r_client, METHOD_REGISTER, json!({ "record": rec })).await.unwrap();

    // 3. First deposit succeeds
    let ok_dep = call(
        &s_client,
        METHOD_DEPOSIT,
        json!({
            "destination": did_of(&receiver.master),
            "deposit_id": sealed1.deposit.deposit_id,
            "ttl_secs": 600,
            "sealed": String::from(sealed1.deposit.sealed.clone()),
        }),
    )
    .await;
    assert!(ok_dep.is_ok());

    // 4. Second identical deposit answers stored
    let dup_dep = call(
        &s_client,
        METHOD_DEPOSIT,
        json!({
            "destination": did_of(&receiver.master),
            "deposit_id": sealed1.deposit.deposit_id,
            "ttl_secs": 600,
            "sealed": String::from(sealed1.deposit.sealed.clone()),
        }),
    )
    .await
    .unwrap();
    let dup_receipt: Receipt = from_value(dup_dep).unwrap();
    assert!(matches!(dup_receipt, Receipt::Stored { .. }));

    // 5. Duplicate ID with changed content is rejected
    let content2 = MessageContent {
        interface: "iface2".to_string(),
        method: "method2".to_string(),
        params: json!({"other": "data"}),
        idempotency_key: "idem-dup".to_string(),
    };
    let sealed2 = make_sealed_deposit(&sender, &receiver, 600, now, content2);
    let err_dup = call(
        &s_client,
        METHOD_DEPOSIT,
        json!({
            "destination": did_of(&receiver.master),
            "deposit_id": sealed1.deposit.deposit_id,
            "ttl_secs": 600,
            "sealed": String::from(sealed2.deposit.sealed.clone()),
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(err_dup.0, codes::DUPLICATE_DEPOSIT);
}

#[tokio::test]
async fn the_queue_and_its_messages_survive_a_restart() {
    let _guard = common::serial_guard().await;
    let mut node = boot_mailbox_node().await;
    let sender = Member::new(&node).await;
    let receiver = Member::new(&node).await;
    let s_client = sender_client(&node, &sender).await;
    let r_client = instance_client(&node, &receiver).await;

    let rec = record(&receiver, node.did());
    call(&r_client, METHOD_REGISTER, json!({ "record": rec })).await.unwrap();

    let now = sender.cert.issued_at_secs + 10;
    let content = MessageContent {
        interface: "restart-iface".to_string(),
        method: "restart-method".to_string(),
        params: json!({}),
        idempotency_key: "idem-restart".to_string(),
    };
    let sealed = make_sealed_deposit(&sender, &receiver, 600, now, content);
    let dep_id = sealed.deposit.deposit_id;

    call(
        &s_client,
        METHOD_DEPOSIT,
        json!({
            "destination": did_of(&receiver.master),
            "deposit_id": dep_id,
            "ttl_secs": 600,
            "sealed": String::from(sealed.deposit.sealed.clone()),
        }),
    )
    .await
    .unwrap();

    // Restart the node in place
    node.restart_in_place().await;

    // The restarted node's in-memory registry needs the anchors republished
    publish_anchors(&node, &[&sender, &receiver]).await;

    // Reconnect receiver client
    let r_client_reconnected = instance_client(&node, &receiver).await;
    let claim_val = call(&r_client_reconnected, METHOD_CLAIM, json!({})).await.unwrap();
    let claim_ans: ClaimAnswer = from_value(claim_val).unwrap();
    assert_eq!(claim_ans.messages.len(), 1);
    assert_eq!(claim_ans.messages[0].deposit_id, dep_id);
}

#[tokio::test]
async fn an_unclaimed_message_returns_after_its_visibility_time() {
    let _guard = common::serial_guard().await;
    let node = boot_mailbox_node().await;
    let sender = Member::new(&node).await;
    let receiver = Member::new(&node).await;
    let s_client = sender_client(&node, &sender).await;
    let r_client = instance_client(&node, &receiver).await;

    let rec = record(&receiver, node.did());
    call(&r_client, METHOD_REGISTER, json!({ "record": rec })).await.unwrap();

    let now = sender.cert.issued_at_secs + 10;
    let content = MessageContent {
        interface: "vis-iface".to_string(),
        method: "vis-method".to_string(),
        params: json!({}),
        idempotency_key: "idem-vis".to_string(),
    };
    let sealed = make_sealed_deposit(&sender, &receiver, 600, now, content);
    let dep_id = sealed.deposit.deposit_id;

    call(
        &s_client,
        METHOD_DEPOSIT,
        json!({
            "destination": did_of(&receiver.master),
            "deposit_id": dep_id,
            "ttl_secs": 600,
            "sealed": String::from(sealed.deposit.sealed.clone()),
        }),
    )
    .await
    .unwrap();

    // First claim receives the message
    let claim1_val = call(&r_client, METHOD_CLAIM, json!({})).await.unwrap();
    let claim1_ans: ClaimAnswer = from_value(claim1_val).unwrap();
    assert_eq!(claim1_ans.messages.len(), 1);
    assert_eq!(claim1_ans.messages[0].claim_count, 1);

    // Immediate second claim gives nothing (visibility window is 2s)
    let claim2_val = call(&r_client, METHOD_CLAIM, json!({})).await.unwrap();
    let claim2_ans: ClaimAnswer = from_value(claim2_val).unwrap();
    assert!(claim2_ans.messages.is_empty());

    // Wait for visibility timeout (visibility_secs = 2s, wait 3s)
    time::sleep(Duration::from_secs(3)).await;

    // Third claim receives the message again with claim_count == 2
    let claim3_val = call(&r_client, METHOD_CLAIM, json!({})).await.unwrap();
    let claim3_ans: ClaimAnswer = from_value(claim3_val).unwrap();
    assert_eq!(claim3_ans.messages.len(), 1);
    assert_eq!(claim3_ans.messages[0].claim_count, 2);
}
