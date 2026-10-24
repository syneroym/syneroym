use serde_json::json;
use syneroym_core::{config::MailboxRole, mailbox::codes};
use syneroym_mailbox_format::{OPEN_MAX_TTL_SECS, Receipt};
use syneroym_rpc::{
    Delivery, DepositStatus, DepositTicket, NOT_HOSTED_HERE_RPC_CODE, PreambleSignature,
    STREAM_UNAUTHENTICATED_RPC_CODE,
};

use super::{helpers::*, mailbox_helpers::*};
use crate::{handshake::now_secs, proxy::mailbox::poll::AnchorCache};

const WEEK_SECS: u64 = OPEN_MAX_TTL_SECS;

async fn node(outcomes: Vec<MockOutcome>) -> SenderNode {
    sender_node(MailboxRole::default(), outcomes).await
}

async fn deposited(node: &SenderNode) -> DepositTicket {
    match node.router.deliver_to_peer(&node.request(), node.peer(true), WEEK_SECS).await {
        Ok(Delivery::Deposited(ticket)) => ticket,
        other => panic!("expected a deposit, got {other:?}"),
    }
}

async fn poll(node: &SenderNode, contexts: Vec<Vec<u8>>) -> Vec<DepositStatus> {
    node.router.poll_deposits_inner(SENDER, &contexts).await
}

async fn poll_one(node: &SenderNode, ticket: &DepositTicket) -> DepositStatus {
    poll(node, vec![ticket.context.clone()]).await.remove(0)
}

fn is_retry(status: &DepositStatus) -> bool {
    matches!(status, DepositStatus::Retry { .. })
}

// -- the direct attempt and the deposit --------------------------------------

#[tokio::test]
async fn a_receiver_that_answers_directly_is_never_deposited() {
    let node = node(vec![MockOutcome::Success(json!("hello"))]).await;

    let delivery = node.router.deliver_to_peer(&node.request(), node.peer(true), WEEK_SECS).await;

    assert!(matches!(delivery, Ok(Delivery::Direct(v)) if v == json!("hello")));
    assert_eq!(node.mailbox.count("deposit"), 0);
}

#[tokio::test]
async fn an_unreachable_receiver_is_deposited_once_with_one_direct_attempt() {
    let node = node(vec![MockOutcome::Transport]).await;

    let ticket = deposited(&node).await;

    assert_eq!(node.hop.call_count(), 1, "one direct attempt, no retry");
    assert_eq!(node.mailbox.count("deposit"), 1);
    assert_eq!(poll_one(&node, &ticket).await, DepositStatus::Waiting);
}

#[tokio::test]
async fn a_receiver_with_no_address_is_deposited_without_a_direct_attempt() {
    let node = node(vec![]).await;

    let delivery = node.router.deliver_to_peer(&node.request(), node.peer(false), WEEK_SECS).await;

    assert!(matches!(delivery, Ok(Delivery::Deposited(_))), "{delivery:?}");
    assert_eq!(node.hop.call_count(), 0);
}

#[tokio::test]
async fn a_refusal_before_dispatch_is_deposited_after_one_attempt() {
    let refusals = [
        MockOutcome::Callee { code: STREAM_UNAUTHENTICATED_RPC_CODE, message: "clock".into() },
        MockOutcome::Callee { code: NOT_HOSTED_HERE_RPC_CODE, message: "moved".into() },
        MockOutcome::Callee { code: SERVICE_NOT_FOUND_RPC_CODE, message: "starting".into() },
        MockOutcome::HostMismatch,
    ];
    for refusal in refusals {
        let node = node(vec![refusal.clone()]).await;

        let delivery =
            node.router.deliver_to_peer(&node.request(), node.peer(true), WEEK_SECS).await;

        assert!(matches!(delivery, Ok(Delivery::Deposited(_))), "{refusal:?}: {delivery:?}");
        assert_eq!(node.hop.call_count(), 1, "{refusal:?}: no retry and no re-homing");
    }
}

