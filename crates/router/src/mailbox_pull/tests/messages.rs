use std::{sync::atomic::Ordering, time::Duration};

use syneroym_core::{local_registry::SubstrateEndpoint, util};
use syneroym_identity::DelegationCertificate;
use syneroym_mailbox_format::{
    DepositId, MAX_CLOCK_SKEW_SECS, MailboxSecretKey, MessageContent, Receipt, ReceiptError,
    Recipient, RejectCode, Sender, SentDeposit, SignedOutcome, seal, seal_unchecked,
};
use tokio::time;

use super::helpers::*;
use crate::{handshake::now_secs, route_handler::deferred::DeferredOutcome};

/// Registers the queue, deposits `content` as a new sender and runs the pass
/// that handles it. Returns the sender, the deposit and the receipt.
async fn one_message(
    node: &PullNode,
    content: &MessageContent,
) -> (SenderMember, SentDeposit, Receipt) {
    node.pass().await;
    let sender = SenderMember::new();
    let sent = node.deposit(&sender, content).await;
    node.pass().await;
    let receipt = node.receipt(&sender, &sent).await;
    (sender, sent, receipt)
}

fn outcome_of(receipt: &Receipt, sent: &SentDeposit) -> Result<SignedOutcome, ReceiptError> {
    receipt.verify_signed(sent, MAX_CLOCK_SKEW_SECS, now_secs())
}

#[tokio::test]
async fn a_message_is_delivered_as_its_sender_and_acked_delivered() {
    let node = pull_node(&["greeter#greet"]).await;
    let (sender, sent, receipt) = one_message(&node, &greet()).await;

    assert_eq!(outcome_of(&receipt, &sent), Ok(SignedOutcome::Delivered));
    assert_eq!(node.dispatch.call_count(), 1);
    let call = node.dispatch.last();
    assert_eq!((call.interface.as_str(), call.method.as_str()), ("greeter", "greet"));
    assert_eq!(call.caller.caller_did, sender.did());
    assert!(call.caller.session.capabilities.is_empty());
    let proof = call.caller.proof.expect("the sender's proof");
    assert_eq!(proof.pubkey_hex, hex::encode(sender.instance.public_key().to_bytes()));
    let cert: DelegationCertificate =
        serde_json::from_str(&proof.delegation_json.unwrap()).unwrap();
    assert_eq!(cert, sender.cert);
    assert!(proof.signed.is_none(), "an unsigned proof cannot be passed on");
    assert!(proof.on_behalf.is_none());
}

#[tokio::test]
async fn a_sender_certificate_that_expired_after_the_send_time_is_still_delivered() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    let sender = SenderMember::with_cert_life(2);
    node.deposit(&sender, &greet()).await;
    time::sleep(Duration::from_secs(3)).await;
    assert!(sender.cert.is_expired());

    // The mailbox no longer admits the sender, so the receipt cannot be read
    // back here; the pass and the delivery show the outcome.
    let summary = node.pass().await;

    assert_eq!(summary.delivered, 1);
    assert_eq!(node.dispatch.call_count(), 1);
    assert!(node.dispatch.last().caller.proof.is_some());
}

#[tokio::test]
async fn a_sender_certificate_that_does_not_verify_is_rejected_unsigned() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    let depositor = SenderMember::new();
    // The certificate names a master that never signed it.
    let attacker = SenderMember::new();
    let mut forged = attacker.cert.clone();
    forged.master_did = depositor.did();
    let key = node.vault.key.public_key();
    let recipient = Recipient { service_id: &node.service_id, mailbox_key: &key };
    let sealed = seal_unchecked(
        &greet(),
        recipient,
        Sender { instance: &attacker.instance, certificate: &forged },
        DepositId::random().unwrap(),
        WEEK_SECS,
        now_secs(),
    )
    .unwrap();
    node.deposit_sealed(&depositor, &sealed).await;

    node.pass().await;

    let receipt = node.receipt(&depositor, &sealed.sent).await;
    assert_eq!(
        receipt,
        Receipt::unsigned_rejection(RejectCode::BadCertificate, rejected_at(&receipt))
    );
    assert_eq!(outcome_of(&receipt, &sealed.sent), Err(ReceiptError::Unsigned));
    assert_eq!(node.dispatch.call_count(), 0);
}

