use std::sync::Arc;

use serde_json::json;
use syneroym_core::{
    config::MailboxRole,
    mailbox::{
        AckAnswer, AckOutcome, ClaimAnswer, InfoAnswer, METHOD_ACK, METHOD_CLAIM, METHOD_DEPOSIT,
        METHOD_INFO, METHOD_REGISTER, METHOD_STATUS, METHOD_UNREGISTER, PROTOCOL_VERSION,
        StatusAnswer, UnregisterAnswer, codes,
    },
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_mailbox_format::{Receipt, open};
use syneroym_rpc::{
    AuthLevel, CallerContext, CallerProof, NativeInvocation, NativeService, RpcError,
    SessionContext,
};
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
    let claim_ans: ClaimAnswer = serde_json::from_value(claim_res.payload).unwrap();
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
    let ack_ans: AckAnswer = serde_json::from_value(ack_res.payload).unwrap();
    assert_eq!(ack_ans.results, vec![AckOutcome::Stored]);

    let st_inv = invocation(METHOD_STATUS, json!({ "deposit_ids": [dep_id] }), sender.caller());
    let st_res = service.dispatch(st_inv).await.unwrap();
    let st_ans: StatusAnswer = serde_json::from_value(st_res.payload).unwrap();
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
    let info: InfoAnswer = serde_json::from_value(res.payload).unwrap();
    assert_eq!(info.version, PROTOCOL_VERSION);
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
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let mut caller2 = receiver.caller();
    caller2.proof = Some(CallerProof {
        pubkey_hex: hex::encode(instance2.public_key().to_bytes()),
        delegation_json: Some(cert2.to_json().unwrap()),
        ..Default::default()
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
    service.clear_revocation_cache();

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

#[tokio::test]
async fn unregister_of_no_queue_answers_false() {
    let (_dir, service, anchors) = test_service(MailboxRole::default());
    let member = Member::new(3600);
    anchors.publish(&member.service_id());

    let inv = invocation(METHOD_UNREGISTER, json!({}), member.caller());
    let res = service.dispatch(inv).await.unwrap();
    let ans: UnregisterAnswer = serde_json::from_value(res.payload).unwrap();
    assert!(!ans.removed);
}

#[tokio::test]
async fn raw_key_caller_with_no_delegation_is_refused_on_deposit_and_status() {
    let (_dir, service, anchors) = test_service(MailboxRole::default());
    let receiver = Member::new(3600);
    anchors.publish(&receiver.service_id());

    let rec = record(&receiver, NODE_DID);
    service
        .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller()))
        .await
        .unwrap();

    let raw_key = Identity::generate().unwrap();
    let raw_key_did = substrate::derive_did_key(&raw_key.public_key());
    let raw_caller = CallerContext {
        caller_did: raw_key_did.clone(),
        app_instance: None,
        session: SessionContext {
            subject_did: raw_key_did,
            capabilities: Vec::new(),
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: Some(CallerProof {
            pubkey_hex: hex::encode(raw_key.public_key().to_bytes()),
            delegation_json: None,
            ..Default::default()
        }),
        on_behalf_did: None,
    };

    let now = syneroym_passport::now_secs();
    let sender = Member::new(3600);
    let dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;

    let dep_res =
        service.dispatch(invocation(METHOD_DEPOSIT, json!(dep), raw_caller.clone())).await;
    match dep_res {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::NOT_AN_INSTANCE),
        other => panic!("expected NOT_AN_INSTANCE on deposit, got {other:?}"),
    }

    let st_res = service
        .dispatch(invocation(METHOD_STATUS, json!({ "deposit_ids": [dep.deposit_id] }), raw_caller))
        .await;
    match st_res {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::NOT_AN_INSTANCE),
        other => panic!("expected NOT_AN_INSTANCE on status, got {other:?}"),
    }
}

