#![allow(clippy::cognitive_complexity)]

use syneroym_identity::{Identity, substrate};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::*;

mod lang;
mod verify;
mod viewed;

pub(super) fn sample_terms() -> AgreedTerms {
    AgreedTerms {
        lang: LangTag::parse("en").unwrap(),
        scope: "Install garden fence".to_string(),
        currency: "EUR".to_string(),
        amount_minor: 50000,
        tax_minor: 5000,
        fees_minor: 2000,
        payment_methods: vec!["card".to_string(), "cash".to_string()],
        payee: "Garden Services Ltd".to_string(),
        payment_timing: PaymentTiming::AfterWork,
        schedule: Some(TimeWindow { earliest_secs: 10000, latest_secs: 20000 }),
        location: QuoteLocation {
            where_: ServiceLocation::AtReceiver,
            area: None,
            address: Some("123 Flower St".to_string()),
        },
        cancellation_terms: "Cancel 24h prior for full refund".to_string(),
        refund_terms: "Full refund if unsatisfactory".to_string(),
        dispute_path: "Contact disputes@example.com".to_string(),
        quote_expires_at_secs: 15000,
    }
}

pub(super) fn sample_request(issuer: &str) -> RequestPayload {
    let conversation = "conv_123456";
    let sequence = 1;
    let request_id = derive_request_id(conversation, issuer, sequence).unwrap();
    RequestPayload {
        request_id,
        conversation: conversation.to_string(),
        sequence,
        listing_id: Some("lst_abc123".to_string()),
        categories: vec!["gardening".to_string()],
        description: "Need fence installed".to_string(),
        lang: LangTag::parse("en").unwrap(),
        translations: vec![],
        area: None,
        window: Some(TimeWindow { earliest_secs: 10000, latest_secs: 20000 }),
        data_use_notice: DEFAULT_DATA_USE_NOTICE.to_string(),
    }
}

pub(super) fn sample_quote(issuer: &str, counterparty_did: &str) -> QuotePayload {
    let conversation = "conv_123456";
    let sequence = 1;
    let quote_id = derive_quote_id(conversation, issuer, sequence).unwrap();
    QuotePayload {
        quote_id,
        conversation: conversation.to_string(),
        sequence,
        request_record_id: "rec_req123".to_string(),
        listing_id: Some("lst_abc123".to_string()),
        slot_id: None,
        counterparty_did: counterparty_did.to_string(),
        terms: sample_terms(),
        translations: vec![],
    }
}

pub(super) fn sign_envelope(
    key: &Identity,
    record_type: &str,
    version: u32,
    subject: &str,
    payload: serde_json::Value,
    expires_at_secs: Option<u64>,
    now_secs: u64,
) -> String {
    let issuer = substrate::derive_did_key(&key.public_key());
    let draft = RecordDraft {
        version,
        record_type: record_type.to_string(),
        subject: subject.to_string(),
        payload,
        expires_at_secs,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, now_secs).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    env.to_json().unwrap()
}

#[test]
fn time_window_validate() {
    let valid = TimeWindow { earliest_secs: 100, latest_secs: 200 };
    assert!(valid.validate().is_ok());
    let equal = TimeWindow { earliest_secs: 100, latest_secs: 100 };
    assert!(equal.validate().is_ok());
    let invalid = TimeWindow { earliest_secs: 200, latest_secs: 100 };
    assert!(matches!(invalid.validate(), Err(TransactionError::InvalidTimeWindow { .. })));
}

#[test]
fn quote_location_validate() {
    let at_cust = QuoteLocation {
        where_: ServiceLocation::AtReceiver,
        area: None,
        address: Some("123 Main St".to_string()),
    };
    assert!(at_cust.validate().is_ok());

    let remote_with_addr = QuoteLocation {
        where_: ServiceLocation::Remote,
        area: None,
        address: Some("123 Main St".to_string()),
    };
    assert_eq!(remote_with_addr.validate(), Err(TransactionError::AddressNotApplicable));

    let long_addr = QuoteLocation {
        where_: ServiceLocation::AtReceiver,
        area: None,
        address: Some("a".repeat(MAX_ADDRESS_LEN + 1)),
    };
    assert_eq!(long_addr.validate(), Err(TransactionError::AddressTooLong));
}

