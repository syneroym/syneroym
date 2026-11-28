//! Where to reach a person. The conversation interface addresses a
//! *service*, never a person, and person-to-substrate resolution needs a
//! Primary Substrate designation that is unbuilt. So the mapping is this
//! product's own, and this is its one definition: Profile & Contacts
//! stores it, a `profile` record signs it, and a listing embeds it.

use serde::{Deserialize, Serialize};
use syneroym_text_lang::{self, TagListError};

use crate::{
    lang::{LangTag, TranslationError, check_text, check_translation_langs},
    listing::revocation_status_word,
    record::{self, RECORD_PROFILE, VerifyOptions},
    verdict::RecordVerdict,
};

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

        syneroym_text_lang::check_tag_list(&self.languages, MAX_LANGUAGES).map_err(
            |e| match e {
                TagListError::TooMany(_) => PersonError::TooManyLanguages,
                TagListError::Duplicate(lang) => PersonError::DuplicateLanguage(lang),
            },
        )?;

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

/// Checks a signed `profile` envelope: its signature, its record shape, the
/// subject and the payload. The caller stores the record; this function only
/// checks it.
///
/// `expected_did` is the person the caller already has in hand (adding a
/// contact). The record must then be issued and written by that person, and
/// its subject must be that person. With `None`, the record must be written
/// by its own subject, which is how a stranger's profile is checked.
#[must_use]
pub fn verify_profile_envelope(
    envelope: &str,
    expected_did: Option<&str>,
    now_secs: u64,
) -> RecordVerdict<ProfilePayload> {
    let opts = match expected_did {
        Some(did) => VerifyOptions::new(now_secs).expecting(did),
        None => VerifyOptions::new(now_secs),
    };
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(format!("profile did not verify: {e}")),
    };
    if verified.record_type != RECORD_PROFILE || verified.version != 1 {
        return RecordVerdict::refused("not a profile record this build understands");
    }
    let subject_matches = match expected_did {
        Some(did) => verified.subject == did,
        None => verified.subject == verified.issuer,
    };
    if !subject_matches {
        let reason = match expected_did {
            Some(did) => {
                format!("profile subject '{}' does not match contact DID '{did}'", verified.subject)
            }
            None => format!(
                "profile subject '{}' does not match its issuer '{}'",
                verified.subject, verified.issuer
            ),
        };
        return RecordVerdict::refused(reason);
    }
    let payload: ProfilePayload = match serde_json::from_value(verified.payload.clone()) {
        Ok(p) => p,
        Err(e) => return RecordVerdict::refused(format!("profile payload: {e}")),
    };
    if let Err(e) = payload.validate() {
        return RecordVerdict::refused(e.to_string());
    }
    RecordVerdict {
        verified: true,
        expired: false,
        reason: None,
        revocation_status: Some(revocation_status_word(verified.revocation_status)),
        record_id: Some(verified.record_id),
        issuer: Some(verified.issuer),
        signer_did: Some(verified.signer_did),
        issued_at_secs: Some(verified.issued_at_secs),
        expires_at_secs: verified.expires_at_secs,
        supersedes: verified.supersedes,
        payload: Some(payload),
    }
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
    fn profile_payload_deserialization_requires_canonical_lang() {
        let base = r#"{
            "display_name": "Alice",
            "conversation_address": "svc-123",
            "lang": "en"
        }"#;
        assert!(serde_json::from_str::<ProfilePayload>(base).is_ok());

        // Missing lang
        let missing = r#"{
            "display_name": "Alice",
            "conversation_address": "svc-123"
        }"#;
        assert!(serde_json::from_str::<ProfilePayload>(missing).is_err());

        // Malformed lang
        let malformed = r#"{
            "display_name": "Alice",
            "conversation_address": "svc-123",
            "lang": "hi_IN"
        }"#;
        assert!(serde_json::from_str::<ProfilePayload>(malformed).is_err());

        // Non-canonical lang
        let non_canonical = r#"{
            "display_name": "Alice",
            "conversation_address": "svc-123",
            "lang": "HI"
        }"#;
        assert!(serde_json::from_str::<ProfilePayload>(non_canonical).is_err());
    }

    #[test]
    fn is_did_key_checks() {
        assert!(is_did_key("did:key:z6M123"));
        assert!(!is_did_key("did:key:"));
        assert!(!is_did_key("did:web:example.com"));
        assert!(!is_did_key("z6M123"));
    }

    mod envelope {
        use syneroym_identity::{Identity, substrate};
        use syneroym_signed_record::{Envelope, RecordDraft};

        use super::*;

        fn sample_payload() -> ProfilePayload {
            ProfilePayload {
                display_name: "Alice".to_string(),
                about: Some("Hello".to_string()),
                conversation_address: "svc-123".to_string(),
                lang: sample_tag("en"),
                languages: vec![],
                translations: vec![],
            }
        }

        fn signed_profile(key: &Identity, subject: &str, payload: &ProfilePayload) -> String {
            let issuer = substrate::derive_did_key(&key.public_key());
            let draft = RecordDraft {
                version: 1,
                record_type: RECORD_PROFILE.to_string(),
                subject: subject.to_string(),
                payload: serde_json::to_value(payload).unwrap(),
                expires_at_secs: None,
                supersedes: None,
            };
            let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, 1_000).unwrap();
            let sig = z32::encode(&key.sign(&bytes).to_bytes());
            env.attach_signature(sig).unwrap();
            env.to_json().unwrap()
        }

        #[test]
        fn a_profile_signed_by_its_subject_verifies() {
            let key = Identity::generate().unwrap();
            let did = substrate::derive_did_key(&key.public_key());
            let envelope = signed_profile(&key, &did, &sample_payload());

            let verdict = verify_profile_envelope(&envelope, None, 1_000);
            assert!(verdict.verified, "{:?}", verdict.reason);
            assert_eq!(verdict.payload.unwrap().display_name, "Alice");
            assert_eq!(verdict.issuer.as_deref(), Some(did.as_str()));
            assert!(verdict.record_id.is_some());
        }

        #[test]
        fn a_profile_for_another_subject_does_not_verify() {
            let key = Identity::generate().unwrap();
            let other = Identity::generate().unwrap();
            let other_did = substrate::derive_did_key(&other.public_key());
            let envelope = signed_profile(&key, &other_did, &sample_payload());

            let verdict = verify_profile_envelope(&envelope, None, 1_000);
            assert!(!verdict.verified);
            let reason = verdict.reason.unwrap_or_default();
            assert!(reason.contains("does not match its issuer"), "{reason}");
        }

        #[test]
        fn a_contact_profile_must_be_issued_by_the_contact() {
            let key = Identity::generate().unwrap();
            let contact = Identity::generate().unwrap();
            let contact_did = substrate::derive_did_key(&contact.public_key());
            let envelope = signed_profile(&key, &contact_did, &sample_payload());

            let verdict = verify_profile_envelope(&envelope, Some(&contact_did), 1_000);
            assert!(!verdict.verified);
            let reason = verdict.reason.unwrap_or_default();
            assert!(reason.starts_with("profile did not verify:"), "{reason}");

            let envelope = signed_profile(&contact, &contact_did, &sample_payload());
            let verdict = verify_profile_envelope(&envelope, Some(&contact_did), 1_000);
            assert!(verdict.verified, "{:?}", verdict.reason);
        }

        #[test]
        fn a_profile_payload_that_does_not_validate_is_refused() {
            let key = Identity::generate().unwrap();
            let did = substrate::derive_did_key(&key.public_key());
            let mut payload = sample_payload();
            payload.display_name = String::new();
            let envelope = signed_profile(&key, &did, &payload);

            let verdict = verify_profile_envelope(&envelope, None, 1_000);
            assert!(!verdict.verified);
        }

        #[test]
        fn a_profile_with_a_broken_signature_is_refused() {
            let key = Identity::generate().unwrap();
            let did = substrate::derive_did_key(&key.public_key());
            let mut envelope: serde_json::Value =
                serde_json::from_str(&signed_profile(&key, &did, &sample_payload())).unwrap();
            envelope["signature"] = serde_json::Value::String("zbroken".to_string());
            let verdict = verify_profile_envelope(&envelope.to_string(), None, 1_000);
            assert!(!verdict.verified);
        }
    }
}