#[tokio::test]
async fn a_callee_answer_is_returned_and_not_deposited() {
    let node = node(vec![MockOutcome::Callee { code: -32010, message: "no".into() }]).await;

    let delivery = node.router.deliver_to_peer(&node.request(), node.peer(true), WEEK_SECS).await;

    assert!(matches!(delivery, Err(ProxyError::Callee { code: -32010, .. })), "{delivery:?}");
    assert_eq!(node.mailbox.count("deposit"), 0);
}

#[tokio::test]
async fn the_direct_attempt_presents_the_sender_instance_and_no_on_behalf() {
    let node = node(vec![MockOutcome::Transport]).await;

    deposited(&node).await;

    let preamble = node.hop.all_preambles.lock().unwrap()[0].clone();
    assert_eq!(preamble.pubkey.as_deref(), Some(node.sender_instance_pubkey_hex.as_str()));
    assert_eq!(preamble.delegation.as_ref(), Some(&node.sender_cert));
    assert!(preamble.sig.is_some());
    assert!(preamble.on_behalf.is_none());
}

#[tokio::test]
async fn a_call_with_a_caller_proof_is_never_deposited() {
    let node = node(vec![MockOutcome::Transport; 3]).await;
    let mut req = node.request();
    req.origin = CallOrigin::Native { service_id: Some(SENDER.to_string()) };
    req.caller.proof = Some(CallerProof {
        pubkey_hex: "deadbeef0011".to_string(),
        delegation_json: None,
        signed: Some(PreambleSignature {
            service_id: "client-svc".to_string(),
            interface: "client-iface".to_string(),
            ts: 1_700_000_000,
            sig_hex: "00".repeat(64),
        }),
        on_behalf: None,
    });

    let delivery = node.router.deliver_to_peer(&req, node.peer(true), WEEK_SECS).await;

    assert!(matches!(delivery, Err(ProxyError::Transport(_))), "{delivery:?}");
    assert_eq!(node.mailbox.count("deposit"), 0);
    assert_eq!(node.hop.call_count(), 3, "today's retry loop");
    let preamble = node.hop.all_preambles.lock().unwrap()[0].clone();
    assert_eq!(preamble.on_behalf.expect("the caller's proof travels").pubkey, "deadbeef0011");
}

#[tokio::test]
async fn a_call_without_a_key_is_never_deposited() {
    let node = node(vec![MockOutcome::Transport]).await;
    let mut req = node.request();
    req.idempotency_key = None;
    req.idempotent = false;

    let delivery = node.router.deliver_to_peer(&req, node.peer(true), WEEK_SECS).await;

    assert!(matches!(delivery, Err(ProxyError::Transport(_))), "{delivery:?}");
    assert_eq!(node.mailbox.count("deposit"), 0);
}

