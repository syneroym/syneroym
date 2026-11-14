use syneroym_identity::{Identity, substrate};

use super::*;
use crate::lang::TranslationError;

pub(super) fn sample_tag(s: &str) -> LangTag {
    LangTag::parse(s).expect("valid tag")
}

#[test]
fn request_translation_validation() {
    let mut req = sample_request("did:key:zRequester");
    req.lang = sample_tag("hi");

    // 5 translations exceeds limit of 4
    let tags = ["ta", "bn", "gu", "kn", "ml"];
    req.translations = tags
        .into_iter()
        .map(|t| RequestTranslation {
            lang: sample_tag(t),
            description: format!("Description {t}"),
        })
        .collect();
    assert_eq!(req.validate(), Err(TransactionError::Translation(TranslationError::TooMany)));

    // 4 translations pass
    req.translations.pop();
    assert!(req.validate().is_ok());

    // Same tag as record
    req.translations = vec![RequestTranslation {
        lang: sample_tag("hi"),
        description: "Same as record".to_string(),
    }];
    assert_eq!(
        req.validate(),
        Err(TransactionError::Translation(TranslationError::SameAsRecord("hi".to_string())))
    );

    // Duplicate tags
    req.translations = vec![
        RequestTranslation { lang: sample_tag("en"), description: "English 1".to_string() },
        RequestTranslation { lang: sample_tag("en"), description: "English 2".to_string() },
    ];
    assert_eq!(
        req.validate(),
        Err(TransactionError::Translation(TranslationError::Duplicate("en".to_string())))
    );

    // Empty description
    req.translations =
        vec![RequestTranslation { lang: sample_tag("en"), description: "   ".to_string() }];
    assert_eq!(
        req.validate(),
        Err(TransactionError::Translation(TranslationError::Empty {
            lang: "en".to_string(),
            field: "description",
        }))
    );

    // Description too long
    req.translations = vec![RequestTranslation {
        lang: sample_tag("en"),
        description: "a".repeat(MAX_DESCRIPTION_LEN + 1),
    }];
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
fn quote_translation_count_and_tags() {
    let mut quote = sample_quote("did:key:zAuthor", "did:key:zCounterparty");
    quote.terms.lang = sample_tag("hi");

    // 5 translations exceeds limit of 4
    let tags = ["ta", "bn", "gu", "kn", "ml"];
    quote.translations = tags
        .into_iter()
        .map(|t| QuoteTranslation {
            lang: sample_tag(t),
            scope: format!("Scope {t}"),
            cancellation_terms: format!("Cancel {t}"),
            refund_terms: format!("Refund {t}"),
            dispute_path: format!("Dispute {t}"),
        })
        .collect();
    assert_eq!(quote.validate(), Err(TransactionError::Translation(TranslationError::TooMany)));

    // 4 translations pass
    quote.translations.pop();
    assert!(quote.validate().is_ok());

    // Same tag as terms.lang
    quote.translations = vec![QuoteTranslation {
        lang: sample_tag("hi"),
        scope: "Scope".to_string(),
        cancellation_terms: "Cancel".to_string(),
        refund_terms: "Refund".to_string(),
        dispute_path: "Dispute".to_string(),
    }];
    assert_eq!(
        quote.validate(),
        Err(TransactionError::Translation(TranslationError::SameAsRecord("hi".to_string())))
    );

    // Duplicate tags
    quote.translations = vec![
        QuoteTranslation {
            lang: sample_tag("en"),
            scope: "Scope 1".to_string(),
            cancellation_terms: "Cancel 1".to_string(),
            refund_terms: "Refund 1".to_string(),
            dispute_path: "Dispute 1".to_string(),
        },
        QuoteTranslation {
            lang: sample_tag("en"),
            scope: "Scope 2".to_string(),
            cancellation_terms: "Cancel 2".to_string(),
            refund_terms: "Refund 2".to_string(),
            dispute_path: "Dispute 2".to_string(),
        },
    ];
    assert_eq!(
        quote.validate(),
        Err(TransactionError::Translation(TranslationError::Duplicate("en".to_string())))
    );
}

#[test]
fn quote_translation_field_validation() {
    let mut quote = sample_quote("did:key:zAuthor", "did:key:zCounterparty");
    quote.terms.lang = sample_tag("hi");

    // Empty scope
    quote.translations = vec![QuoteTranslation {
        lang: sample_tag("en"),
        scope: "   ".to_string(),
        cancellation_terms: "Cancel".to_string(),
        refund_terms: "Refund".to_string(),
        dispute_path: "Dispute".to_string(),
    }];
    assert_eq!(
        quote.validate(),
        Err(TransactionError::Translation(TranslationError::Empty {
            lang: "en".to_string(),
            field: "scope",
        }))
    );

    // Scope too long
    quote.translations = vec![QuoteTranslation {
        lang: sample_tag("en"),
        scope: "a".repeat(MAX_SCOPE_LEN + 1),
        cancellation_terms: "Cancel".to_string(),
        refund_terms: "Refund".to_string(),
        dispute_path: "Dispute".to_string(),
    }];
    assert_eq!(
        quote.validate(),
        Err(TransactionError::Translation(TranslationError::TooLong {
            lang: "en".to_string(),
            field: "scope",
            max: MAX_SCOPE_LEN,
        }))
    );

    // Empty cancellation_terms
    quote.translations = vec![QuoteTranslation {
        lang: sample_tag("en"),
        scope: "Scope".to_string(),
        cancellation_terms: "   ".to_string(),
        refund_terms: "Refund".to_string(),
        dispute_path: "Dispute".to_string(),
    }];
    assert_eq!(
        quote.validate(),
        Err(TransactionError::Translation(TranslationError::Empty {
            lang: "en".to_string(),
            field: "cancellation_terms",
        }))
    );

    // Empty refund_terms
    quote.translations = vec![QuoteTranslation {
        lang: sample_tag("en"),
        scope: "Scope".to_string(),
        cancellation_terms: "Cancel".to_string(),
        refund_terms: "   ".to_string(),
        dispute_path: "Dispute".to_string(),
    }];
    assert_eq!(
        quote.validate(),
        Err(TransactionError::Translation(TranslationError::Empty {
            lang: "en".to_string(),
            field: "refund_terms",
        }))
    );

    // Empty dispute_path
    quote.translations = vec![QuoteTranslation {
        lang: sample_tag("en"),
        scope: "Scope".to_string(),
        cancellation_terms: "Cancel".to_string(),
        refund_terms: "Refund".to_string(),
        dispute_path: "   ".to_string(),
    }];
    assert_eq!(
        quote.validate(),
        Err(TransactionError::Translation(TranslationError::Empty {
            lang: "en".to_string(),
            field: "dispute_path",
        }))
    );
}

#[test]
fn request_verify_refuses_missing_malformed_and_non_canonical_lang() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let req = sample_request(&issuer);
    let base_val = serde_json::to_value(&req).unwrap();

    // Missing lang
    let mut missing_lang = base_val.clone();
    missing_lang.as_object_mut().unwrap().remove("lang");
    let env_missing = sign_envelope(
        &key,
        RECORD_REQUEST,
        REQUEST_VERSION,
        &req.request_id,
        missing_lang,
        None,
        1000,
    );
    let v_missing = verify_request(&env_missing, 1000);
    assert!(!v_missing.verified);
    assert!(
        v_missing.reason.as_deref().unwrap_or("").contains("payload: missing field `lang`"),
        "expected missing field lang refusal: {:?}",
        v_missing.reason
    );

    // Malformed lang: hi_IN
    let mut malformed = base_val.clone();
    malformed["lang"] = serde_json::json!("hi_IN");
    let env_malformed = sign_envelope(
        &key,
        RECORD_REQUEST,
        REQUEST_VERSION,
        &req.request_id,
        malformed,
        None,
        1000,
    );
    let v_malformed = verify_request(&env_malformed, 1000);
    assert!(!v_malformed.verified);

    // Non-canonical lang: HI
    let mut non_canonical = base_val.clone();
    non_canonical["lang"] = serde_json::json!("HI");
    let env_non_canonical = sign_envelope(
        &key,
        RECORD_REQUEST,
        REQUEST_VERSION,
        &req.request_id,
        non_canonical,
        None,
        1000,
    );
    let v_non_canonical = verify_request(&env_non_canonical, 1000);
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
fn quote_verify_refuses_missing_malformed_and_non_canonical_terms_lang() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let quote = sample_quote(&issuer, "did:key:zCounterparty");
    let base_val = serde_json::to_value(&quote).unwrap();

    // Missing terms.lang
    let mut missing_lang = base_val.clone();
    missing_lang["terms"].as_object_mut().unwrap().remove("lang");
    let env_missing = sign_envelope(
        &key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote.quote_id,
        missing_lang,
        Some(quote.terms.quote_expires_at_secs),
        1000,
    );
    let v_missing = verify_quote(&env_missing, 1000);
    assert!(!v_missing.verified);
    assert!(
        v_missing.reason.as_deref().unwrap_or("").contains("missing field `lang`"),
        "expected missing field lang refusal: {:?}",
        v_missing.reason
    );

    // Malformed terms.lang: hi_IN
    let mut malformed = base_val.clone();
    malformed["terms"]["lang"] = serde_json::json!("hi_IN");
    let env_malformed = sign_envelope(
        &key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote.quote_id,
        malformed,
        Some(quote.terms.quote_expires_at_secs),
        1000,
    );
    let v_malformed = verify_quote(&env_malformed, 1000);
    assert!(!v_malformed.verified);

    // Non-canonical terms.lang: HI
    let mut non_canonical = base_val.clone();
    non_canonical["terms"]["lang"] = serde_json::json!("HI");
    let env_non_canonical = sign_envelope(
        &key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote.quote_id,
        non_canonical,
        Some(quote.terms.quote_expires_at_secs),
        1000,
    );
    let v_non_canonical = verify_quote(&env_non_canonical, 1000);
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
    terms_hi.lang = sample_tag("hi");

    let half_c = AgreementReceiptPayload {
        quote_record_id: "rec_quote123".to_string(),
        counterparty_did: "did:key:zCounterparty".to_string(),
        quote_author_did: "did:key:zAuthor".to_string(),
        role: DealRole::Counterparty,
        viewed_lang: base_terms.lang.clone(),
        viewed_via_mt: false,
        terms: base_terms,
    };
    let half_a = AgreementReceiptPayload {
        quote_record_id: "rec_quote123".to_string(),
        counterparty_did: "did:key:zCounterparty".to_string(),
        quote_author_did: "did:key:zAuthor".to_string(),
        role: DealRole::QuoteAuthor,
        viewed_lang: terms_hi.lang.clone(),
        viewed_via_mt: false,
        terms: terms_hi,
    };
    assert!(!halves_agree(&half_c, &half_a));
}

