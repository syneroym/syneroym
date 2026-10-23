//! Sealing and opening a message: the call a sender defers and the receiver
//! later runs.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use syneroym_identity::{
    DelegationCertificate, Identity,
    delegation::SCOPE_SERVICE_INSTANCE,
    substrate::{self, canonicalize_json_value, verify_json_signature},
};

use crate::{
    MAX_CLOCK_SKEW_SECS, MAX_SEALED_BYTES, MESSAGE_DOMAIN, OPEN_MAX_TTL_SECS,
    crypto::{self, CryptoError},
    deposit::{Deposit, DepositId, SealedBytes},
    error::{OpenError, ReceiptError, SealError},
    keys::{MailboxPublicKey, MailboxSecretKey},
    receipt::{self, Receipt, RejectCode, SignedOutcome},
};

/// The call a sender seals. Only the receiver reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageContent {
    pub interface: String,
    pub method: String,
    pub params: Value,
    pub idempotency_key: String,
}

/// SHA-256 of the canonical message statement: the exact message a
/// receipt is about. 64 lowercase hex characters on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct MessageHash([u8; 32]);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MessageHashParseError {
    #[error("a message hash is 64 lowercase hex characters")]
    Invalid,
}

/// The receiving service member: its id and its published mailbox key.
#[derive(Debug, Clone, Copy)]
pub struct Recipient<'a> {
    pub service_id: &'a str,
    pub mailbox_key: &'a MailboxPublicKey,
}

/// The sending service instance: its instance key and the certificate by
/// which its member master allows that key (ADR-0020).
#[derive(Debug, Clone, Copy)]
pub struct Sender<'a> {
    pub instance: &'a Identity,
    pub certificate: &'a DelegationCertificate,
}

/// What the sender keeps after a deposit, to check the receipt later. It
/// never holds the sealed bytes: a later deposit is a new `seal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentDeposit {
    pub deposit_id: DepositId,
    pub receiver_service_id: String,
    pub sender_master_did: String,
    pub message_hash: MessageHash,
    /// The send time. A receipt cannot be older than this (`verify_signed`).
    pub sent_at_secs: u64,
}

/// The result of `seal`: what goes to the mailbox, and what the sender keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub deposit: Deposit,
    pub sent: SentDeposit,
}

/// Limits the receiver applies to the send time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenPolicy {
    /// The longest time-to-live of the receiver's mailbox queue.
    pub max_ttl_secs: u64,
    pub max_clock_skew_secs: u64,
}

impl Default for OpenPolicy {
    fn default() -> Self {
        Self { max_ttl_secs: OPEN_MAX_TTL_SECS, max_clock_skew_secs: MAX_CLOCK_SKEW_SECS }
    }
}

/// A message that opened and passed every check of this crate. Revocation
/// is not checked yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedMessage {
    pub content: MessageContent,
    pub sender_certificate: DelegationCertificate,
    pub sent_at_secs: u64,
    pub deposit_id: DepositId,
    pub message_hash: MessageHash,
}

/// A message signed by the key its certificate names, that failed a check
/// on the sender's side: the certificate itself, or the send time. It must
/// not be delivered. `receipt` makes the ack for it. Its sender is proven
/// only when `code` is not `BadCertificate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedMessage {
    pub code: RejectCode,
    pub detail: String,
    message: OpenedMessage,
}

/// The plaintext before sealing.
///
/// Unknown fields are ignored: the sender may run a newer version, and every
/// field that carries meaning is under the sender signature.
#[derive(Serialize, Deserialize)]
struct SealedContent {
    interface: String,
    method: String,
    params: Value,
    idempotency_key: String,
    sent_at_secs: u64,
    sender_certificate: DelegationCertificate,
    /// z-base-32 Ed25519 signature over the message statement.
    signature: String,
}

impl MessageHash {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for MessageHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl FromStr for MessageHash {
    type Err = MessageHashParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        // One hash has one text form, so refuse upper case.
        if text.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(MessageHashParseError::Invalid);
        }
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(text, &mut bytes).map_err(|_| MessageHashParseError::Invalid)?;
        Ok(Self(bytes))
    }
}