#[tokio::test]
async fn a_sender_with_an_expired_certificate_is_never_deposited() {
    let node = node(vec![MockOutcome::Transport; 3]).await;
    let expired = DelegationCertificate::issue(
        &node.sender_master,
        node.router.service_instance(SENDER).unwrap().key.public_key(),
        0,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    node.registry.set_instance_cert(SENDER.to_string(), expired).await.unwrap();

    let delivery = node.router.deliver_to_peer(&node.request(), node.peer(true), WEEK_SECS).await;

    assert!(matches!(delivery, Err(ProxyError::Transport(_))), "{delivery:?}");
    assert_eq!(node.mailbox.count("deposit"), 0);
}

#[tokio::test]
async fn a_receiver_without_a_mailbox_keeps_todays_path() {
    let node = node(vec![MockOutcome::Transport; 3]).await;

    let delivery =
        node.router.deliver_to_peer(&node.request(), node.peer_without_mailbox(), WEEK_SECS).await;

    assert!(matches!(delivery, Err(ProxyError::Transport(_))), "{delivery:?}");
    assert_eq!(node.hop.call_count(), 3, "the retry loop runs");
    assert_eq!(node.mailbox.count("deposit"), 0);
}

#[tokio::test]
async fn a_receiver_without_a_mailbox_is_looked_up_again_on_not_hosted_here() {
    let node =
        node(vec![MockOutcome::Callee { code: NOT_HOSTED_HERE_RPC_CODE, message: "moved".into() }])
            .await;

    let delivery =
        node.router.deliver_to_peer(&node.request(), node.peer_without_mailbox(), WEEK_SECS).await;

    // No registry answers the second lookup, so the call ends as not found.
    assert!(matches!(delivery, Err(ProxyError::ServiceNotFound(_))), "{delivery:?}");
    assert_eq!(node.hop.call_count(), 1);
}

#[tokio::test]
async fn a_remote_mailbox_call_is_signed_by_the_sender_instance_and_carries_no_on_behalf() {
    let node = node(vec![MockOutcome::Success(json!({ "receipts": [] }))]).await;
    let sender = node.router.service_instance(SENDER).unwrap();
    let call = ProxyRouter::mailbox_request(
        &sender,
        &node.mailbox_did,
        "status",
        json!({ "deposit_ids": [] }),
    );
    assert!(call.caller.proof.is_some(), "the unsigned proof is for a same-node mailbox");

    node.router.invoke_remote_once(&synthetic_peer(), &call, Duration::from_secs(1)).await.unwrap();

    let preamble = node.hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey.as_deref(), Some(node.sender_instance_pubkey_hex.as_str()));
    assert_eq!(preamble.delegation.as_ref(), Some(&node.sender_cert));
    assert!(preamble.sig.is_some());
    assert!(preamble.on_behalf.is_none());
}

#[tokio::test]
async fn a_message_too_large_to_seal_stays_on_direct_delivery() {
    let node = node(vec![MockOutcome::Transport]).await;
    let mut req = node.request();
    req.params = json!({ "blob": "x".repeat(70 * 1024) });

    let delivery = node.router.deliver_to_peer(&req, node.peer(true), WEEK_SECS).await;

    assert!(matches!(delivery, Err(ProxyError::Transport(_))), "{delivery:?}");
    assert_eq!(node.mailbox.count("deposit"), 0);
}

#[tokio::test]
async fn a_ttl_the_mailbox_refuses_is_sealed_again_shorter() {
    let role = MailboxRole { max_ttl_secs: 3600, ..MailboxRole::default() };
    let node = sender_node(role, vec![MockOutcome::Transport]).await;

    let ticket = deposited(&node).await;

    assert_eq!(node.mailbox.count("deposit"), 2, "one refusal, one deposit");
    let latest_end_ms = (now_secs() + 3600 + 60) * 1000 + 3_600_000;
    assert!(ticket.deadline_at_ms <= i64::try_from(latest_end_ms).unwrap());
    assert_eq!(poll_one(&node, &ticket).await, DepositStatus::Waiting);
}

#[tokio::test]
async fn a_mailbox_that_cannot_be_reached_returns_the_direct_error() {
    let node = node(vec![MockOutcome::Transport]).await;
    let mut peer = node.peer(true);
    peer.mailbox.as_mut().unwrap().service_id = "did:key:zNobodyHostsThis".to_string();

    let delivery = node.router.deliver_to_peer(&node.request(), peer, WEEK_SECS).await;

    assert!(matches!(delivery, Err(ProxyError::Transport(_))), "{delivery:?}");
}

#[tokio::test]
async fn a_same_node_deposit_carries_the_instance_proof() {
    let node = node(vec![MockOutcome::Transport]).await;

    let ticket = deposited(&node).await;

    // The mailbox keyed the deposit on the sender's master, so the same
    // sender can ask about it, and nobody else could.
    assert_eq!(poll_one(&node, &ticket).await, DepositStatus::Waiting);
    let sent = sent_of(&ticket);
    assert_eq!(sent.sender_master_did, node.sender_cert.master_did);
    assert!(matches!(node.receipt_for(&sent).await, Receipt::Stored { .. }));
}

