//! The body of a message that forwards a signed Roym record.
//!
//! The record travels as itself: this body carries the envelope and the
//! name of its type. The node stores the body and returns it. A client asks
//! the service that owns the record type to check the envelope before it
//! draws the record, so the check stays where it is today and the
//! conversation service links no record payload code.

use serde::{Deserialize, Serialize};

use crate::{card::MAX_CARD_BODY_BYTES, record};

/// The content type of a forwarded signed record.
pub const RECORD_FORWARD_CONTENT_TYPE: &str = "application/vnd.roym.record-forward+json";
/// The body version this build writes and reads.
pub const RECORD_FORWARD_VERSION: u32 = 1;
/// The record types a person can forward in this release.
pub const RECORD_FORWARD_TYPES: &[&str] =
    &[record::RECORD_LISTING, record::RECORD_PROFILE, record::RECORD_QUOTE];

/// The body of a forwarded signed record. A later version may add fields,
/// so a body from another node is read without refusing unknown ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordForwardBody {
    pub record_forward_version: u32,
    pub record_type: String,
    pub envelope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordForwardError {
    #[error("a record forward body is over the {MAX_CARD_BODY_BYTES}-byte maximum")]
    TooLarge,
    #[error("record forward body version {0} is not one this build reads")]
    UnsupportedVersion(u32),
    #[error("a '{0}' record cannot be forwarded")]
    UnsupportedRecordType(String),
    #[error("record forward body: {0}")]
    Malformed(String),
}

impl RecordForwardBody {
    /// Builds the body a sender's node stores. It refuses a type that is not
    /// in [`RECORD_FORWARD_TYPES`], an envelope over the size limit, and an
    /// envelope that is not a JSON object.
    pub fn new(record_type: &str, envelope: &str) -> Result<Self, RecordForwardError> {
        let body = Self {
            record_forward_version: RECORD_FORWARD_VERSION,
            record_type: record_type.to_string(),
            envelope: envelope.to_string(),
        };
        body.check(true)?;
        Ok(body)
    }

    /// Reads a body that another node wrote. It checks the version, the
    /// record type and the size before anything else reads the envelope.
    pub fn parse(body: &[u8]) -> Result<Self, RecordForwardError> {
        if body.len() > MAX_CARD_BODY_BYTES {
            return Err(RecordForwardError::TooLarge);
        }
        let parsed: Self = serde_json::from_slice(body)
            .map_err(|e| RecordForwardError::Malformed(e.to_string()))?;
        parsed.check(false)?;
        Ok(parsed)
    }

    /// The bytes this body travels as.
    pub fn to_bytes(&self) -> Result<Vec<u8>, RecordForwardError> {
        self.check(false)?;
        let bytes =
            serde_json::to_vec(self).map_err(|e| RecordForwardError::Malformed(e.to_string()))?;
        if bytes.len() > MAX_CARD_BODY_BYTES {
            return Err(RecordForwardError::TooLarge);
        }
        Ok(bytes)
    }

    /// `require_object` is true only on the send path: the receiver's check
    /// owns the verdict on the envelope, so a read here is structural only.
    fn check(&self, require_object: bool) -> Result<(), RecordForwardError> {
        if self.record_forward_version != RECORD_FORWARD_VERSION {
            return Err(RecordForwardError::UnsupportedVersion(self.record_forward_version));
        }
        if !RECORD_FORWARD_TYPES.contains(&self.record_type.as_str()) {
            return Err(RecordForwardError::UnsupportedRecordType(self.record_type.clone()));
        }
        if self.envelope.len() > MAX_CARD_BODY_BYTES {
            return Err(RecordForwardError::TooLarge);
        }
        if require_object
            && serde_json::from_str::<serde_json::Value>(&self.envelope)
                .map(|v| !v.is_object())
                .unwrap_or(true)
        {
            return Err(RecordForwardError::Malformed(
                "the record envelope is not a JSON object".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