fn rejected_at(receipt: &Receipt) -> u64 {
    match receipt {
        Receipt::Rejected { at_secs, .. } => *at_secs,
        other => panic!("not a rejection: {other:?}"),
    }
}

#[tokio::test]
async fn an_error_answer_is_rejected_unsigned_as_call_failed() {
    let node = pull_node(&["greeter#greet"]).await;
    node.dispatch.script(DeferredOutcome::Failed { code: -32602 });

    let (_, sent, receipt) = one_message(&node, &greet()).await;

    assert_eq!(receipt, Receipt::unsigned_rejection(RejectCode::CallFailed, rejected_at(&receipt)));
    assert_eq!(outcome_of(&receipt, &sent), Err(ReceiptError::Unsigned));
}

#[tokio::test]
async fn a_method_that_did_not_opt_in_is_rejected_signed() {
    let node = pull_node(&[]).await;

    let (_, sent, receipt) = one_message(&node, &greet()).await;

    assert_eq!(outcome_of(&receipt, &sent), Ok(SignedOutcome::Rejected(RejectCode::NotEligible)));
    assert_eq!(node.dispatch.call_count(), 0);
}

#[tokio::test]
async fn the_conversation_methods_are_always_accepted() {
    let node = pull_node(&[]).await;
    node.registry
        .register(
            node.service_id.clone(),
            "conversation".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: node.service_id.clone() },
        )
        .await
        .unwrap();
    for (method, expected) in [
        ("deliver", SignedOutcome::Delivered),
        ("group-push", SignedOutcome::Delivered),
        ("send", SignedOutcome::Rejected(RejectCode::NotEligible)),
    ] {
        let content = MessageContent {
            interface: "conversation".to_string(),
            method: method.to_string(),
            params: serde_json::json!({}),
            idempotency_key: format!("key-{method}"),
        };
        let (_, sent, receipt) = one_message(&node, &content).await;
        assert_eq!(outcome_of(&receipt, &sent), Ok(expected), "{method}");
    }
}

#[tokio::test]
async fn a_short_hash_interface_matches_its_listed_name() {
    let node = pull_node(&["greeter#greet"]).await;
    let content = MessageContent { interface: util::short_hash("greeter"), ..greet() };

    let (_, sent, receipt) = one_message(&node, &content).await;

    assert_eq!(outcome_of(&receipt, &sent), Ok(SignedOutcome::Delivered));
    assert_eq!(node.dispatch.last().interface, "greeter", "the service gets the canonical name");
}

#[tokio::test]
async fn an_entry_that_names_the_hash_matches_the_canonical_interface() {
    let entry = format!("{}#greet", util::short_hash("greeter"));
    let node = pull_node(&[entry.as_str()]).await;

    let (_, sent, receipt) = one_message(&node, &greet()).await;

    assert_eq!(outcome_of(&receipt, &sent), Ok(SignedOutcome::Delivered));
}

#[tokio::test]
async fn an_empty_key_is_rejected_signed() {
    let node = pull_node(&["greeter#greet"]).await;

    let (_, sent, receipt) = one_message(&node, &with_key("")).await;

    assert_eq!(outcome_of(&receipt, &sent), Ok(SignedOutcome::Rejected(RejectCode::NotEligible)));
    assert_eq!(node.dispatch.call_count(), 0);
}

#[tokio::test]
async fn a_revoked_sender_is_rejected_signed() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    let sender = SenderMember::new();
    let sent = node.deposit(&sender, &greet()).await;
    node.anchors.revoked.lock().unwrap().push(sender.cert.temporary_did.clone());

    node.pass().await;

    let receipt = node.receipt(&sender, &sent).await;
    assert_eq!(outcome_of(&receipt, &sent), Ok(SignedOutcome::Rejected(RejectCode::Revoked)));
    assert_eq!(node.dispatch.call_count(), 0);
}

