//! Where to reach a person. The conversation interface addresses a
//! *service*, never a person, and person-to-substrate resolution needs a
//! Primary Substrate designation that is unbuilt. So the mapping is this
//! product's own, and this is its one definition: Profile & Contacts
//! stores it, a `profile` record signs it, and a listing embeds it.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::lang::{LangTag, TranslationError, check_text, check_translation_langs};

pub const MAX_DISPLAY_NAME_LEN: usize = 128;
pub const MAX_ABOUT_LEN: usize = 1024;
pub const MAX_ADDRESS_LEN: usize = 256;
pub const MAX_LANGUAGES: usize = 8;

/// An author translation of a profile's free-text fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileTranslation {
    pub lang: LangTag,
    pub about: String,
}

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
    pub lang: LangTag,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<LangTag>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub translations: Vec<ProfileTranslation>,
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
    #[error("more than {MAX_LANGUAGES} languages")]
    TooManyLanguages,
    #[error("duplicate language '{0}'")]
    DuplicateLanguage(String),
    #[error("profile has translations but no about text")]
    TranslationWithoutAbout,
    #[error(transparent)]
    Translation(#[from] TranslationError),
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

        if self.languages.len() > MAX_LANGUAGES {
            return Err(PersonError::TooManyLanguages);
        }
        let mut seen_langs = HashSet::with_capacity(self.languages.len());
        for lang in &self.languages {
            if !seen_langs.insert(lang) {
                return Err(PersonError::DuplicateLanguage(lang.to_string()));
            }
        }

        if !self.translations.is_empty() {
            if self.about.as_deref().unwrap_or("").trim().is_empty() {
                return Err(PersonError::TranslationWithoutAbout);
            }
            check_translation_langs(&self.lang, self.translations.iter().map(|t| &t.lang))?;
            for t in &self.translations {
                check_text(&t.lang, "about", &t.about, MAX_ABOUT_LEN, true)?;
            }
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

    fn sample_tag(s: &str) -> LangTag {
        LangTag::parse(s).expect("valid tag")
    }

    #[test]
    fn profile_payload_validation() {
        let valid = ProfilePayload {
            display_name: "Alice".to_string(),
            about: Some("Hello".to_string()),
            conversation_address: "svc-123".to_string(),
            lang: sample_tag("en"),
            languages: vec![sample_tag("en"), sample_tag("hi")],
            translations: vec![],
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
    fn profile_languages_validation() {
        let mut p = ProfilePayload {
            display_name: "Alice".to_string(),
            about: Some("Hello".to_string()),
            conversation_address: "svc-123".to_string(),
            lang: sample_tag("en"),
            languages: vec![],
            translations: vec![],
        };
        assert!(p.validate().is_ok());

        // 9 languages is too many
        let tags = ["en", "hi", "bn", "gu", "kn", "ml", "mr", "or", "pa"];
        p.languages = tags.into_iter().map(sample_tag).collect();
        assert_eq!(p.validate(), Err(PersonError::TooManyLanguages));

        // Duplicate language
        p.languages = vec![sample_tag("hi"), sample_tag("hi")];
        assert_eq!(p.validate(), Err(PersonError::DuplicateLanguage("hi".to_string())));
    }

    #[test]
    fn profile_translations_validation() {
        let mut p = ProfilePayload {
            display_name: "Alice".to_string(),
            about: None,
            conversation_address: "svc-123".to_string(),
            lang: sample_tag("en"),
            languages: vec![],
            translations: vec![ProfileTranslation {
                lang: sample_tag("hi"),
                about: "नमस्ते".to_string(),
            }],
        };
        assert_eq!(p.validate(), Err(PersonError::TranslationWithoutAbout));

        p.about = Some("   ".to_string());
        assert_eq!(p.validate(), Err(PersonError::TranslationWithoutAbout));

        p.about = Some("English about".to_string());
        assert!(p.validate().is_ok());

        // Translation with empty text
        p.translations[0].about = "".to_string();
        assert_eq!(
            p.validate(),
            Err(PersonError::Translation(TranslationError::Empty {
                lang: "hi".to_string(),
                field: "about",
            }))
        );
    }

    #[test]
    fn payload_with_locale_and_no_lang_fails_deserialization() {
        let json_str = r#"{
            "display_name": "Alice",
            "conversation_address": "svc-123",
            "locale": "en-US"
        }"#;
        assert!(serde_json::from_str::<ProfilePayload>(json_str).is_err());
    }

    #[test]
    fn is_did_key_checks() {
        assert!(is_did_key("did:key:z6M123"));
        assert!(!is_did_key("did:key:"));
        assert!(!is_did_key("did:web:example.com"));
        assert!(!is_did_key("z6M123"));
    }
}
