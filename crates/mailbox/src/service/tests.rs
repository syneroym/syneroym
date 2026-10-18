use std::sync::Arc;

use serde_json::json;
use syneroym_core::{
    config::MailboxRole,
    mailbox::{
        METHOD_ACK, METHOD_CLAIM, METHOD_DEPOSIT, METHOD_INFO, METHOD_REGISTER, METHOD_STATUS,
        codes,
    },
};
use syneroym_identity::{DelegationCertificate, Identity};
use syneroym_mailbox_format::{Receipt, open};
use syneroym_rpc::{CallerContext, NativeInvocation, NativeService, RpcError};
use tempfile::tempdir;

use crate::{
    service::MailboxService,
    test_support::{Anchors, Member, record, sealed_deposit},
};

const NODE_DID: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

fn test_service(role: MailboxRole) -> (tempfile::TempDir, MailboxService, Arc<Anchors>) {
    let dir = tempdir().unwrap();
    let anchors = Arc::new(Anchors::new());
    let service = MailboxService::open(&role, dir.path(), NODE_DID, anchors.clone()).unwrap();
    (dir, service, anchors)
}

fn invocation(method: &str, params: serde_json::Value, caller: CallerContext) -> NativeInvocation {
    NativeInvocation {
        interface: "mailbox".to_string(),
        method: method.to_string(),
        params,
        caller,
    }
}

#[tokio::test]
async fn an_unknown_method_is_method_not_found() {
    let (_dir, service, _anchors) = test_service(MailboxRole::default());
    let member = Member::new(3600);
    let inv = invocation("nonexistent", json!({}), member.caller());
    let res = service.dispatch(inv).await;
    assert!(matches!(res, Err(RpcError::MethodNotFound(_))));
}

#[tokio::test]
async fn a_system_caller_is_refused() {
    let (_dir, service, _anchors) = test_service(MailboxRole::default());
    let inv = invocation(METHOD_REGISTER, json!({}), CallerContext::service_system("test-service"));
    let res = service.dispatch(inv).await;
    match res {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::UNAUTHENTICATED),
        other => panic!("expected UNAUTHENTICATED error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_routing_certificate_cannot_register() {
    let (_dir, service, _anchors) = test_service(MailboxRole::default());
    let mut member = Member::new(3600);
    member.cert = DelegationCertificate::issue(
        &member.master,
        member.instance.public_key(),
        3600,
        "routing".to_string(),
    )
    .unwrap();

    let rec = record(&member, NODE_DID);
    let inv = invocation(METHOD_REGISTER, json!({ "record": rec }), member.caller());
    let res = service.dispatch(inv).await;
    match res {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::NOT_AN_INSTANCE),
        other => panic!("expected NOT_AN_INSTANCE error, got {other:?}"),
    }
}

#[tokio::test]
async fn register_deposit_claim_ack_status_round_trip() {
    let (_dir, service, anchors) = test_service(MailboxRole::default());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    anchors.publish(&sender.service_id());
    anchors.publish(&receiver.service_id());

    let rec = record(&receiver, NODE_DID);
    let reg_inv = invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller());
    let reg_res = service.dispatch(reg_inv).await.unwrap();
    assert!(reg_res.payload.get("limits").is_some());

    let sealed = sealed_deposit(&sender, &receiver, 3600, syneroym_passport::now_secs());
    let dep_id = sealed.deposit.deposit_id;
    let dep_inv =
        invocation(METHOD_DEPOSIT, serde_json::to_value(&sealed.deposit).unwrap(), sender.caller());
    let dep_res = service.dispatch(dep_inv).await.unwrap();
    let stored_receipt: Receipt = serde_json::from_value(dep_res.payload).unwrap();
    assert!(matches!(stored_receipt, Receipt::Stored { .. }));

    let claim_inv = invocation(METHOD_CLAIM, json!({ "max": 10 }), receiver.caller());
    let claim_res = service.dispatch(claim_inv).await.unwrap();
    let claim_ans: syneroym_core::mailbox::ClaimAnswer =
        serde_json::from_value(claim_res.payload).unwrap();
    assert_eq!(claim_ans.messages.len(), 1);

    let claimed_msg = &claim_ans.messages[0];
    let now = syneroym_passport::now_secs();
    let opened = open(
        &receiver.mailbox,
        &receiver.service_id(),
        &claimed_msg.deposit_id,
        &claimed_msg.sealed,
        syneroym_mailbox_format::OpenPolicy { max_ttl_secs: 86400, max_clock_skew_secs: 3600 },
        now,
    )
    .unwrap();
    let delivered = opened.sign_delivered(&receiver.instance, &receiver.cert, now).unwrap();

    let ack_inv = invocation(
        METHOD_ACK,
        json!({
            "acks": [{
                "sender": sender.service_id(),
                "deposit_id": dep_id,
                "receipt": delivered
            }]
        }),
        receiver.caller(),
    );
    let ack_res = service.dispatch(ack_inv).await.unwrap();
    let ack_ans: syneroym_core::mailbox::AckAnswer =
        serde_json::from_value(ack_res.payload).unwrap();
    assert_eq!(ack_ans.results, vec![syneroym_core::mailbox::AckOutcome::Stored]);

    let st_inv = invocation(METHOD_STATUS, json!({ "deposit_ids": [dep_id] }), sender.caller());
    let st_res = service.dispatch(st_inv).await.unwrap();
    let st_ans: syneroym_core::mailbox::StatusAnswer =
        serde_json::from_value(st_res.payload).unwrap();
    assert_eq!(st_ans.receipts.len(), 1);
    assert!(st_ans.receipts[0].receipt.verify_signed(&sealed.sent, 3600, now).is_ok());
}