impl From<MessageHash> for String {
    fn from(hash: MessageHash) -> Self {
        hash.to_string()
    }
}

impl TryFrom<String> for MessageHash {
    type Error = MessageHashParseError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl OpenedMessage {
    /// The caller identity of the delivered call: the original sender, never
    /// the mailbox.
    #[must_use]
    pub fn sender_master_did(&self) -> &str {
        &self.sender_certificate.master_did
    }

    /// The key to check against the master's revocation list.
    #[must_use]
    pub fn sender_instance_did(&self) -> &str {
        &self.sender_certificate.temporary_did
    }

    /// The signed `delivered` receipt for this message.
    pub fn sign_delivered(
        &self,
        instance: &Identity,
        certificate: &DelegationCertificate,
        at_secs: u64,
    ) -> Result<Receipt, ReceiptError> {
        receipt::sign(instance, certificate, self, SignedOutcome::Delivered, at_secs)
    }

    /// A signed rejection for one of the receiver's own checks after `open`
    /// (`Revoked`, `NotEligible`). A code that is not sender-caused is
    /// refused with `NotSignable`.
    pub fn sign_rejection(
        &self,
        instance: &Identity,
        certificate: &DelegationCertificate,
        code: RejectCode,
        at_secs: u64,
    ) -> Result<Receipt, ReceiptError> {
        receipt::sign(instance, certificate, self, SignedOutcome::Rejected(code), at_secs)
    }
}

impl RefusedMessage {
    #[must_use]
    pub fn sender_master_did(&self) -> &str {
        self.message.sender_master_did()
    }