#[tokio::test]
async fn a_deposit_context_stays_small() {
    let node = node(vec![MockOutcome::Transport]).await;

    let ticket = deposited(&node).await;

    assert!(ticket.context.len() < 1024, "{} bytes", ticket.context.len());
}

// -- polling ----------------------------------------------------------------

#[tokio::test]
async fn poll_completes_on_a_signed_delivery() {
    let node = node(vec![MockOutcome::Transport]).await;
    let ticket = deposited(&node).await;

    node.receiver_delivers().await;

    assert_eq!(poll_one(&node, &ticket).await, DepositStatus::Delivered);
}

#[tokio::test]
async fn poll_fails_on_a_signed_not_eligible_rejection() {
    let node = node(vec![MockOutcome::Transport]).await;
    let ticket = deposited(&node).await;

    node.receiver_rejects_as_not_eligible().await;

    assert_eq!(
        poll_one(&node, &ticket).await,
        DepositStatus::Rejected { code: "not_eligible".to_string() }
    );
}

#[tokio::test]
async fn poll_retries_on_an_unsigned_rejection_expiry_or_an_unknown_state() {
    let node = node(vec![MockOutcome::Transport]).await;
    let sent = sent_of(&deposited(&node).await);
    let at_secs = now_secs();
    let receipts = [
        Receipt::Rejected { at_secs, code: "malformed".to_string(), proof: None },
        Receipt::Expired { at_secs },
        Receipt::Unknown,
    ];
    for receipt in receipts {
        let status =
            node.router.judge(Some(&receipt), &sent, at_secs, &mut AnchorCache::new()).await;
        assert!(is_retry(&status), "{receipt:?}: {status:?}");
    }
    assert!(is_retry(&node.router.judge(None, &sent, at_secs, &mut AnchorCache::new()).await));
}

#[tokio::test]
async fn poll_retries_when_the_receiver_key_is_revoked() {
    let node = node(vec![MockOutcome::Transport]).await;
    let ticket = deposited(&node).await;
    node.receiver_delivers().await;
    node.router_anchors.revoked.lock().unwrap().push(node.receiver.cert.temporary_did.clone());

    assert!(is_retry(&poll_one(&node, &ticket).await));
}

#[tokio::test]
async fn poll_waits_when_the_receiver_anchor_cannot_be_read() {
    let node = node(vec![MockOutcome::Transport]).await;
    let ticket = deposited(&node).await;
    node.receiver_delivers().await;
    node.router_anchors.unreadable.store(true, Ordering::SeqCst);

    assert_eq!(poll_one(&node, &ticket).await, DepositStatus::Waiting);
}

#[tokio::test]
async fn poll_retries_on_a_receipt_for_another_message() {
    let node = node(vec![MockOutcome::Transport, MockOutcome::Transport]).await;
    let first = sent_of(&deposited(&node).await);
    let second = sent_of(&deposited(&node).await);
    node.receiver_delivers().await;
    let receipt_of_second = node.receipt_for(&second).await;

    let status = node
        .router
        .judge(Some(&receipt_of_second), &first, now_secs(), &mut AnchorCache::new())
        .await;

    assert!(is_retry(&status), "{status:?}");
}

#[tokio::test]
async fn poll_waits_when_the_mailbox_is_unreachable() {
    let node = node(vec![MockOutcome::Transport]).await;
    let ticket = deposited(&node).await;
    let mut context: Value = serde_json::from_slice(&ticket.context).unwrap();
    context["mailbox"] = json!("did:key:zNobodyHostsThis");
    let moved = serde_json::to_vec(&context).unwrap();

    assert_eq!(poll(&node, vec![moved]).await, vec![DepositStatus::Waiting]);
}

