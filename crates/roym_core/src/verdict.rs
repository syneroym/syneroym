use std::fmt;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    listing,
    record::{self, VerifyOptions},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordVerdict<P> {
    pub verified: bool,
    /// Expiry is not a refusal here. A record past its own
    /// `expires_at_secs` still verifies -- the signature, the issuer and
    /// the delegation window are all still good -- and this says the
    /// window has passed. A caller that must decide whether to act
    /// refuses on this; a caller that must show what was offered
    /// does not. Always false for a record type that carries no expiry.
    pub expired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// The key that actually produced the signature. For a record signed
    /// under a person's delegation this is the service key the person
    /// certified -- the key a service-signed record from the same service
    /// carries as its issuer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer_did: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_at_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<P>,
}

impl<P> RecordVerdict<P> {
    pub fn refused(reason: impl Into<String>) -> Self {
        Self {
            verified: false,
            expired: false,
            reason: Some(reason.into()),
            revocation_status: None,
            record_id: None,
            issuer: None,
            signer_did: None,
            issued_at_secs: None,
            expires_at_secs: None,
            supersedes: None,
            payload: None,
        }
    }
}

/// A payload for a party-signed record tied to an agreement.
pub trait PartyRecordPayload: DeserializeOwned {
    type Error: fmt::Display;

    fn validate(&self) -> Result<(), Self::Error>;
    fn agreement(&self) -> &str;
    fn quote_author_did(&self) -> &str;
    fn counterparty_did(&self) -> &str;
}

/// Verifies a party-signed deal record against its envelope.
pub fn verify_party_record<P: PartyRecordPayload>(
    envelope: &str,
    now_secs: u64,
    record_type: &str,
    version: u32,
    noun: &str,
    issuer_noun: &str,
) -> RecordVerdict<P> {
    let opts = VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != record_type || verified.version != version {
        return RecordVerdict::refused(format!(
            "not a {record_type} record this build understands"
        ));
    }
    let payload: P = match serde_json::from_value(verified.payload.clone()) {
        Ok(p) => p,
        Err(e) => return RecordVerdict::refused(format!("payload: {e}")),
    };
    if let Err(e) = payload.validate() {
        return RecordVerdict::refused(e.to_string());
    }
    if verified.subject != payload.agreement() {
        return RecordVerdict::refused("envelope subject does not match agreement");
    }
    if verified.expires_at_secs.is_some() {
        return RecordVerdict::refused(format!("a {noun} may not declare an expiry"));
    }
    if verified.issuer != payload.quote_author_did()
        && verified.issuer != payload.counterparty_did()
    {
        return RecordVerdict::refused(format!(
            "{issuer_noun} issuer is not a party to the agreement"
        ));
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