#[test]
fn agreed_terms_validate_rules() {
    let mut t = sample_terms();
    assert!(t.validate().is_ok());

    // Scope empty / too long
    t.scope = "   ".to_string();
    assert_eq!(t.validate(), Err(TransactionError::ScopeEmpty));
    t.scope = "a".repeat(MAX_SCOPE_LEN + 1);
    assert_eq!(t.validate(), Err(TransactionError::ScopeTooLong));
    t.scope = "Valid scope".to_string();

    // Currency unknown
    t.currency = "XYZ".to_string();
    assert!(matches!(t.validate(), Err(TransactionError::CurrencyUnknown(_))));
    t.currency = "EUR".to_string();

    // Negative amounts
    t.amount_minor = -1;
    assert!(matches!(t.validate(), Err(TransactionError::NegativeAmount(-1))));
    t.amount_minor = 50000;

    t.tax_minor = -1;
    assert!(matches!(t.validate(), Err(TransactionError::NegativeTax(-1))));
    t.tax_minor = 5000;

    t.fees_minor = -1;
    assert!(matches!(t.validate(), Err(TransactionError::NegativeFees(-1))));
    t.fees_minor = 2000;

    // Breakdown exceeds total (including checked_add overflow)
    t.tax_minor = 40000;
    t.fees_minor = 20000; // 40000 + 20000 = 60000 > 50000
    assert!(matches!(t.validate(), Err(TransactionError::BreakdownExceedsTotal { .. })));
    t.tax_minor = i64::MAX;
    t.fees_minor = 1;
    assert!(matches!(t.validate(), Err(TransactionError::BreakdownExceedsTotal { .. })));
    t.tax_minor = 5000;
    t.fees_minor = 2000;

    // Payee
    t.payee = "".to_string();
    assert_eq!(t.validate(), Err(TransactionError::PayeeEmpty));
    t.payee = "p".repeat(MAX_PAYEE_LEN + 1);
    assert_eq!(t.validate(), Err(TransactionError::PayeeTooLong));
    t.payee = "Valid Payee".to_string();

    // Payment methods
    t.payment_methods = (0..MAX_PAYMENT_METHODS + 1).map(|i| format!("m{i}")).collect();
    assert_eq!(t.validate(), Err(TransactionError::TooManyPaymentMethods));
    for bad in ["x".repeat(MAX_PAYMENT_METHOD_LEN + 1), "UPI".into(), " ".into()] {
        t.payment_methods = vec![bad.clone()];
        assert_eq!(t.validate(), Err(TransactionError::PaymentMethodShape(bad)));
    }
    t.payment_methods = vec!["card".to_string()];

    // Cancellation terms
    t.cancellation_terms = "".to_string();
    assert_eq!(t.validate(), Err(TransactionError::CancellationTermsEmpty));
    t.cancellation_terms = "x".repeat(MAX_TERMS_TEXT_LEN + 1);
    assert_eq!(t.validate(), Err(TransactionError::CancellationTermsTooLong));
    t.cancellation_terms = "Valid cancel".to_string();

    // Refund terms
    t.refund_terms = "".to_string();
    assert_eq!(t.validate(), Err(TransactionError::RefundTermsEmpty));
    t.refund_terms = "x".repeat(MAX_TERMS_TEXT_LEN + 1);
    assert_eq!(t.validate(), Err(TransactionError::RefundTermsTooLong));
    t.refund_terms = "Valid refund".to_string();

    // Dispute path
    t.dispute_path = "".to_string();
    assert_eq!(t.validate(), Err(TransactionError::DisputePathEmpty));
    t.dispute_path = "x".repeat(MAX_TERMS_TEXT_LEN + 1);
    assert_eq!(t.validate(), Err(TransactionError::DisputePathTooLong));
    t.dispute_path = "Valid dispute".to_string();

    // Quote expiry zero
    t.quote_expires_at_secs = 0;
    assert_eq!(t.validate(), Err(TransactionError::QuoteExpiryZero));
    t.quote_expires_at_secs = 1000;
    assert!(t.validate().is_ok());
}