#[tokio::test]
async fn poll_waits_when_the_sender_has_no_valid_certificate() {
    let node = node(vec![MockOutcome::Transport]).await;
    let ticket = deposited(&node).await;

    let out = node.router.poll_deposits_inner("did:key:zUnknownSender", &[ticket.context]).await;

    assert_eq!(out, vec![DepositStatus::Waiting]);
}

#[tokio::test]
async fn poll_retries_on_an_unreadable_context() {
    let node = node(vec![]).await;

    let out = poll(&node, vec![b"not json".to_vec()]).await;

    assert!(is_retry(&out[0]));
}

#[tokio::test]
async fn poll_splits_more_than_one_hundred_ids_into_batches() {
    let node = node(vec![MockOutcome::Transport]).await;
    let ticket = deposited(&node).await;
    let contexts = vec![ticket.context.clone(); 101];

    let out = poll(&node, contexts).await;

    assert_eq!(out.len(), 101);
    assert!(out.iter().all(|s| *s == DepositStatus::Waiting));
    assert_eq!(node.mailbox.count("status"), 2);
}

#[tokio::test]
async fn poll_reads_a_receivers_anchor_once_for_all_its_receipts() {
    let node = node(vec![MockOutcome::Transport; 3]).await;
    let mut contexts = Vec::new();
    for _ in 0..3 {
        contexts.push(deposited(&node).await.context);
    }
    node.receiver_delivers().await;

    let out = poll(&node, contexts).await;

    assert_eq!(out, vec![DepositStatus::Delivered; 3]);
    assert_eq!(node.router_anchors.lookups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn poll_asks_an_unreadable_anchor_once_and_waits_for_every_receipt() {
    let node = node(vec![MockOutcome::Transport; 3]).await;
    let mut contexts = Vec::new();
    for _ in 0..3 {
        contexts.push(deposited(&node).await.context);
    }
    node.receiver_delivers().await;
    node.router_anchors.unreadable.store(true, Ordering::SeqCst);

    let out = poll(&node, contexts).await;

    assert_eq!(out, vec![DepositStatus::Waiting; 3]);
    assert_eq!(node.router_anchors.lookups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_mailbox_that_is_a_local_guest_is_never_used() {
    let node = node(vec![MockOutcome::Transport]).await;
    let context = deposited(&node).await.context;
    node.registry
        .register(
            node.mailbox_did.clone(),
            "mailbox".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: node.mailbox_did.clone() },
        )
        .await
        .unwrap();
    node.hop.outcomes.lock().unwrap().push_back(MockOutcome::Transport);

    let delivery = node.router.deliver_to_peer(&node.request(), node.peer(true), WEEK_SECS).await;

    assert!(matches!(delivery, Err(ProxyError::Transport(_))), "{delivery:?}");
    assert_eq!(node.mailbox.count("deposit"), 1, "only the first, native one");
    assert_eq!(poll(&node, vec![context]).await, vec![DepositStatus::Waiting]);
    assert_eq!(node.mailbox.count("status"), 0, "a guest is never asked");
}

#[tokio::test]
async fn a_mailbox_refusal_leaves_the_call_on_direct_delivery() {
    let refusals = [
        codes::UNKNOWN_DESTINATION,
        codes::TOO_LARGE,
        codes::QUEUE_FULL,
        codes::RATE_LIMITED,
        codes::MAILBOX_FULL,
        codes::NOT_AN_INSTANCE,
        codes::UNAUTHENTICATED,
    ];
    for code in refusals {
        let node = node(vec![MockOutcome::Transport]).await;
        node.mailbox.refuse_deposits_with(code);

        let delivery =
            node.router.deliver_to_peer(&node.request(), node.peer(true), WEEK_SECS).await;

        // The caller gets what the direct attempt said, not the mailbox's code.
        assert!(matches!(delivery, Err(ProxyError::Transport(_))), "{code}: {delivery:?}");
        assert_eq!(node.mailbox.count("deposit"), 1, "{code}: one deposit, no reseal");
    }
}
