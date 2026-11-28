//! The chat body and the signed origin of a Roym text message.
//!
//! A message sent with a forward mode carries a JSON body instead of plain
//! text. A `signed` message also carries a `message-origin` record: the
//! author's own statement of what text they wrote, at what time. Any node
//! can check that record without any key material, so a forward can name
//! its first author. A `not-forwardable` message carries no origin and is
//! never named as someone else's words.

use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Value, json};
use sha2::Digest;
use syneroym_app_host::types::signing::RecordDraft;

use crate::{
    backup::bytes_to_hex,
    listing,
    record::{self, Envelope, VerifyOptions},
    verdict::RecordVerdict,
};

/// The content type of every message that names a forward mode.
pub const ROYM_MESSAGE_CONTENT_TYPE: &str = "application/vnd.roym.message+json";
pub const CHAT_BODY_VERSION: u32 = 1;
/// The only inner content type in this version. A later version can add
/// another one without making this build read it wrongly.
pub const INNER_TEXT_CONTENT_TYPE: &str = "text/plain";
/// An origin envelope longer than this is not read, before any parse.
pub const MAX_ORIGIN_BYTES: usize = 8 * 1024;
/// "Forwarded many times" from this count on. The view carries the flag,
/// so every client shows the same cut-off.
pub const MANY_FORWARDS: u32 = 5;
/// A larger count in a body is read as this. The count is only a hint, so
/// clamping loses nothing and keeps a changed client from breaking the body.
pub const MAX_FORWARD_COUNT: u32 = 1_000;
/// The version of the origin payload this build writes and reads.
pub const MESSAGE_ORIGIN_VERSION: u32 = 1;

/// The sender's choice for one message. A local method input with another
/// string is refused, so a typo cannot silently become `not-forwardable`.
/// Only a body that another node wrote is read leniently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ForwardMode {
    Signed,
    NotForwardable,
}

impl ForwardMode {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Signed => "signed",
            Self::NotForwardable => "not-forwardable",
        }
    }
}

/// The body of a message that names a forward mode. Unknown fields are
/// ignored: another node wrote it, and a later version may add fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatBody {
    pub chat_version: u32,
    /// The content type of [`Self::text`], never the chat body type itself.
    pub content_type: String,
    pub text: String,
    #[serde(deserialize_with = "lenient_mode")]
    pub forward_mode: ForwardMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, deserialize_with = "clamped_count", skip_serializing_if = "Option::is_none")]
    pub forward_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChatBodyError {
    #[error("chat body is not valid JSON: {0}")]
    Malformed(String),
    #[error("chat body is not one this build reads: {0}")]
    Unsupported(String),
}

/// A later mode never makes a message forwardable on this build.
fn lenient_mode<'de, D>(deserializer: D) -> Result<ForwardMode, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    Ok(if raw == ForwardMode::Signed.as_str() {
        ForwardMode::Signed
    } else {
        ForwardMode::NotForwardable
    })
}

/// Reads any JSON number, then clamps it. A plain integer field refuses a
/// number above `u64::MAX` and a negative number, which would make the
/// whole body unreadable; the count is a hint, so a clamped read is right.
fn clamped_count<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let count = match value {
        Value::Null => None,
        Value::Number(n) => read_count(n),
        _ => return Err(de::Error::custom("forward count is not a number")),
    };
    Ok(count)
}

fn read_count(n: serde_json::Number) -> Option<u32> {
    if let Some(v) = n.as_u64() {
        return match v {
            0 => None,
            _ => Some(u32::try_from(v).unwrap_or(MAX_FORWARD_COUNT).min(MAX_FORWARD_COUNT)),
        };
    }
    if let Some(v) = n.as_i64() {
        return match v {
            ..=0 => None,
            _ => Some(u32::try_from(v).unwrap_or(MAX_FORWARD_COUNT).min(MAX_FORWARD_COUNT)),
        };
    }
    // Any number too big for an integer, or a float.
    let v = n.as_f64()?;
    if !v.is_finite() || v < 1.0 {
        return None;
    }
    if v.fract() != 0.0 {
        return None;
    }
    Some((v as u64).min(u64::from(MAX_FORWARD_COUNT)) as u32)
}

impl ChatBody {
    pub fn parse(bytes: &[u8]) -> Result<ChatBody, ChatBodyError> {
        let body: ChatBody =
            serde_json::from_slice(bytes).map_err(|e| ChatBodyError::Malformed(e.to_string()))?;
        if body.chat_version != CHAT_BODY_VERSION {
            return Err(ChatBodyError::Unsupported(format!(
                "chat body version {}",
                body.chat_version
            )));
        }
        if body.content_type != INNER_TEXT_CONTENT_TYPE {
            return Err(ChatBodyError::Unsupported(format!(
                "inner content type {}",
                body.content_type
            )));
        }
        Ok(body)
    }

