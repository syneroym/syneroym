//! Where to reach a person. The conversation interface addresses a
//! *service*, never a person, and person-to-substrate resolution needs a
//! Primary Substrate designation that is unbuilt. So the mapping is this
//! product's own, and this is its one definition: Profile & Contacts
//! stores it, a `profile` record signs it, and a listing embeds it.

use serde::{Deserialize, Serialize};

pub const MAX_DISPLAY_NAME_LEN: usize = 128;
pub const MAX_ABOUT_LEN: usize = 1024;
pub const MAX_ADDRESS_LEN: usize = 256;

/// The `profile` record's payload at version 1. Every field is a value to
/// display, never markup: the Hub inserts these as text nodes only, the
/// same rule that governs every value a card carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfilePayload {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// The routing service id `open-direct` takes -- this person's own
    /// Conversation service. Signed as part of the profile, so a stranger
    /// who verifies the profile has an address they can attribute.
    pub conversation_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PersonError {
    #[error("display name is empty or over {MAX_DISPLAY_NAME_LEN} bytes")]
    DisplayName,
    #[error("about is over {MAX_ABOUT_LEN} bytes")]
    About,
    #[error("conversation address is empty or over {MAX_ADDRESS_LEN} bytes")]
    Address,
    #[error("'{0}' is not a did:key")]
    NotADid(String),
}

impl ProfilePayload {
    pub fn validate(&self) -> Result<(), PersonError> {
        if self.display_name.trim().is_empty() || self.display_name.len() > MAX_DISPLAY_NAME_LEN {
            return Err(PersonError::DisplayName);
        }
        if let Some(ref about) = self.about
            && about.len() > MAX_ABOUT_LEN
        {
            return Err(PersonError::About);
        }
        if self.conversation_address.trim().is_empty()
            || self.conversation_address.len() > MAX_ADDRESS_LEN
        {
            return Err(PersonError::Address);
        }
        Ok(())
    }
}

/// `did:key:` plus a non-empty remainder. Deliberately not a multicodec
/// parse: this crate has no crypto, and a DID that passes here but fails
/// key resolution fails loudly at the host boundary, which is where the
/// real check belongs.
pub fn is_did_key(s: &str) -> bool {
    s.strip_prefix("did:key:").is_some_and(|r| !r.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_payload_validation() {
        let valid = ProfilePayload {
            display_name: "Alice".to_string(),
            about: Some("Hello".to_string()),
            conversation_address: "svc-123".to_string(),
            locale: Some("en-US".to_string()),
        };
        assert!(valid.validate().is_ok());

        let mut invalid_name = valid.clone();
        invalid_name.display_name = "".to_string();
        assert_eq!(invalid_name.validate(), Err(PersonError::DisplayName));

        let mut invalid_about = valid.clone();
        invalid_about.about = Some("a".repeat(MAX_ABOUT_LEN + 1));
        assert_eq!(invalid_about.validate(), Err(PersonError::About));

        let mut invalid_addr = valid.clone();
        invalid_addr.conversation_address = "".to_string();
        assert_eq!(invalid_addr.validate(), Err(PersonError::Address));
    }

    #[test]
    fn is_did_key_checks() {
        assert!(is_did_key("did:key:z6M123"));
        assert!(!is_did_key("did:key:"));
        assert!(!is_did_key("did:web:example.com"));
        assert!(!is_did_key("z6M123"));
    }
}
