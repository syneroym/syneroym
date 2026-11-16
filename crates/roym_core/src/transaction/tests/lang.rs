use syneroym_identity::{Identity, substrate};

use super::*;
use crate::{deal::TextTranslation, lang::TranslationError};

pub(super) fn sample_tag(s: &str) -> LangTag {
    LangTag::parse(s).expect("valid tag")
}

#[test]
fn enquiry_translation_validation() {
    let mut req = sample_enquiry("did:key:zRequester");
    req.description.lang = sample_tag("hi");

    // 5 translations exceeds limit of 4
    let tags = ["ta", "bn", "gu", "kn", "ml"];
    req.description.translations = tags
        .into_iter()
        .map(|t| TextTranslation { lang: sample_tag(t), text: format!("Description {t}") })
        .collect();
    assert_eq!(req.validate(), Err(TransactionError::Translation(TranslationError::TooMany)));

    // 4 translations pass
    req.description.translations.pop();
    assert!(req.validate().is_ok());

    // Same tag as record
    req.description.translations =
        vec![TextTranslation { lang: sample_tag("hi"), text: "Same as record".to_string() }];
    assert_eq!(
        req.validate(),
        Err(TransactionError::Translation(TranslationError::SameAsRecord("hi".to_string())))
    );

    // Duplicate tags
    req.description.translations = vec![
        TextTranslation { lang: sample_tag("en"), text: "English 1".to_string() },
        TextTranslation { lang: sample_tag("en"), text: "English 2".to_string() },
    ];
    assert_eq!(
        req.validate(),
        Err(TransactionError::Translation(TranslationError::Duplicate("en".to_string())))
    );

    // Empty description
    req.description.translations =
        vec![TextTranslation { lang: sample_tag("en"), text: "   ".to_string() }];
    assert_eq!(
        req.validate(),
        Err(TransactionError::Translation(TranslationError::Empty {
            lang: "en".to_string(),
            field: "description",
        }))
    );

    // Description too long
    req.description.translations =
        vec![TextTranslation { lang: sample_tag("en"), text: "a".repeat(MAX_DESCRIPTION_LEN + 1) }];
    assert_eq!(
        req.validate(),
        Err(TransactionError::Translation(TranslationError::TooLong {
            lang: "en".to_string(),
            field: "description",
            max: MAX_DESCRIPTION_LEN,
        }))
    );
}

#[test]
fn enquiry_verify_refuses_missing_malformed_and_non_canonical_lang() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let req = sample_enquiry(&issuer);
    let base_val = serde_json::to_value(&req).unwrap();

    // Missing lang inside description
    let mut missing_lang = base_val.clone();
    missing_lang["description"].as_object_mut().unwrap().remove("lang");
    let env_missing = sign_envelope(
        &key,
        RECORD_ENQUIRY,
        ENQUIRY_VERSION,
        &req.enquiry_id,
        missing_lang,
        None,
        1000,
    );
    let v_missing = verify_enquiry(&env_missing, 1000);
    assert!(!v_missing.verified);
    assert!(
        v_missing.reason.as_deref().unwrap_or("").contains("missing field `lang`"),
        "expected missing field lang refusal: {:?}",
        v_missing.reason
    );

    // Malformed lang: hi_IN
    let mut malformed = base_val.clone();
    malformed["description"]["lang"] = serde_json::json!("hi_IN");
    let env_malformed = sign_envelope(
        &key,
        RECORD_ENQUIRY,
        ENQUIRY_VERSION,
        &req.enquiry_id,
        malformed,
        None,
        1000,
    );
    let v_malformed = verify_enquiry(&env_malformed, 1000);
    assert!(!v_malformed.verified);

    // Non-canonical lang: HI
    let mut non_canonical = base_val.clone();
    non_canonical["description"]["lang"] = serde_json::json!("HI");
    let env_non_canonical = sign_envelope(
        &key,
        RECORD_ENQUIRY,
        ENQUIRY_VERSION,
        &req.enquiry_id,
        non_canonical,
        None,
        1000,
    );
    let v_non_canonical = verify_enquiry(&env_non_canonical, 1000);
    assert!(!v_non_canonical.verified);
    assert!(
        v_non_canonical
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("is not in canonical form; send 'hi'"),
        "expected non-canonical refusal: {:?}",
        v_non_canonical.reason
    );
}

#[test]
fn halves_agree_differs_on_terms_lang() {
    let base_terms = sample_terms();
    let mut terms_hi = base_terms.clone();
    terms_hi.scope.lang = sample_tag("hi");
    terms_hi.commitments[0].description.lang = sample_tag("hi");
    terms_hi.cancellation_terms.lang = sample_tag("hi");
    terms_hi.refund_terms.lang = sample_tag("hi");
    terms_hi.dispute_path.lang = sample_tag("hi");

    let half_c = AgreementReceiptPayload {
        quote_record_id: "rec_quote123".to_string(),
        counterparty_did: "did:key:zCounterparty".to_string(),
        quote_author_did: "did:key:zAuthor".to_string(),
        role: DealRole::Counterparty,
        viewed_lang: base_terms.scope.lang.clone(),
        viewed_via_mt: false,
        terms: base_terms,
    };
    let half_a = AgreementReceiptPayload {
        quote_record_id: "rec_quote123".to_string(),
        counterparty_did: "did:key:zCounterparty".to_string(),
        quote_author_did: "did:key:zAuthor".to_string(),
        role: DealRole::QuoteAuthor,
        viewed_lang: terms_hi.scope.lang.clone(),
        viewed_via_mt: false,
        terms: terms_hi,
    };
    assert!(!halves_agree(&half_c, &half_a));
}