    /// Serialization of these plain fields cannot fail. The object is built
    /// here rather than through a fallible `to_vec` so a failure can never
    /// turn into an empty message.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut obj = json!({
            "chat_version": self.chat_version,
            "content_type": self.content_type,
            "text": self.text,
            "forward_mode": self.forward_mode,
        });
        if let Some(origin) = &self.origin {
            obj["origin"] = json!(origin);
        }
        if let Some(count) = self.forward_count {
            obj["forward_count"] = json!(count);
        }
        obj.to_string().into_bytes()
    }
}

/// The payload of a `message-origin` record. The author and the send time
/// are the envelope's `issuer` and `issued_at_secs`, so they are not
/// repeated here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageOriginPayload {
    pub content_type: String,
    /// The message's tags as stored, 0 to 3, in the same order.
    pub languages: Vec<String>,
    /// See [`content_hash`].
    pub content_hash: String,
}

/// What an origin check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginCheck {
    Checked { author_did: String, written_at_secs: u64, record_id: String },
    Failed { reason: String },
    Absent,
}

/// A draft that names the author's text. The record has no subject and no
/// expiry: it is a statement about one message, not a credential.
#[must_use]
pub fn origin_draft(payload: &MessageOriginPayload) -> RecordDraft {
    RecordDraft {
        version: MESSAGE_ORIGIN_VERSION,
        record_type: record::RECORD_MESSAGE_ORIGIN.to_string(),
        subject: String::new(),
        payload: json!({
            "content_type": payload.content_type,
            "languages": payload.languages,
            "content_hash": payload.content_hash,
        })
        .to_string(),
        expires_at_secs: None,
        supersedes: None,
    }
}

/// Verifies the origin envelope and checks its shape as an origin. This is
/// a form check: a checked origin proves that the holder of the issuer DID
/// wrote this text. It does not prove that the holder is someone the
/// reader knows; the reader's own contacts decide the name shown.
///
/// No revocation list is passed, as for every other Roym record, so the
/// verdict says "unknown".
#[must_use]
pub fn verify_message_origin(envelope: &str, now_secs: u64) -> RecordVerdict<MessageOriginPayload> {
    let parsed = match Envelope::from_json(envelope) {
        Ok(e) => e,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    let verified = match record::verify_json(envelope, &VerifyOptions::new(now_secs)) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != record::RECORD_MESSAGE_ORIGIN
        || verified.version != MESSAGE_ORIGIN_VERSION
    {
        return RecordVerdict::refused("not a message-origin record this build understands");
    }
    let payload: MessageOriginPayload = match serde_json::from_value(verified.payload.clone()) {
        Ok(p) => p,
        Err(e) => return RecordVerdict::refused(format!("payload: {e}")),
    };
    if !verified.subject.is_empty() {
        return RecordVerdict::refused("a message-origin may not declare a subject");
    }
    if verified.expires_at_secs.is_some() {
        return RecordVerdict::refused("a message-origin may not declare an expiry");
    }
    if parsed.delegation.is_none() {
        return RecordVerdict::refused("a message-origin must be signed as a person");
    }
    RecordVerdict {
        verified: true,
        expired: false,
        reason: None,
        revocation_status: Some(listing::revocation_status_word(verified.revocation_status)),
        record_id: Some(verified.record_id),
        issuer: Some(verified.issuer),
        signer_did: Some(verified.signer_did),
        issued_at_secs: Some(verified.issued_at_secs),
        expires_at_secs: None,
        supersedes: verified.supersedes,
        payload: Some(payload),
    }
}

/// Checks a body's origin against the message it travels in.
#[must_use]
pub fn check_origin(body: &ChatBody, languages: &[String], now_secs: u64) -> OriginCheck {
    if body.forward_mode != ForwardMode::Signed {
        return OriginCheck::Absent;
    }
    let Some(origin) = body.origin.as_deref() else {
        return OriginCheck::Failed { reason: "no signed origin".to_string() };
    };
    if origin.len() > MAX_ORIGIN_BYTES {
        return OriginCheck::Failed { reason: "origin too large".to_string() };
    }
    let verdict = verify_message_origin(origin, now_secs);
    if !verdict.verified {
        return OriginCheck::Failed {
            reason: verdict.reason.unwrap_or_else(|| "origin could not be checked".to_string()),
        };
    }
    let Some(payload) = verdict.payload else {
        return OriginCheck::Failed { reason: "origin has no payload".to_string() };
    };
    if payload.content_type != body.content_type {
        return OriginCheck::Failed { reason: "origin is for another content type".to_string() };
    }
    if payload.languages != languages {
        return OriginCheck::Failed { reason: "origin is for other languages".to_string() };
    }
    let recomputed = content_hash(&body.content_type, languages, body.text.as_bytes(), &[]);
    if payload.content_hash != recomputed {
        return OriginCheck::Failed { reason: "content hash does not match".to_string() };
    }
    let Some(author_did) = verdict.issuer else {
        return OriginCheck::Failed { reason: "origin names no author".to_string() };
    };
    let Some(written_at_secs) = verdict.issued_at_secs else {
        return OriginCheck::Failed { reason: "origin has no send time".to_string() };
    };
    let Some(record_id) = verdict.record_id else {
        return OriginCheck::Failed { reason: "origin has no record id".to_string() };
    };
    OriginCheck::Checked { author_did, written_at_secs, record_id }
}

/// The hash an origin names. Attachments are not built yet, so the blob
/// list is always empty; the list is in the format so a later version can
/// add one without changing the meaning of a hash.
#[must_use]
pub fn content_hash(
    content_type: &str,
    languages: &[String],
    body: &[u8],
    blob_hashes: &[[u8; 32]],
) -> String {
    let mut input = Vec::new();
    input.extend_from_slice(b"roym-message-origin-v1");
    push_len(&mut input, content_type.len());
    input.extend_from_slice(content_type.as_bytes());
    push_len(&mut input, languages.len());
    for tag in languages {
        push_len(&mut input, tag.len());
        input.extend_from_slice(tag.as_bytes());
    }
    push_len(&mut input, body.len());
    input.extend_from_slice(body);
    push_len(&mut input, blob_hashes.len());
    for hash in blob_hashes {
        input.extend_from_slice(hash);
    }
    format!("sha256:{}", bytes_to_hex(&sha2::Sha256::digest(&input)))
}

/// A length prefix keeps two different splits of the same bytes apart.
/// The host refuses a message body far below 4 GiB, so the cast cannot
/// lose a real length.
fn push_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_be_bytes());
}

