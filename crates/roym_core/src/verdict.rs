use serde::{Deserialize, Serialize};

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