    /// The ack for this message: signed when the code is sender-caused (only
    /// `FromFuture` can be), unsigned otherwise.
    pub fn receipt(
        &self,
        instance: &Identity,
        certificate: &DelegationCertificate,
        at_secs: u64,
    ) -> Result<Receipt, ReceiptError> {
        if self.code.is_sender_fault() {
            self.message.sign_rejection(instance, certificate, self.code, at_secs)
        } else {
            Ok(Receipt::unsigned_rejection(self.code, at_secs))
        }
    }
}

/// Lowercase hex of SHA-256 over the canonical JSON of the interface, the
/// method and the params. It binds the method as well as the params.
fn payload_hash(
    interface: &str,
    method: &str,
    params: &Value,
) -> Result<String, serde_json::Error> {
    let payload = json!({ "interface": interface, "method": method, "params": params });
    let bytes = serde_json::to_vec(&canonicalize_json_value(&payload))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

/// The statement the sender signs. `deposit_id` is in it so that the mailbox
/// cannot give the receiver an existing message under another deposit id.
/// `sender` (the member master DID) is in it so that the mailbox cannot swap
/// the certificate for one from another master: any master can issue a
/// valid certificate for any instance key, so the signature alone would not
/// say whose message this is.
fn message_statement(
    destination: &str,
    sender: &str,
    deposit_id: &DepositId,
    idempotency_key: &str,
    payload_hash: &str,
    sent_at_secs: u64,
) -> Value {
    json!({
        "domain": MESSAGE_DOMAIN,
        "destination": destination,
        "sender": sender,
        "deposit_id": deposit_id.to_string(),
        "idempotency_key": idempotency_key,
        "payload_hash": payload_hash,
        "sent_at_secs": sent_at_secs,
    })
}

/// SHA-256 over the same canonical bytes `Identity::sign_json` signs.
fn message_hash(statement: &Value) -> Result<MessageHash, serde_json::Error> {
    let bytes = serde_json::to_vec(&canonicalize_json_value(statement))?;
    Ok(MessageHash(Sha256::digest(bytes).into()))
}

/// Seals a call for a receiver. Each call makes a new deposit id and uses
/// `now_secs` as the send time; to deposit the same call again, call `seal`
/// again.
pub fn seal(
    content: &MessageContent,
    recipient: Recipient<'_>,
    sender: Sender<'_>,
    ttl_secs: u64,
    now_secs: u64,
) -> Result<Sealed, SealError> {
    if ttl_secs == 0 {
        return Err(SealError::ZeroTtl);
    }
    substrate::resolve_did_key(recipient.service_id)
        .map_err(|_| SealError::NotADidKey(recipient.service_id.to_string()))?;
    if substrate::derive_did_key(&sender.instance.public_key()) != sender.certificate.temporary_did
    {
        return Err(SealError::SignerMismatch);
    }
    // A message the receiver would reject for its certificate is never made.
    sender
        .certificate
        .verify_at(&sender.certificate.master_did, &[SCOPE_SERVICE_INSTANCE], now_secs)
        .map_err(|e| SealError::BadCertificate(e.to_string()))?;
    let deposit_id = DepositId::random().map_err(|e| SealError::Random(e.to_string()))?;
    seal_unchecked(content, recipient, sender, deposit_id, ttl_secs, now_secs)
}

/// Builds and seals without the sender-side checks, so tests can make
/// messages a real sender would refuse to make. Exported only with the
/// `test-support` feature, for tests in other crates.
pub fn seal_unchecked(
    content: &MessageContent,
    recipient: Recipient<'_>,
    sender: Sender<'_>,
    deposit_id: DepositId,
    ttl_secs: u64,
    sent_at_secs: u64,
) -> Result<Sealed, SealError> {
    let json_err = |e: serde_json::Error| SealError::Json(e.to_string());
    let p_hash =
        payload_hash(&content.interface, &content.method, &content.params).map_err(json_err)?;
    let statement = message_statement(
        recipient.service_id,
        &sender.certificate.master_did,
        &deposit_id,
        &content.idempotency_key,
        &p_hash,
        sent_at_secs,
    );
    let m_hash = message_hash(&statement).map_err(json_err)?;
    let signature =
        sender.instance.sign_json(&statement).map_err(|e| SealError::Json(e.to_string()))?;
    let inner = SealedContent {
        interface: content.interface.clone(),
        method: content.method.clone(),
        params: content.params.clone(),
        idempotency_key: content.idempotency_key.clone(),
        sent_at_secs,
        sender_certificate: sender.certificate.clone(),
        signature,
    };
    let plaintext = serde_json::to_vec(&inner).map_err(json_err)?;
    let size = plaintext.len() + crypto::SEAL_OVERHEAD;
    if size > MAX_SEALED_BYTES {
        return Err(SealError::TooLarge { bytes: size, max: MAX_SEALED_BYTES });
    }
    let sealed = crypto::seal_to(recipient.mailbox_key.as_bytes(), &plaintext)
        .map_err(|e| SealError::Crypto(e.to_string()))?;
    Ok(Sealed {
        deposit: Deposit {
            destination: recipient.service_id.to_string(),
            deposit_id,
            ttl_secs,
            sealed: SealedBytes::new(sealed),
        },
        sent: SentDeposit {
            deposit_id,
            receiver_service_id: recipient.service_id.to_string(),
            sender_master_did: sender.certificate.master_did.clone(),
            message_hash: m_hash,
            sent_at_secs,
        },
    })
}

/// Opens a sealed message and checks it.
///
/// The receiver passes its **own** service id and the deposit id the mailbox
/// gave at claim. It never reads either from inside the message, so a message
/// made for another service or another deposit fails `BadSignature`.
///
/// The checks run in two groups. Until the message signature matches the key
/// its certificate names, the sender is not known: a failure is `TooLarge`,
/// `Undecryptable`, `Malformed` or `BadSignature`. After that, the
/// certificate itself and the send time are checked, and a failure is
/// `Refused`, which carries the authentic message for the ack.
///
/// Revocation of the sender's instance key is not checked here: that needs a
/// network lookup. The caller checks `sender_instance_did` afterwards.
pub fn open(
    key: &MailboxSecretKey,
    own_service_id: &str,
    deposit_id: &DepositId,
    sealed: &SealedBytes,
    policy: OpenPolicy,
    now_secs: u64,
) -> Result<OpenedMessage, OpenError> {
    let inner = decrypt_inner(key, sealed)?;
    let message = check_signature(inner, own_service_id, deposit_id)?;
    check_sender(message, policy, now_secs)
}

fn decrypt_inner(key: &MailboxSecretKey, sealed: &SealedBytes) -> Result<SealedContent, OpenError> {
    if sealed.len() > MAX_SEALED_BYTES {
        return Err(OpenError::TooLarge { bytes: sealed.len(), max: MAX_SEALED_BYTES });
    }
    let plaintext = crypto::open_with(key.seed(), sealed.as_slice()).map_err(|e| match e {
        CryptoError::TooShort => OpenError::Malformed("too short".to_string()),
        _ => OpenError::Undecryptable,
    })?;
    serde_json::from_slice(&plaintext).map_err(|e| OpenError::Malformed(e.to_string()))
}

/// The signature is checked against the certificate's own `temporary_did`.
/// The result is "the key this certificate names signed this", not yet "the
/// master allowed it": that is `check_sender`.
fn check_signature(
    inner: SealedContent,
    own_service_id: &str,
    deposit_id: &DepositId,
) -> Result<OpenedMessage, OpenError> {
    let malformed = |e: serde_json::Error| OpenError::Malformed(e.to_string());
    let p_hash = payload_hash(&inner.interface, &inner.method, &inner.params).map_err(malformed)?;
    let statement = message_statement(
        own_service_id,
        &inner.sender_certificate.master_did,
        deposit_id,
        &inner.idempotency_key,
        &p_hash,
        inner.sent_at_secs,
    );
    verify_json_signature(&inner.sender_certificate.temporary_did, &statement, &inner.signature)
        .map_err(|_| OpenError::BadSignature)?;
    let message_hash = message_hash(&statement).map_err(malformed)?;
    Ok(OpenedMessage {
        content: MessageContent {
            interface: inner.interface,
            method: inner.method,
            params: inner.params,
            idempotency_key: inner.idempotency_key,
        },
        sender_certificate: inner.sender_certificate,
        sent_at_secs: inner.sent_at_secs,
        deposit_id: *deposit_id,
        message_hash,
    })
}

/// The certificate is checked at the send time, not now: a message that
/// waits hours at the mailbox would otherwise carry an expired certificate.
///
/// `TooOld` is refused but is not the sender's fault: a mailbox can hold a
/// message back and release it late. `BadCertificate` is not either: the
/// certificate may be forged, so the sender it names is not proven.
fn check_sender(
    message: OpenedMessage,
    policy: OpenPolicy,
    now_secs: u64,
) -> Result<OpenedMessage, OpenError> {
    let refuse = |message: OpenedMessage, code: RejectCode, detail: String| {
        Err(OpenError::Refused(Box::new(RefusedMessage { code, detail, message })))
    };
    let certificate = &message.sender_certificate;
    if let Err(e) = certificate.verify_at(
        &certificate.master_did,
        &[SCOPE_SERVICE_INSTANCE],
        message.sent_at_secs,
    ) {
        return refuse(message, RejectCode::BadCertificate, e.to_string());
    }
    let oldest =
        now_secs.saturating_sub(policy.max_ttl_secs.saturating_add(policy.max_clock_skew_secs));
    if message.sent_at_secs < oldest {
        let detail = format!("sent at {}, oldest accepted {oldest}", message.sent_at_secs);
        return refuse(message, RejectCode::TooOld, detail);
    }
    // Conversation has the same bound. Without it, a send time far in the
    // future would make the age check above useless.
    let latest = now_secs.saturating_add(policy.max_clock_skew_secs);
    if message.sent_at_secs > latest {
        let detail = format!("sent at {}, latest accepted {latest}", message.sent_at_secs);
        return refuse(message, RejectCode::FromFuture, detail);
    }
    Ok(message)
}

#[cfg(test)]
mod tests;
