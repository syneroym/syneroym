//! One claimed message: open it, check the sender, hand it to the service
//! and make the receipt.

use syneroym_core::mailbox::{AckEntry, ClaimedMessage};
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_mailbox_format::{
    OpenError, OpenPolicy, OpenedMessage, Receipt, ReceiptError, RejectCode, open,
};
use syneroym_rpc::{AuthLevel, CallerContext, CallerProof, DEFAULT_PROXY_CALL_TIMEOUT};
use tokio::time;
use tracing::{debug, warn};

use super::{
    MailboxPuller,
    service::{Inputs, ServicePass},
};
use crate::{handshake::now_secs, route_handler::deferred::DeferredOutcome};

/// Which counter an acked message adds to, once the mailbox stored its
/// receipt.
#[derive(Clone, Copy)]
pub(super) enum Count {
    Delivered,
    Rejected,
}

/// What to do about one message.
enum Handled {
    Ack(Receipt, Count),
    /// Leave it unacked: it comes back after the visibility time.
    NoAck(String),
}

/// The caller a delivered call presents: the sender's master, with the
/// instance key and certificate as proof. No capabilities, and no stream
/// signature, so a receiving service cannot pass the proof on.
fn delivery_caller(opened: &OpenedMessage, now: u64) -> anyhow::Result<CallerContext> {
    let certificate: &DelegationCertificate = &opened.sender_certificate;
    let key = substrate::resolve_did_key(&certificate.temporary_did)?;
    let mut caller =
        CallerContext::verified(&certificate.master_did, AuthLevel::Delegated, Vec::new());
    caller.session.verified_at_secs = now;
    caller.proof = Some(CallerProof {
        pubkey_hex: hex::encode(key.to_bytes()),
        delegation_json: certificate.to_json().ok(),
        signed: None,
        on_behalf: None,
    });
    Ok(caller)
}

fn receipt_of(receipt: Result<Receipt, ReceiptError>, count: Count) -> Handled {
    match receipt {
        Ok(receipt) => Handled::Ack(receipt, count),
        Err(e) => {
            warn!(error = %e, "could not sign a receipt; the message stays unacked");
            Handled::NoAck("signing failed".into())
        }
    }
}

/// A signed rejection for a check the receiver made itself.
fn reject(
    opened: &OpenedMessage,
    me: &Identity,
    certificate: &DelegationCertificate,
    code: RejectCode,
    now: u64,
) -> Handled {
    receipt_of(opened.sign_rejection(me, certificate, code, now), Count::Rejected)
}

impl MailboxPuller {
    /// Handles one message. The ack to send, if any, and how to count it
    /// once the mailbox stored it.
    pub(super) async fn handle_one(
        &self,
        pass: &mut ServicePass<'_>,
        inputs: &Inputs,
        claimed: &ClaimedMessage,
        policy: OpenPolicy,
    ) -> Option<(AckEntry, Count)> {
        match self.handle(pass, inputs, claimed, policy, now_secs()).await {
            Handled::Ack(receipt, count) => {
                let entry = AckEntry {
                    sender: claimed.sender.clone(),
                    deposit_id: claimed.deposit_id,
                    receipt,
                };
                Some((entry, count))
            }
            Handled::NoAck(why) => {
                debug!(service_id = pass.service_id, why, "message left unacked");
                pass.summary.not_acked += 1;
                metrics::counter!("substrate.mailbox_receiver.messages", "result" => "not_acked")
                    .increment(1);
                None
            }
        }
    }

    async fn handle(
        &self,
        pass: &mut ServicePass<'_>,
        inputs: &Inputs,
        claimed: &ClaimedMessage,
        policy: OpenPolicy,
        now: u64,
    ) -> Handled {
        let me = inputs.instance.key.as_ref();
        let certificate = &inputs.instance.certificate;
        let opened = match open(
            &inputs.key,
            pass.service_id,
            &claimed.deposit_id,
            &claimed.sealed,
            policy,
            now,
        ) {
            Ok(opened) => opened,
            Err(OpenError::Refused(refused)) => {
                if refused.code == RejectCode::FromFuture {
                    warn!(
                        service_id = pass.service_id,
                        "a message is dated in the future; check the clock of this node"
                    );
                }
                return receipt_of(refused.receipt(me, certificate, now), Count::Rejected);
            }
            Err(e) => {
                let receipt = Receipt::unsigned_rejection(e.reject_code(), now);
                return Handled::Ack(receipt, Count::Rejected);
            }
        };
        if opened.sender_master_did() != claimed.sender {
            debug!("the depositor differs from the sealed sender; the sealed sender is used");
        }
        if let Some(handled) = self.screen(pass, &opened, me, certificate, now).await {
            return handled;
        }
        self.deliver(pass, &opened, me, certificate, now).await
    }