#[tokio::test]
async fn register_above_rate_limits_is_refused() {
    let role = MailboxRole {
        register_per_minute_per_master: 2,
        register_per_minute_total: 3,
        ..Default::default()
    };
    let (_dir, service, anchors) = test_service(role);
    let m1 = Member::new(3600);
    let m2 = Member::new(3600);
    let m3 = Member::new(3600);
    anchors.publish(&m1.service_id());
    anchors.publish(&m2.service_id());
    anchors.publish(&m3.service_id());

    let rec1 = record(&m1, NODE_DID);
    assert!(
        service
            .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec1 }), m1.caller()))
            .await
            .is_ok()
    );
    assert!(
        service
            .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec1 }), m1.caller()))
            .await
            .is_ok()
    );

    let res_m1 =
        service.dispatch(invocation(METHOD_REGISTER, json!({ "record": rec1 }), m1.caller())).await;
    match res_m1 {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::RATE_LIMITED),
        other => panic!("expected RATE_LIMITED for m1, got {other:?}"),
    }

    let rec2 = record(&m2, NODE_DID);
    assert!(
        service
            .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec2 }), m2.caller()))
            .await
            .is_ok()
    );

    let rec3 = record(&m3, NODE_DID);
    let res_m3 =
        service.dispatch(invocation(METHOD_REGISTER, json!({ "record": rec3 }), m3.caller())).await;
    match res_m3 {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::RATE_LIMITED),
        other => panic!("expected RATE_LIMITED for total register, got {other:?}"),
    }
}

#[tokio::test]
async fn an_expired_record_on_existing_queue_is_refused_while_existing_messages_remain_claimable() {
    let (_dir, service, anchors) = test_service(MailboxRole::default());
    let receiver = Member::new(3600);
    let sender = Member::new(3600);
    anchors.publish(&receiver.service_id());

    let rec = record(&receiver, NODE_DID);
    service
        .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller()))
        .await
        .unwrap();

    let now = syneroym_passport::now_secs();
    let dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    let dep_id = dep.deposit_id;
    service.dispatch(invocation(METHOD_DEPOSIT, json!(dep), sender.caller())).await.unwrap();

    // Try to register an expired record for the same queue
    let mut expired_rec = record(&receiver, NODE_DID);
    expired_rec.info.not_after = now.saturating_sub(60);
    let resigned = expired_rec.info.sign(&receiver.master).unwrap();

    let res = service
        .dispatch(invocation(METHOD_REGISTER, json!({ "record": resigned }), receiver.caller()))
        .await;
    match res {
        Err(RpcError::Custom(code, _, Some(data))) => {
            assert_eq!(code, codes::BAD_RECORD);
            assert_eq!(data["kind"], "bad_record");
        }
        other => panic!("expected BAD_RECORD for expired record, got {other:?}"),
    }

    // Existing messages and claiming on the queue still work fine!
    let claim_res =
        service.dispatch(invocation(METHOD_CLAIM, json!({}), receiver.caller())).await.unwrap();
    let claim_ans: ClaimAnswer = serde_json::from_value(claim_res.payload).unwrap();
    assert_eq!(claim_ans.messages.len(), 1);
    assert_eq!(claim_ans.messages[0].deposit_id, dep_id);
}

#[tokio::test]
async fn revocation_cache_admits_calls_within_15_seconds_and_refuses_after() {
    let (_dir, service, anchors) = test_service(MailboxRole::default());
    let receiver = Member::new(3600);
    anchors.publish(&receiver.service_id());

    let rec = record(&receiver, NODE_DID);
    let t0 = receiver.cert.issued_at_secs + 10;
    assert!(
        service
            .dispatch_at(
                invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller()),
                t0,
            )
            .await
            .is_ok()
    );

    // Revoke the key at the master anchor
    anchors.revoke(&receiver.service_id(), &receiver.cert.temporary_did);

    // Call within 15 seconds (t0 + 5) succeeds via revocation cache
    let claim_res_cached =
        service.dispatch_at(invocation(METHOD_CLAIM, json!({}), receiver.caller()), t0 + 5).await;
    assert!(claim_res_cached.is_ok());

    // Call after 15 seconds (t0 + 16) discovers revocation and is refused
    let claim_res_expired =
        service.dispatch_at(invocation(METHOD_CLAIM, json!({}), receiver.caller()), t0 + 16).await;
    match claim_res_expired {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::NOT_AN_INSTANCE),
        other => panic!("expected NOT_AN_INSTANCE after cache expires, got {other:?}"),
    }
}