#[test]
fn quote_with_five_versions_at_maximum_length_signs_and_verifies() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let counterparty_key = Identity::generate().unwrap();
    let counterparty_did = substrate::derive_did_key(&counterparty_key.public_key());

    let mut quote = sample_quote(&issuer, &counterparty_did);
    quote.terms.scope = "s".repeat(MAX_SCOPE_LEN);
    quote.terms.cancellation_terms = "c".repeat(MAX_TERMS_TEXT_LEN);
    quote.terms.refund_terms = "r".repeat(MAX_TERMS_TEXT_LEN);
    quote.terms.dispute_path = "d".repeat(MAX_TERMS_TEXT_LEN);

    let langs = ["hi", "ta", "bn", "mr"];
    quote.translations = langs
        .into_iter()
        .map(|l| QuoteTranslation {
            lang: sample_tag(l),
            scope: "S".repeat(MAX_SCOPE_LEN),
            cancellation_terms: "C".repeat(MAX_TERMS_TEXT_LEN),
            refund_terms: "R".repeat(MAX_TERMS_TEXT_LEN),
            dispute_path: "D".repeat(MAX_TERMS_TEXT_LEN),
        })
        .collect();

    assert!(quote.validate().is_ok());

    let payload_val = serde_json::to_value(&quote).unwrap();
    let env_json = sign_envelope(
        &key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote.quote_id,
        payload_val,
        Some(quote.terms.quote_expires_at_secs),
        1000,
    );

    let verdict = verify_quote(&env_json, 1000);
    assert!(verdict.verified, "{:?}", verdict.reason);
    assert_eq!(verdict.payload.unwrap().translations.len(), 4);
}