    /// The checks before delivery. `Some` ends the handling of the message.
    async fn screen(
        &self,
        pass: &mut ServicePass<'_>,
        opened: &OpenedMessage,
        me: &Identity,
        certificate: &DelegationCertificate,
        now: u64,
    ) -> Option<Handled> {
        match self.proxy.revoked_keys(opened.sender_master_did(), pass.anchors).await {
            None => return Some(Handled::NoAck("the sender's anchor cannot be read".into())),
            Some(revoked) if revoked.iter().any(|k| k == opened.sender_instance_did()) => {
                return Some(reject(opened, me, certificate, RejectCode::Revoked, now));
            }
            Some(_) => {}
        }
        // The fence cannot run a call without a key.
        let not_eligible = opened.content.idempotency_key.is_empty()
            || self
                .canonical_interface(pass, opened)
                .is_none_or(|interface| !pass.fact.accepts(&interface, &opened.content.method));
        not_eligible.then(|| reject(opened, me, certificate, RejectCode::NotEligible, now))
    }

    /// The interface name the service serves, whatever form the sender used.
    fn canonical_interface(
        &self,
        pass: &ServicePass<'_>,
        opened: &OpenedMessage,
    ) -> Option<String> {
        self.registry
            .lookup(pass.service_id, &opened.content.interface)
            .map(|(_, interface)| interface)
    }

    async fn deliver(
        &self,
        pass: &ServicePass<'_>,
        opened: &OpenedMessage,
        me: &Identity,
        certificate: &DelegationCertificate,
        now: u64,
    ) -> Handled {
        let Some(interface) = self.canonical_interface(pass, opened) else {
            return reject(opened, me, certificate, RejectCode::NotEligible, now);
        };
        let Ok(caller) = delivery_caller(opened, now) else {
            return Handled::NoAck("the sender key does not resolve".into());
        };
        // A task of its own, so a timeout here stops only the wait. The call
        // is never cut off halfway: it finishes and settles its fence claim,
        // and the next claim of the message gets its answer.
        let dispatch = self.dispatch.clone();
        let (service_id, content) = (pass.service_id.to_string(), opened.content.clone());
        let delivery = tokio::spawn(async move {
            dispatch.dispatch_deferred(&service_id, &interface, &content, &caller).await
        });
        match time::timeout(DEFAULT_PROXY_CALL_TIMEOUT, delivery).await {
            Err(_) => Handled::NoAck("the service did not answer in time".into()),
            Ok(Err(e)) => Handled::NoAck(format!("the delivery task failed: {e}")),
            Ok(Ok(outcome)) => answer_receipt(outcome, opened, me, certificate, now),
        }
    }
}

/// What to ack for the service's answer.
fn answer_receipt(
    outcome: DeferredOutcome,
    opened: &OpenedMessage,
    me: &Identity,
    certificate: &DelegationCertificate,
    now: u64,
) -> Handled {
    match outcome {
        DeferredOutcome::Succeeded => {
            receipt_of(opened.sign_delivered(me, certificate, now), Count::Delivered)
        }
        DeferredOutcome::Failed { code } => {
            // The sender goes back to direct attempts and gets the same
            // answer from the receiver fence there. A signed rejection
            // would let a mailbox end a message by delaying it.
            debug!(code, "the service answered an error");
            let receipt = Receipt::unsigned_rejection(RejectCode::CallFailed, now);
            Handled::Ack(receipt, Count::Rejected)
        }
        DeferredOutcome::NotServed => reject(opened, me, certificate, RejectCode::NotEligible, now),
        DeferredOutcome::NotRun(why) => Handled::NoAck(why),
    }
}