#[test]
fn request_payload_validate_rules() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let mut r = sample_request(&issuer);
    assert!(r.validate().is_ok());

    r.request_id = "".to_string();
    assert_eq!(r.validate(), Err(TransactionError::RequestIdEmpty));
    r.request_id = "req_123".to_string();

    r.conversation = "".to_string();
    assert_eq!(r.validate(), Err(TransactionError::ConversationEmpty));
    r.conversation = "conv_123".to_string();

    r.sequence = 0;
    assert_eq!(r.validate(), Err(TransactionError::InvalidSequence(0)));
    r.sequence = 1;

    r.description = "   ".to_string();
    assert_eq!(r.validate(), Err(TransactionError::DescriptionEmpty));
    r.description = "d".repeat(MAX_DESCRIPTION_LEN + 1);
    assert_eq!(r.validate(), Err(TransactionError::DescriptionTooLong));
    r.description = "Valid description".to_string();

    r.categories = (0..MAX_CATEGORIES + 1).map(|i| format!("c{i}")).collect();
    assert_eq!(r.validate(), Err(TransactionError::TooManyCategories));
    r.categories = vec!["UPPERCASE".to_string()];
    assert!(matches!(r.validate(), Err(TransactionError::InvalidCategory(_))));
    r.categories = vec!["valid-cat".to_string()];

    r.data_use_notice = "".to_string();
    assert_eq!(r.validate(), Err(TransactionError::DataUseNoticeEmpty));
    r.data_use_notice = "n".repeat(MAX_NOTICE_LEN + 1);
    assert_eq!(r.validate(), Err(TransactionError::DataUseNoticeTooLong));
    r.data_use_notice = DEFAULT_DATA_USE_NOTICE.to_string();
    assert!(r.validate().is_ok());
}

#[test]
fn quote_payload_validate_rules() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let counterparty_key = Identity::generate().unwrap();
    let counterparty_did = substrate::derive_did_key(&counterparty_key.public_key());

    let mut q = sample_quote(&issuer, &counterparty_did);
    assert!(q.validate().is_ok());

    q.quote_id = "".to_string();
    assert_eq!(q.validate(), Err(TransactionError::QuoteIdEmpty));
    q.quote_id = "quo_123".to_string();

    q.conversation = "".to_string();
    assert_eq!(q.validate(), Err(TransactionError::ConversationEmpty));
    q.conversation = "conv_123".to_string();

    q.sequence = 0;
    assert_eq!(q.validate(), Err(TransactionError::InvalidSequence(0)));
    q.sequence = 1;

    q.request_record_id = "bad_prefix".to_string();
    assert!(matches!(q.validate(), Err(TransactionError::InvalidRecordId(_))));
    q.request_record_id = "rec_req123".to_string();

    q.counterparty_did = "not-a-did".to_string();
    assert!(matches!(q.validate(), Err(TransactionError::InvalidDid(_))));
    q.counterparty_did = counterparty_did;
    assert!(q.validate().is_ok());
}

