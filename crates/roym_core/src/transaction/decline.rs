//! Signed deal-decline records.
//!
//! A deal-decline record is signed by the party that declines:
//! the counterparty a quote version or its own enquiry (a withdrawal),
//! or the quote author an enquiry.

use serde::{Deserialize, Serialize};
use syneroym_signed_record::VerifyOptions;
use thiserror::Error;

use crate::{
    deal::LocalText,
    lang::TranslationError,
    listing,
    record::{self, RECORD_DEAL_DECLINE},
    verdict::RecordVerdict,
};

pub const DEAL_DECLINE_VERSION: u32 = 1;
pub const MAX_DECLINE_REASON_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DeclineError {
    #[error("declines record id '{0}' must start with 'rec_'")]
    InvalidDeclinesRecordId(String),
    #[error("deal id is empty")]
    DealEmpty,
    #[error("conversation id is empty")]
    ConversationEmpty,
    #[error("reason: {0}")]
    Reason(#[from] TranslationError),
}

/// The signed payload of a `deal-decline` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DealDeclinePayload {
    /// The record_id of the enquiry or quote version declined.
    pub declines: String,
    /// The quote_id, or the enquiry_id when no quote exists.
    pub deal: String,
    pub conversation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<LocalText>,
}

impl DealDeclinePayload {
    pub fn validate(&self) -> Result<(), DeclineError> {
        if !self.declines.starts_with("rec_") {
            return Err(DeclineError::InvalidDeclinesRecordId(self.declines.clone()));
        }
        if self.deal.trim().is_empty() {
            return Err(DeclineError::DealEmpty);
        }
        if self.conversation.trim().is_empty() {
            return Err(DeclineError::ConversationEmpty);
        }
        if let Some(ref reason) = self.reason {
            reason.validate("reason", MAX_DECLINE_REASON_LEN)?;
        }
        Ok(())
    }
}

/// Verifies a signed `deal-decline` record envelope.
pub fn verify_deal_decline(envelope: &str, now_secs: u64) -> RecordVerdict<DealDeclinePayload> {
    let opts = VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != RECORD_DEAL_DECLINE || verified.version != DEAL_DECLINE_VERSION {
        return RecordVerdict::refused("not a deal-decline record this build understands");
    }
    let payload: DealDeclinePayload = match serde_json::from_value(verified.payload.clone()) {
        Ok(p) => p,
        Err(e) => return RecordVerdict::refused(format!("payload: {e}")),
    };
    if let Err(e) = payload.validate() {
        return RecordVerdict::refused(e.to_string());
    }
    if verified.subject != payload.declines {
        return RecordVerdict::refused("envelope subject does not match declines");
    }
    if verified.expires_at_secs.is_some() {
        return RecordVerdict::refused("a deal decline may not declare an expiry");
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

#[cfg(test)]
mod tests {
    use syneroym_identity::{Identity, substrate};
    use syneroym_signed_record::{Envelope, RecordDraft};

    use super::*;
    use crate::lang::LangTag;

    fn sign_decline_envelope(
        key: &Identity,
        subject: &str,
        payload: &DealDeclinePayload,
        expires_at_secs: Option<u64>,
        now_secs: u64,
    ) -> String {
        let issuer = substrate::derive_did_key(&key.public_key());
        let draft = RecordDraft {
            version: DEAL_DECLINE_VERSION,
            record_type: RECORD_DEAL_DECLINE.to_string(),
            subject: subject.to_string(),
            payload: serde_json::to_value(payload).unwrap(),
            expires_at_secs,
            supersedes: None,
        };
        let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, now_secs).unwrap();
        let sig = z32::encode(&key.sign(&bytes).to_bytes());
        env.attach_signature(sig).unwrap();
        env.to_json().unwrap()
    }

    #[test]
    fn valid_decline_payload_validates() {
        let payload = DealDeclinePayload {
            declines: "rec_123".to_string(),
            deal: "quo_123".to_string(),
            conversation: "conv_123".to_string(),
            reason: Some(LocalText::plain(
                LangTag::parse_canonical("en").expect("valid lang"),
                "Budget too low",
            )),
        };
        assert!(payload.validate().is_ok());
    }

    #[test]
    fn invalid_declines_id_rejected() {
        let payload = DealDeclinePayload {
            declines: "bad_id".to_string(),
            deal: "quo_123".to_string(),
            conversation: "conv_123".to_string(),
            reason: None,
        };
        assert!(payload.validate().is_err());
    }

    #[test]
    fn verify_deal_decline_happy_path() {
        let key = Identity::generate().unwrap();
        let payload = DealDeclinePayload {
            declines: "rec_quote_1".to_string(),
            deal: "quo_deal_1".to_string(),
            conversation: "conv_1".to_string(),
            reason: None,
        };
        let env_json = sign_decline_envelope(&key, "rec_quote_1", &payload, None, 1000);
        let verdict = verify_deal_decline(&env_json, 1000);
        assert!(verdict.verified);
        assert_eq!(verdict.payload.unwrap().deal, "quo_deal_1");
    }

    #[test]
    fn verify_deal_decline_subject_mismatch_refused() {
        let key = Identity::generate().unwrap();
        let payload = DealDeclinePayload {
            declines: "rec_quote_1".to_string(),
            deal: "quo_deal_1".to_string(),
            conversation: "conv_1".to_string(),
            reason: None,
        };
        let env_json = sign_decline_envelope(&key, "rec_other", &payload, None, 1000);
        let verdict = verify_deal_decline(&env_json, 1000);
        assert!(!verdict.verified);
        assert!(verdict.reason.unwrap().contains("subject does not match"));
    }

    #[test]
    fn verify_deal_decline_with_expiry_refused() {
        let key = Identity::generate().unwrap();
        let payload = DealDeclinePayload {
            declines: "rec_quote_1".to_string(),
            deal: "quo_deal_1".to_string(),
            conversation: "conv_1".to_string(),
            reason: None,
        };
        let env_json = sign_decline_envelope(&key, "rec_quote_1", &payload, Some(2000), 1000);
        let verdict = verify_deal_decline(&env_json, 1000);
        assert!(!verdict.verified);
        assert!(verdict.reason.unwrap().contains("may not declare an expiry"));
    }
}