#[tokio::test]
async fn deposits_above_the_sender_rate_are_refused() {
    let role = MailboxRole {
        deposits_per_minute_per_sender: 2,
        deposits_per_minute_total: 100,
        ..Default::default()
    };
    let (_dir, service, anchors) = test_service(role);
    let s1 = Member::new(3600);
    let s2 = Member::new(3600);
    let receiver = Member::new(3600);
    anchors.publish(&receiver.service_id());

    let rec = record(&receiver, NODE_DID);
    service
        .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller()))
        .await
        .unwrap();

    let now = syneroym_passport::now_secs();
    let d1 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    let d2 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    let d3 = sealed_deposit(&s1, &receiver, 3600, now).deposit;

    assert!(service.dispatch(invocation(METHOD_DEPOSIT, json!(d1), s1.caller())).await.is_ok());
    assert!(service.dispatch(invocation(METHOD_DEPOSIT, json!(d2), s1.caller())).await.is_ok());

    let res3 = service.dispatch(invocation(METHOD_DEPOSIT, json!(d3), s1.caller())).await;
    match res3 {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::RATE_LIMITED),
        other => panic!("expected RATE_LIMITED, got {other:?}"),
    }

    let d_s2 = sealed_deposit(&s2, &receiver, 3600, now).deposit;
    assert!(service.dispatch(invocation(METHOD_DEPOSIT, json!(d_s2), s2.caller())).await.is_ok());
}

#[tokio::test]
async fn the_node_rate_limits_every_sender() {
    let role = MailboxRole {
        deposits_per_minute_per_sender: 10,
        deposits_per_minute_total: 2,
        ..Default::default()
    };
    let (_dir, service, anchors) = test_service(role);
    let s1 = Member::new(3600);
    let s2 = Member::new(3600);
    let s3 = Member::new(3600);
    let receiver = Member::new(3600);
    anchors.publish(&receiver.service_id());

    let rec = record(&receiver, NODE_DID);
    service
        .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller()))
        .await
        .unwrap();

    let now = syneroym_passport::now_secs();
    let d1 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    let d2 = sealed_deposit(&s2, &receiver, 3600, now).deposit;
    let d3 = sealed_deposit(&s3, &receiver, 3600, now).deposit;

    assert!(service.dispatch(invocation(METHOD_DEPOSIT, json!(d1), s1.caller())).await.is_ok());
    assert!(service.dispatch(invocation(METHOD_DEPOSIT, json!(d2), s2.caller())).await.is_ok());

    let res3 = service.dispatch(invocation(METHOD_DEPOSIT, json!(d3), s3.caller())).await;
    match res3 {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::RATE_LIMITED),
        other => panic!("expected RATE_LIMITED, got {other:?}"),
    }
}

