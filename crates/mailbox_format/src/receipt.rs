//! The receipt a sender reads back from the mailbox, and the receiver's
//! signature on it.
//!
//! A receipt the receiver signs names the deposit id, the sender and a hash of
//! the exact message. The sender rebuilds that statement from what it sent,
//! so a receipt for another message does not verify, and a mailbox cannot
//! make a delivery of a message it put in the queue itself.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_identity::{
    DelegationCertificate, Identity,
    delegation::SCOPE_SERVICE_INSTANCE,
    substrate::{self, verify_json_signature},
};

use crate::{
    RECEIPT_DOMAIN,
    deposit::DepositId,
    error::ReceiptError,
    message::{MessageHash, OpenedMessage, SentDeposit},
};

/// Why a message was closed without delivery. On the wire a code is a plain
/// string, so a code from a newer version survives a round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RejectCode {
    // Caused by the sender, found after its certificate verified: the
    // receiver may sign these.
    FromFuture,
    Revoked,
    NotEligible,
    // The mailbox could cause these, or the sender is not proven: never
    // signed.
    Malformed,
    Undecryptable,
    BadSignature,
    TooLarge,
    BadCertificate,
    TooOld,
    TooManyClaims,
}

impl RejectCode {
    pub const ALL: [RejectCode; 10] = [
        Self::FromFuture,
        Self::Revoked,
        Self::NotEligible,
        Self::Malformed,
        Self::Undecryptable,
        Self::BadSignature,
        Self::TooLarge,
        Self::BadCertificate,
        Self::TooOld,
        Self::TooManyClaims,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FromFuture => "from_future",
            Self::Revoked => "revoked",
            Self::NotEligible => "not_eligible",
            Self::Malformed => "malformed",
            Self::Undecryptable => "undecryptable",
            Self::BadSignature => "bad_signature",
            Self::TooLarge => "too_large",
            Self::BadCertificate => "bad_certificate",
            Self::TooOld => "too_old",
            Self::TooManyClaims => "too_many_claims",
        }
    }

    /// The code for exactly one of the strings of `as_str`, else `None`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|code| code.as_str() == text)
    }

    /// True only for a failure the mailbox cannot cause and whose sender is
    /// proven: a check made after the sender's certificate verified, on
    /// content the sender signed or on the receiver's own rules. Only these
    /// may be signed, and only a signed one ends a message.
    #[must_use]
    pub fn is_sender_fault(self) -> bool {
        matches!(self, Self::FromFuture | Self::Revoked | Self::NotEligible)
    }
}

/// The receiver's signature over a receipt, with the certificate that ties
/// its instance key to the receiving service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverProof {
    pub receiver_certificate: DelegationCertificate,
    /// z-base-32 Ed25519 signature over the receipt statement.
    pub signature: String,
}

/// What the mailbox answers to `status` for one deposit id. The mailbox
/// stores a receiver-signed receipt as it got it; it cannot make one.
///
/// There is no `deny_unknown_fields`: a sender reads receipts from mailboxes
/// of other operators, which may run a newer version.
///
/// `Unknown` covers an unknown `state` only. A known state whose body is
/// missing a field does not parse at all. The caller treats an answer that
/// does not parse like `expired`: back to direct attempts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Receipt {
    Stored {
        at_secs: u64,
    },
    Delivered {
        at_secs: u64,
        proof: ReceiverProof,
    },
    Rejected {
        at_secs: u64,
        /// A `RejectCode` string, or a code from a newer version.
        code: String,
        /// Present only for a sender-caused code the receiver signed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proof: Option<ReceiverProof>,
    },
    Expired {
        at_secs: u64,
    },
    /// The mailbox has no receipt for this id (never stored, or its receipt
    /// time ended), or it sent a state this version does not know.
    #[serde(other)]
    Unknown,
}

/// The two outcomes a receiver signs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedOutcome {
    Delivered,
    Rejected(RejectCode),
}

/// `state` is `"delivered"` or `"rejected"`. `code` is set only for
/// `"rejected"`, as the raw wire string.
fn receipt_statement(
    deposit_id: &DepositId,
    sender: &str,
    message_hash: &MessageHash,
    state: &str,
    code: Option<&str>,
    at_secs: u64,
) -> Value {
    let mut statement = json!({
        "domain": RECEIPT_DOMAIN,
        "deposit_id": deposit_id.to_string(),
        "sender": sender,
        "message_hash": message_hash.to_string(),
        "state": state,
        "at_secs": at_secs,
    });
    if let (Some(code), Some(map)) = (code, statement.as_object_mut()) {
        map.insert("code".to_string(), Value::String(code.to_string()));
    }
    statement
}