#[tokio::test]
async fn an_unreadable_anchor_leaves_the_message_unacked() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    let sender = SenderMember::new();
    let sent = node.deposit(&sender, &greet()).await;
    node.anchors.unreadable.store(true, Ordering::SeqCst);

    let summary = node.pass().await;

    assert_eq!(summary.not_acked, 1);
    assert!(matches!(node.receipt(&sender, &sent).await, Receipt::Stored { .. }));
    assert_eq!(node.dispatch.call_count(), 0);
}

#[tokio::test]
async fn a_temporary_failure_is_not_acked() {
    let node = pull_node(&["greeter#greet"]).await;
    node.dispatch.script(DeferredOutcome::NotRun("the call is running".to_string()));

    let (_, _, receipt) = one_message(&node, &greet()).await;

    assert!(matches!(receipt, Receipt::Stored { .. }), "{receipt:?}");
}

#[tokio::test]
async fn a_tcp_service_rejects_signed() {
    let node = pull_node(&["greeter#greet"]).await;
    node.dispatch.script(DeferredOutcome::NotServed);

    let (_, sent, receipt) = one_message(&node, &greet()).await;

    assert_eq!(outcome_of(&receipt, &sent), Ok(SignedOutcome::Rejected(RejectCode::NotEligible)));
}

#[tokio::test]
async fn a_message_for_another_key_is_rejected_unsigned() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    let sender = SenderMember::new();
    let other_key = MailboxSecretKey::generate().unwrap().public_key();
    let recipient = Recipient { service_id: &node.service_id, mailbox_key: &other_key };
    let sealed = seal(
        &greet(),
        recipient,
        Sender { instance: &sender.instance, certificate: &sender.cert },
        WEEK_SECS,
        now_secs(),
    )
    .unwrap();
    node.deposit_sealed(&sender, &sealed).await;

    node.pass().await;

    let receipt = node.receipt(&sender, &sealed.sent).await;
    assert_eq!(
        receipt,
        Receipt::unsigned_rejection(RejectCode::Undecryptable, rejected_at(&receipt))
    );
}

#[tokio::test]
async fn a_message_from_the_future_is_rejected_signed() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    // A certificate that is valid at the (late) send time.
    let sender = SenderMember::with_cert_life(3 * 24 * 3600);
    let key = node.vault.key.public_key();
    let recipient = Recipient { service_id: &node.service_id, mailbox_key: &key };
    let sealed = seal(
        &greet(),
        recipient,
        Sender { instance: &sender.instance, certificate: &sender.cert },
        WEEK_SECS,
        now_secs() + MAX_CLOCK_SKEW_SECS + 3600,
    )
    .unwrap();
    node.deposit_sealed(&sender, &sealed).await;

    node.pass().await;

    // The receiver signs it. The sender cannot accept it: the receipt time
    // is more than the clock allowance before the send time, so the sender
    // goes back to direct attempts.
    let receipt = node.receipt(&sender, &sealed.sent).await;
    assert!(
        matches!(&receipt, Receipt::Rejected { code, proof: Some(_), .. } if code == "from_future"),
        "{receipt:?}"
    );
    assert_eq!(outcome_of(&receipt, &sealed.sent), Err(ReceiptError::BeforeSend));
    assert_eq!(node.dispatch.call_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_delivery_past_its_timeout_is_not_cut_off() {
    let node = pull_node(&["greeter#greet"]).await;
    node.pass().await;
    let sender = SenderMember::new();
    let sent = node.deposit(&sender, &greet()).await;
    *node.dispatch.delay.lock().unwrap() = Duration::from_secs(40);

    let summary = node.pass().await;

    assert_eq!(summary.not_acked, 1);
    assert!(matches!(node.receipt(&sender, &sent).await, Receipt::Stored { .. }));
    assert_eq!(node.dispatch.finished.load(Ordering::SeqCst), 0);
    // The call goes on after the wait stopped, and settles its fence claim.
    time::sleep(Duration::from_secs(15)).await;
    assert_eq!(node.dispatch.finished.load(Ordering::SeqCst), 1);
}