#[tokio::test]
async fn a_deposit_over_the_size_limit_is_too_large() {
    let (_dir, service, anchors) = test_service(MailboxRole::default());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    anchors.publish(&receiver.service_id());

    let rec = record(&receiver, NODE_DID);
    service
        .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller()))
        .await
        .unwrap();

    let giant_sealed = "A".repeat(90_000);
    let dep_json = json!({
        "destination": receiver.service_id(),
        "deposit_id": "0123456789abcdef0123456789abcdef",
        "ttl_secs": 3600,
        "sealed": giant_sealed
    });

    let res = service.dispatch(invocation(METHOD_DEPOSIT, dep_json, sender.caller())).await;
    match res {
        Err(RpcError::Custom(code, _, Some(data))) => {
            assert_eq!(code, codes::TOO_LARGE);
            assert_eq!(data.get("kind").and_then(|v| v.as_str()), Some("too_large"));
        }
        other => panic!("expected TOO_LARGE error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_register_with_a_record_for_another_mailbox_is_refused() {
    let (_dir, service, anchors) = test_service(MailboxRole::default());
    let receiver = Member::new(3600);
    anchors.publish(&receiver.service_id());

    let other_mailbox_did = "did:key:z6MkrAnotherMailboxKey";
    let rec = record(&receiver, other_mailbox_did);
    let inv = invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller());
    let res = service.dispatch(inv).await;
    match res {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::BAD_RECORD),
        other => panic!("expected BAD_RECORD error, got {other:?}"),
    }
}

#[tokio::test]
async fn info_reports_the_limits() {
    let (_dir, service, _anchors) = test_service(MailboxRole::default());
    let member = Member::new(3600);
    let res = service.dispatch(invocation(METHOD_INFO, json!({}), member.caller())).await.unwrap();
    let info: syneroym_core::mailbox::InfoAnswer = serde_json::from_value(res.payload).unwrap();
    assert_eq!(info.version, syneroym_core::mailbox::PROTOCOL_VERSION);
    assert_eq!(info.mailbox, NODE_DID);
}

#[tokio::test]
async fn a_ttl_over_the_limit_reports_the_queue_limit() {
    let role = MailboxRole { max_ttl_secs: 500, ..Default::default() };
    let (_dir, service, anchors) = test_service(role);
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    anchors.publish(&receiver.service_id());

    let rec = record(&receiver, NODE_DID);
    service
        .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller()))
        .await
        .unwrap();

    let now = syneroym_passport::now_secs();
    let mut dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    dep.ttl_secs = 1000;

    let res = service.dispatch(invocation(METHOD_DEPOSIT, json!(dep), sender.caller())).await;
    match res {
        Err(RpcError::Custom(code, _, Some(data))) => {
            assert_eq!(code, codes::TTL_TOO_LONG);
            assert_eq!(data.get("kind").and_then(|v| v.as_str()), Some("ttl_too_long"));
            assert_eq!(data.get("max_ttl_secs").and_then(|v| v.as_u64()), Some(500));
        }
        other => panic!("expected TTL_TOO_LONG error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_revoked_instance_is_refused() {
    let (_dir, service, anchors) = test_service(MailboxRole::default());
    let receiver = Member::new(3600);
    anchors.publish(&receiver.service_id());
    anchors.revoke(&receiver.service_id(), &receiver.cert.temporary_did);

    let rec = record(&receiver, NODE_DID);
    let res = service
        .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller()))
        .await;
    match res {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::NOT_AN_INSTANCE),
        other => panic!("expected NOT_AN_INSTANCE, got {other:?}"),
    }

    let instance2 = Identity::generate().unwrap();
    let cert2 = DelegationCertificate::issue(
        &receiver.master,
        instance2.public_key(),
        3600,
        syneroym_identity::delegation::SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let mut caller2 = receiver.caller();
    caller2.proof = Some(syneroym_rpc::CallerProof {
        pubkey_hex: hex::encode(instance2.public_key().to_bytes()),
        delegation_json: Some(cert2.to_json().unwrap()),
    });

    let res2 =
        service.dispatch(invocation(METHOD_REGISTER, json!({ "record": rec }), caller2)).await;
    assert!(res2.is_ok());
}

#[tokio::test]
async fn an_unreadable_master_anchor_refuses_receiver_calls() {
    let (_dir, service, anchors) = test_service(MailboxRole::default());
    let receiver = Member::new(3600);
    let sender = Member::new(3600);
    anchors.publish(&receiver.service_id());

    let rec = record(&receiver, NODE_DID);
    service
        .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller()))
        .await
        .unwrap();

    anchors.set_fail(true);

    let claim_res =
        service.dispatch(invocation(METHOD_CLAIM, json!({ "max": 10 }), receiver.caller())).await;
    assert!(matches!(claim_res, Err(RpcError::InternalError(_))));

    let now = syneroym_passport::now_secs();
    let dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;

    let dep_res = service.dispatch(invocation(METHOD_DEPOSIT, json!(dep), sender.caller())).await;
    assert!(dep_res.is_ok());

    let st_res = service
        .dispatch(invocation(
            METHOD_STATUS,
            json!({ "deposit_ids": [dep.deposit_id] }),
            sender.caller(),
        ))
        .await;
    assert!(st_res.is_ok());
}