#[test]
fn agreement_receipt_payload_validate_rules() {
    let counterparty_key = Identity::generate().unwrap();
    let counterparty_did = substrate::derive_did_key(&counterparty_key.public_key());
    let quote_author_key = Identity::generate().unwrap();
    let quote_author_did = substrate::derive_did_key(&quote_author_key.public_key());

    let mut a = AgreementReceiptPayload {
        quote_record_id: "rec_quote123".to_string(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: quote_author_did.clone(),
        role: DealRole::Counterparty,
        terms: sample_terms(),
        viewed_lang: sample_terms().lang,
        viewed_via_mt: false,
    };
    assert!(a.validate().is_ok());

    a.quote_record_id = "bad_prefix".to_string();
    assert!(matches!(a.validate(), Err(TransactionError::InvalidRecordId(_))));
    a.quote_record_id = "rec_quote123".to_string();

    a.counterparty_did = "invalid".to_string();
    assert!(matches!(a.validate(), Err(TransactionError::InvalidDid(_))));
    a.counterparty_did = counterparty_did.clone();

    a.quote_author_did = "invalid".to_string();
    assert!(matches!(a.validate(), Err(TransactionError::InvalidDid(_))));
    a.quote_author_did = quote_author_did.clone();

    a.counterparty_did = quote_author_did;
    assert_eq!(a.validate(), Err(TransactionError::SamePartyDid));
    a.counterparty_did = counterparty_did;
    assert!(a.validate().is_ok());
}

#[test]
fn derive_request_id_and_quote_id_are_stable_and_differ_on_any_input_change() {
    let id1 = derive_request_id("c1", "did:key:1", 1).unwrap();
    let id2 = derive_request_id("c1", "did:key:1", 1).unwrap();
    assert_eq!(id1, id2);
    assert!(id1.starts_with(REQUEST_ID_PREFIX));

    // Changing conversation
    assert_ne!(id1, derive_request_id("c2", "did:key:1", 1).unwrap());
    // Changing issuer
    assert_ne!(id1, derive_request_id("c1", "did:key:2", 1).unwrap());
    // Changing sequence
    assert_ne!(id1, derive_request_id("c1", "did:key:1", 2).unwrap());

    // Quote id
    let q1 = derive_quote_id("c1", "did:key:1", 1).unwrap();
    let q2 = derive_quote_id("c1", "did:key:1", 1).unwrap();
    assert_eq!(q1, q2);
    assert!(q1.starts_with(QUOTE_ID_PREFIX));
    assert_ne!(id1, q1);
    assert_ne!(q1, derive_quote_id("c2", "did:key:1", 1).unwrap());
}

#[test]
fn halves_agree_table_test() {
    let counterparty_did = "did:key:zCounterparty".to_string();
    let quote_author_did = "did:key:zQuoteAuthor".to_string();
    let terms = sample_terms();

    let base_counterparty = AgreementReceiptPayload {
        quote_record_id: "rec_quo123".to_string(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: quote_author_did.clone(),
        role: DealRole::Counterparty,
        terms: terms.clone(),
        viewed_lang: terms.lang.clone(),
        viewed_via_mt: false,
    };
    let base_quote_author = AgreementReceiptPayload {
        quote_record_id: "rec_quo123".to_string(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: quote_author_did.clone(),
        role: DealRole::QuoteAuthor,
        terms: terms.clone(),
        viewed_lang: terms.lang.clone(),
        viewed_via_mt: false,
    };

    // Identical except role -> true
    assert!(halves_agree(&base_counterparty, &base_quote_author));
    assert!(halves_agree(&base_quote_author, &base_counterparty));

    // Same role -> false
    assert!(!halves_agree(&base_counterparty, &base_counterparty));
    assert!(!halves_agree(&base_quote_author, &base_quote_author));

    // Differ in quote_record_id -> false
    let mut diff_quote = base_quote_author.clone();
    diff_quote.quote_record_id = "rec_diff".to_string();
    assert!(!halves_agree(&base_counterparty, &diff_quote));

    // Differ in counterparty_did -> false
    let mut diff_cdid = base_quote_author.clone();
    diff_cdid.counterparty_did = "did:key:zOther".to_string();
    assert!(!halves_agree(&base_counterparty, &diff_cdid));

    // Differ in quote_author_did -> false
    let mut diff_pdid = base_quote_author.clone();
    diff_pdid.quote_author_did = "did:key:zOther".to_string();
    assert!(!halves_agree(&base_counterparty, &diff_pdid));

    // Differ in terms (e.g. amount) -> false
    let mut diff_terms = base_quote_author.clone();
    diff_terms.terms.amount_minor = 99999;
    assert!(!halves_agree(&base_counterparty, &diff_terms));
}

#[test]
fn agreed_terms_validate_refuses_xyz_as_currency() {
    let mut t = sample_terms();
    t.currency = "XYZ".to_string();
    assert!(matches!(t.validate(), Err(TransactionError::CurrencyUnknown(c)) if c == "XYZ"));
}

#[test]
fn pair_state_transitions() {
    let half_c = ReceiptHalf {
        envelope: "{}".to_string(),
        record_id: "rec_c".to_string(),
        issuer: "did:key:c".to_string(),
        issued_at_secs: 1000,
    };
    let half_p = ReceiptHalf {
        envelope: "{}".to_string(),
        record_id: "rec_p".to_string(),
        issuer: "did:key:p".to_string(),
        issued_at_secs: 1000,
    };

    assert_eq!(pair_state(None, None), PairState::None);
    assert_eq!(pair_state(Some(&half_c), None), PairState::Half { role: DealRole::Counterparty });
    assert_eq!(pair_state(None, Some(&half_p)), PairState::Half { role: DealRole::QuoteAuthor });
    assert_eq!(pair_state(Some(&half_c), Some(&half_p)), PairState::Complete);
}

#[test]
fn the_ui_notices_match_this_crate() {
    use std::{fs, path::PathBuf};

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir.join("../roym_web/ui/src/i18n/en.ftl");
    assert!(path.exists(), "missing ../roym_web/ui/src/i18n/en.ftl");
    let content = fs::read_to_string(&path).expect("Failed to read ../roym_web/ui/src/i18n/en.ftl");

    for (id, notice) in [
        ("request-data-use-notice", DEFAULT_DATA_USE_NOTICE),
        ("quote-address-disclosure-notice", ADDRESS_DISCLOSURE_NOTICE),
    ] {
        let line = format!("{id} = {notice}");
        assert!(
            content.lines().any(|l| l == line),
            "en.ftl must hold the whole line `{id} = <the Rust notice>`"
        );
    }
}
