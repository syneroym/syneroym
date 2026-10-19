//! Validation for service records kept by the mailbox.

use syneroym_core::{
    dht_registry::{EndpointType, SignedEndpointInfo},
    mailbox::MAX_RECORD_JSON_BYTES,
};

use crate::{caller::Instance, error::MailboxError};

/// A service record the mailbox may keep and republish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckedRecord {
    pub json: String,
    /// The pkarr timestamp, as `SignedEndpointInfo::verify` returns it.
    pub signed_at: u64,
    pub not_after: u64,
}

impl CheckedRecord {
    pub(crate) fn check(
        record: &SignedEndpointInfo,
        owner: &Instance,
        mailbox_did: &str,
        now: u64,
    ) -> Result<Self, MailboxError> {
        let info = &record.info;
        if info.service_id != owner.master_did {
            return Err(MailboxError::BadRecord("the record is for another service".to_string()));
        }
        if info.endpoint_type != EndpointType::Service {
            return Err(MailboxError::BadRecord("not a service record".to_string()));
        }
        if info.is_private {
            return Err(MailboxError::BadRecord(
                "a private record is never republished".to_string(),
            ));
        }
        match &info.mailbox {
            Some(m) if m.service_id == mailbox_did => {}
            _ => {
                return Err(MailboxError::BadRecord(
                    "the record does not name this mailbox".to_string(),
                ));
            }
        }
        if info.not_after <= now {
            return Err(MailboxError::BadRecord("the record has expired".to_string()));
        }
        let json = serde_json::to_string(record)
            .map_err(|e| MailboxError::BadRecord(format!("unserializable record: {e}")))?;
        if json.len() > MAX_RECORD_JSON_BYTES {
            return Err(MailboxError::BadRecord("the record is too large".to_string()));
        }
        let timestamp = record.verify().map_err(|e| MailboxError::BadRecord(e.to_string()))?;
        let signed_at = timestamp.as_u64();

        Ok(CheckedRecord { json, signed_at, not_after: info.not_after })
    }
}

#[cfg(test)]
mod tests;
