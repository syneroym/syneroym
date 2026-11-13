use serde_json::Value;
use syneroym_identity::{Identity, substrate};

use super::{lang::sample_tag, *};

/// A receipt payload for a quote whose original language is `en`.
fn receipt(counterparty_did: &str, role: DealRole, lang: &str, via_mt: bool) -> Value {
    json!({
        "quote_record_id": "rec_quote123",
        "counterparty_did": counterparty_did,
        "quote_author_did": "did:key:zQuoteAuthorViewed",
        "role": role,
        "terms": sample_terms(),
        "viewed_lang": lang,
        "viewed_via_mt": via_mt,
    })
}

#[test]
fn receipt_requires_viewed_fields() {
    let full = receipt("did:key:zCounterpartyViewed", DealRole::Counterparty, "en", false);
    assert!(serde_json::from_value::<AgreementReceiptPayload>(full.clone()).is_ok());
    for missing in ["viewed_lang", "viewed_via_mt"] {
        let mut payload = full.clone();
        payload.as_object_mut().unwrap().remove(missing);
        let err = serde_json::from_value::<AgreementReceiptPayload>(payload).unwrap_err();
        assert!(err.to_string().contains(missing), "{missing}: {err}");
    }
}

#[test]
fn receipt_refuses_non_canonical_viewed_lang() {
    for bad in ["HI-latn", "hi_IN"] {
        let payload = receipt("did:key:zCounterpartyViewed", DealRole::Counterparty, bad, false);
        assert!(
            serde_json::from_value::<AgreementReceiptPayload>(payload).is_err(),
            "{bad} must not deserialise"
        );
    }
}

#[test]
fn receipt_refuses_mt_of_the_original() {
    let mut payload = AgreementReceiptPayload {
        quote_record_id: "rec_quote123".to_string(),
        counterparty_did: "did:key:zCounterpartyViewed".to_string(),
        quote_author_did: "did:key:zQuoteAuthorViewed".to_string(),
        role: DealRole::Counterparty,
        terms: sample_terms(),
        viewed_lang: sample_tag("en"),
        viewed_via_mt: true,
    };
    assert_eq!(payload.validate(), Err(TransactionError::ViewedMtOfOriginal));

    payload.viewed_lang = sample_tag("hi");
    assert_eq!(payload.validate(), Ok(()));
}

#[test]
fn halves_differing_in_viewed_fields_still_agree() {
    let half = |role, lang: &str, via_mt| AgreementReceiptPayload {
        quote_record_id: "rec_quote123".to_string(),
        counterparty_did: "did:key:zCounterpartyViewed".to_string(),
        quote_author_did: "did:key:zQuoteAuthorViewed".to_string(),
        role,
        terms: sample_terms(),
        viewed_lang: sample_tag(lang),
        viewed_via_mt: via_mt,
    };
    let counterparty = half(DealRole::Counterparty, "hi", true);
    let author = half(DealRole::QuoteAuthor, "en", false);
    assert!(halves_agree(&counterparty, &author));
}

#[test]
fn verify_round_trips_viewed_fields() {
    let key = Identity::generate().unwrap();
    let did = substrate::derive_did_key(&key.public_key());
    let payload = receipt(&did, DealRole::Counterparty, "hi", true);
    let env = sign_envelope(
        &key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        "rec_quote123",
        payload,
        None,
        1000,
    );

    let verdict = verify_agreement_receipt(&env, 1000);
    assert!(verdict.verified, "{:?}", verdict.reason);
    let read = verdict.payload.unwrap();
    assert_eq!(read.viewed_lang, sample_tag("hi"));
    assert!(read.viewed_via_mt);
}