/// Signs a receipt for `message` with the receiver's instance key.
pub(crate) fn sign(
    instance: &Identity,
    certificate: &DelegationCertificate,
    message: &OpenedMessage,
    outcome: SignedOutcome,
    at_secs: u64,
) -> Result<Receipt, ReceiptError> {
    if let SignedOutcome::Rejected(code) = outcome
        && !code.is_sender_fault()
    {
        return Err(ReceiptError::NotSignable);
    }
    if substrate::derive_did_key(&instance.public_key()) != certificate.temporary_did {
        return Err(ReceiptError::SignerMismatch);
    }
    certificate
        .verify_at(&certificate.master_did, &[SCOPE_SERVICE_INSTANCE], at_secs)
        .map_err(|e| ReceiptError::BadCertificate(e.to_string()))?;
    let (state, code) = match outcome {
        SignedOutcome::Delivered => ("delivered", None),
        SignedOutcome::Rejected(code) => ("rejected", Some(code.as_str())),
    };
    let statement = receipt_statement(
        &message.deposit_id,
        message.sender_master_did(),
        &message.message_hash,
        state,
        code,
        at_secs,
    );
    let signature =
        instance.sign_json(&statement).map_err(|e| ReceiptError::Json(e.to_string()))?;
    let proof = ReceiverProof { receiver_certificate: certificate.clone(), signature };
    Ok(match outcome {
        SignedOutcome::Delivered => Receipt::Delivered { at_secs, proof },
        SignedOutcome::Rejected(code) => {
            Receipt::Rejected { at_secs, code: code.as_str().to_string(), proof: Some(proof) }
        }
    })
}

impl Receipt {
    /// A rejection with no receiver signature. For the receiver's ack of a
    /// failure the mailbox could have caused, and for the mailbox's own
    /// `too_many_claims`. A sender never counts it as final.
    #[must_use]
    pub fn unsigned_rejection(code: RejectCode, at_secs: u64) -> Self {
        Self::Rejected { at_secs, code: code.as_str().to_string(), proof: None }
    }

    /// Checks a `Delivered` or `Rejected` receipt against what the sender
    /// sent. Every value in the rebuilt statement comes from `sent`, never
    /// from the receipt, so a receipt for another message fails.
    ///
    /// Any `Err` means the sender handles the receipt like `expired`: it
    /// returns to direct attempts. Such a receipt is a bug or a dishonest
    /// mailbox, and it must never count as delivered or as a final rejection.
    pub fn verify_signed(
        &self,
        sent: &SentDeposit,
        max_clock_skew_secs: u64,
        now_secs: u64,
    ) -> Result<SignedOutcome, ReceiptError> {
        let (outcome, state, code, at_secs, proof) = match self {
            Self::Delivered { at_secs, proof } => {
                (SignedOutcome::Delivered, "delivered", None, *at_secs, proof)
            }
            Self::Rejected { at_secs, code, proof: Some(proof) } => {
                let known = RejectCode::parse(code)
                    .filter(|c| c.is_sender_fault())
                    .ok_or(ReceiptError::NotSignable)?;
                (SignedOutcome::Rejected(known), "rejected", Some(code.as_str()), *at_secs, proof)
            }
            _ => return Err(ReceiptError::Unsigned),
        };
        let certificate = &proof.receiver_certificate;
        if certificate.master_did != sent.receiver_service_id {
            return Err(ReceiptError::WrongReceiver);
        }
        certificate
            .verify_at(&sent.receiver_service_id, &[SCOPE_SERVICE_INSTANCE], at_secs)
            .map_err(|e| ReceiptError::BadCertificate(e.to_string()))?;
        let statement = receipt_statement(
            &sent.deposit_id,
            &sent.sender_master_did,
            &sent.message_hash,
            state,
            code,
            at_secs,
        );
        verify_json_signature(&certificate.temporary_did, &statement, &proof.signature)
            .map_err(|_| ReceiptError::BadSignature)?;
        if at_secs > now_secs.saturating_add(max_clock_skew_secs) {
            return Err(ReceiptError::FromFuture);
        }
        // A receipt is signed after the message was sent. Without this bound,
        // an old receiver key whose certificate window has long ended could
        // still sign a receipt for a message it never delivered.
        if at_secs.saturating_add(max_clock_skew_secs) < sent.sent_at_secs {
            return Err(ReceiptError::BeforeSend);
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests;