#[tokio::test]
async fn claim_and_receiver_calls_are_rate_limited_per_master() {
    let role = MailboxRole { receiver_calls_per_minute_per_master: 2, ..Default::default() };
    let (_dir, service, anchors) = test_service(role);
    let receiver = Member::new(3600);
    anchors.publish(&receiver.service_id());

    let rec = record(&receiver, NODE_DID);
    assert!(
        service
            .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller()))
            .await
            .is_ok()
    );

    // Two receiver calls within the 2/minute budget succeed
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), receiver.caller())).await.is_ok());
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), receiver.caller())).await.is_ok());

    // Third receiver call exceeds 2 per minute and is refused with RATE_LIMITED
    let res = service.dispatch(invocation(METHOD_CLAIM, json!({}), receiver.caller())).await;
    match res {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::RATE_LIMITED),
        other => panic!("expected RATE_LIMITED for claim over limit, got {other:?}"),
    }

    // Register draws from a separate limiter and remains usable
    assert!(
        service
            .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec }), receiver.caller()))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn receiver_calls_are_rate_limited_node_wide() {
    let role = MailboxRole {
        max_queues: 3,
        receiver_calls_per_minute_per_master: 10,
        receiver_calls_per_minute_total: 6,
        ..Default::default()
    };
    let (_dir, service, anchors) = test_service(role);
    let r1 = Member::new(3600);
    let r2 = Member::new(3600);
    let r3 = Member::new(3600);
    anchors.publish(&r1.service_id());
    anchors.publish(&r2.service_id());
    anchors.publish(&r3.service_id());

    let rec1 = record(&r1, NODE_DID);
    let rec2 = record(&r2, NODE_DID);
    let rec3 = record(&r3, NODE_DID);
    assert!(
        service
            .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec1 }), r1.caller()))
            .await
            .is_ok()
    );
    assert!(
        service
            .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec2 }), r2.caller()))
            .await
            .is_ok()
    );
    assert!(
        service
            .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec3 }), r3.caller()))
            .await
            .is_ok()
    );

    // Receiver calls across the three receivers exhaust the node-wide limit of 6
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), r1.caller())).await.is_ok());
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), r1.caller())).await.is_ok());
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), r2.caller())).await.is_ok());
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), r2.caller())).await.is_ok());
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), r3.caller())).await.is_ok());
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), r3.caller())).await.is_ok());

    // Seventh receiver call across the node is refused with RATE_LIMITED
    let res = service.dispatch(invocation(METHOD_CLAIM, json!({}), r3.caller())).await;
    match res {
        Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::RATE_LIMITED),
        other => panic!("expected RATE_LIMITED for total receiver calls, got {other:?}"),
    }
}

#[tokio::test]
async fn single_master_over_limit_does_not_consume_node_wide_budget() {
    let role = MailboxRole {
        max_queues: 2,
        receiver_calls_per_minute_per_master: 2,
        receiver_calls_per_minute_total: 4,
        ..Default::default()
    };
    let (_dir, service, anchors) = test_service(role);
    let master_a = Member::new(3600);
    let master_b = Member::new(3600);
    anchors.publish(&master_a.service_id());
    anchors.publish(&master_b.service_id());

    let rec_a = record(&master_a, NODE_DID);
    let rec_b = record(&master_b, NODE_DID);
    assert!(
        service
            .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec_a }), master_a.caller()))
            .await
            .is_ok()
    );
    assert!(
        service
            .dispatch(invocation(METHOD_REGISTER, json!({ "record": rec_b }), master_b.caller()))
            .await
            .is_ok()
    );

    // Master A makes 2 successful receiver calls
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), master_a.caller())).await.is_ok());
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), master_a.caller())).await.is_ok());

    // Master A makes 3 more receiver calls; all are refused by per-master limit
    for _ in 0..3 {
        let res = service.dispatch(invocation(METHOD_CLAIM, json!({}), master_a.caller())).await;
        match res {
            Err(RpcError::Custom(code, _, _)) => assert_eq!(code, codes::RATE_LIMITED),
            other => panic!("expected RATE_LIMITED for master A, got {other:?}"),
        }
    }

    // Master B still has budget under the node-wide limit of 4 (A's rejected calls
    // did not consume total tokens)
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), master_b.caller())).await.is_ok());
    assert!(service.dispatch(invocation(METHOD_CLAIM, json!({}), master_b.caller())).await.is_ok());
}