/// What the reading node found about a chat body. The Hub offers "Forward"
/// exactly when `forwardable` is true, and draws the forwarded line only
/// when `forwarded` is true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardView {
    pub mode: ForwardMode,
    pub forwardable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginView>,
    /// Why a `signed` body's origin did not check, or why the body could
    /// not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_error: Option<String>,
    /// 0 for a message that is not a forward.
    pub forward_count: u32,
    pub many_forwards: bool,
    pub forwarded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginView {
    pub author_did: String,
    pub written_at_secs: u64,
    pub record_id: String,
    /// The author is the reader, so the Hub says "you".
    pub is_you: bool,
}

/// Reads one chat body for display. `sender_person_did` is the person who
/// sent the message to this node, when the receiver's contacts name one.
#[must_use]
pub fn forward_view(
    body: &ChatBody,
    languages: &[String],
    sender_person_did: Option<&str>,
    reader_did: Option<&str>,
    now_secs: u64,
) -> ForwardView {
    let count = body.forward_count.unwrap_or(0);
    if body.forward_mode != ForwardMode::Signed {
        return ForwardView {
            mode: ForwardMode::NotForwardable,
            forwardable: false,
            origin: None,
            origin_error: None,
            forward_count: 0,
            many_forwards: false,
            forwarded: false,
        };
    }
    match check_origin(body, languages, now_secs) {
        OriginCheck::Checked { author_did, written_at_secs, record_id } => {
            // A sender who is known and is not the author sent a forward,
            // even if a changed client dropped the count. An unknown
            // sender is the known gap: the origin still names the author.
            let from_someone_else = sender_person_did.is_some_and(|did| did != author_did);
            ForwardView {
                mode: ForwardMode::Signed,
                forwardable: true,
                origin: Some(OriginView {
                    is_you: reader_did == Some(author_did.as_str()),
                    author_did,
                    written_at_secs,
                    record_id,
                }),
                origin_error: None,
                forward_count: count,
                many_forwards: count >= MANY_FORWARDS,
                forwarded: count >= 1 || from_someone_else,
            }
        }
        OriginCheck::Failed { reason } => ForwardView {
            mode: ForwardMode::Signed,
            forwardable: false,
            origin: None,
            origin_error: Some(reason),
            forward_count: count,
            many_forwards: false,
            forwarded: false,
        },
        OriginCheck::Absent => ForwardView {
            mode: ForwardMode::Signed,
            forwardable: false,
            origin: None,
            origin_error: Some("no signed origin".to_string()),
            forward_count: count,
            many_forwards: false,
            forwarded: false,
        },
    }
}

/// The view of a chat body that could not be read. The Hub draws its "no
/// body" text instead of the raw JSON a changed client made.
#[must_use]
pub fn unreadable_view(reason: &str) -> ForwardView {
    ForwardView {
        mode: ForwardMode::NotForwardable,
        forwardable: false,
        origin: None,
        origin_error: Some(reason.to_string()),
        forward_count: 0,
        many_forwards: false,
        forwarded: false,
    }
}

/// The count a forward carries. A person's own forward already counts
/// them, so forwarding it again keeps the count; otherwise every hop adds
/// one, up to the clamp.
#[must_use]
pub fn next_forward_count(old: u32, source_is_own_forward: bool) -> u32 {
    if source_is_own_forward && old >= 1 {
        return old.min(MAX_FORWARD_COUNT);
    }
    old.saturating_add(1).min(MAX_FORWARD_COUNT)
}

#[cfg(test)]
mod tests;
