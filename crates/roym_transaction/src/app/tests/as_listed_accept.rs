//! Service tests for the counterparty's automatic acceptance.

use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    area::Area,
    as_listed::{self, AsListed},
    backup::{Bundle, ImportInput, SECTION_LEDGER},
    deal::{
        CommitmentKind, DealTerms, Details, DetailsExtra, ExtensionRef, LocalText, ResourceKind,
        TextTranslation,
    },
    listing::{self, PaymentModel, ServiceLocation},
    method::NoParams,
    record::{Envelope, RECORD_QUOTE, RecordDraft},
    signing,
    testing::{
        TestHost,
        sd::{PlainQuote, conceal_quote},
    },
    transaction::{
        self, Answers, DEFAULT_DATA_USE_NOTICE, DealRole, ENQUIRY_VERSION, EnquiryPayload,
        QUOTE_VERSION, QuoteLocation, RECORD_ENQUIRY, TimeWindow, derive_quote_id, verify_quote,
    },
};
use z32;

use super::{
    as_listed_fixture::{CONVERSATION, lang, listing_payload, sign_listing},
    fixture,
};
use crate::app::{
    AGREEMENTS, AS_LISTED, AgreementRow, LEDGER,
    as_listed_ops::{
        AutoAcceptRow,
        accept::{AutoAccept, accept_waiting, auto_accept, auto_accept_id, same_terms},
    },
    backup, delete_row, disclosures,
    enquiry_ops::{
        self,
        as_listed::{AsListedRow, AsListedState},
    },
    get_row, put_row,
    quote_ops::store_received_quote,
};

/// Signs and holds an as-listed enquiry from a peer, and returns its record id.
async fn hold_enquiry(
    host: &TestHost,
    peer: &Identity,
    tap: &Tap,
    valid_until_secs: u64,
    now: u64,
    owner_did: &str,
) -> String {
    let peer_did = substrate::derive_did_key(&peer.public_key());
    let enquiry_id =
        transaction::derive_enquiry_id(CONVERSATION, &peer_did, 1).expect("enquiry id");
    let payload = EnquiryPayload {
        enquiry_id: enquiry_id.clone(),
        conversation: CONVERSATION.to_string(),
        sequence: 1,
        listing_id: Some("lst_tap".to_string()),
        categories: Vec::new(),
        description: None,
        as_listed: Some(AsListed {
            record_id: tap.record_id.clone(),
            slot: None,
            variant: None,
            valid_until_secs,
        }),
        area: None,
        window: None,
        data_use_notice: DEFAULT_DATA_USE_NOTICE.to_string(),
    };
    let draft = RecordDraft {
        version: ENQUIRY_VERSION,
        record_type: RECORD_ENQUIRY.to_string(),
        subject: enquiry_id,
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, peer_did, None, now).expect("envelope");
    let sig = z32::encode(&peer.sign(&bytes).to_bytes());
    env.attach_signature(sig).expect("signature");
    let envelope = env.to_json().expect("json");
    let verdict = transaction::verify_enquiry(&envelope, now);
    enquiry_ops::store_received_enquiry(host, &envelope, &verdict, &payload, now, owner_did)
        .await
        .expect("store enquiry");
    verdict.record_id.expect("enquiry record id")
}

/// Stores a quote from the owner that answers the tap, and returns its record
/// id.
async fn store_quote(
    host: &TestHost,
    counterparty_did: &str,
    owner: &Identity,
    enquiry_record: &str,
    terms: DealTerms,
    expires_at_secs: u64,
    now: u64,
) -> String {
    let issuer = substrate::derive_did_key(&owner.public_key());
    let quote_id = derive_quote_id(CONVERSATION, &issuer, 1).expect("quote id");
    let plain = PlainQuote {
        quote_id,
        conversation: CONVERSATION.to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: enquiry_record.to_string() },
        counterparty_did: counterparty_did.to_string(),
        terms,
    };
    let (payload, disclosures) = conceal_quote(plain, 7);
    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: CONVERSATION.to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: Some(expires_at_secs),
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, now).expect("envelope");
    let sig = z32::encode(&owner.sign(&bytes).to_bytes());
    env.attach_signature(sig).expect("signature");
    let envelope = env.to_json().expect("json");
    let verdict = verify_quote(&envelope, now);
    let record_id = verdict.record_id.clone().expect("record id");
    store_received_quote(host, &envelope, &verdict, &payload, now, counterparty_did)
        .await
        .expect("store quote");
    disclosures::store(host, &record_id, RECORD_QUOTE, &disclosures, now)
        .await
        .expect("store disclosures");
    record_id
}

struct Tap {
    envelope: String,
    record_id: String,
    terms: DealTerms,
}

/// The offering a person tapped, with the terms it builds.
fn tap(owner: &Identity, now: u64) -> Tap {
    let (envelope, payload) = sign_listing(owner, listing_payload(PaymentModel::Fixed), now);
    let verdict = listing::verify_envelope(&envelope, now);
    let record_id = verdict.record_id.expect("listing record");
    let consent = AsListed {
        record_id: record_id.clone(),
        slot: None,
        variant: None,
        valid_until_secs: now + 3_600,
    };
    let issuer = substrate::derive_did_key(&owner.public_key());
    let terms = as_listed::terms_from_listing(&payload, &issuer, &consent, now).expect("terms");
    Tap { envelope, record_id, terms }
}

async fn hold_tap(
    host: &TestHost,
    tap: &Tap,
    enquiry_record: &str,
    valid_until_secs: u64,
    now: u64,
) {
    let row = AsListedRow {
        enquiry_record_id: enquiry_record.to_string(),
        listing_id: "lst_tap".to_string(),
        conversation: CONVERSATION.to_string(),
        envelope: tap.envelope.clone(),
        record_id: tap.record_id.clone(),
        slot: None,
        valid_until_secs,
        viewed_lang: "en".to_string(),
        viewed_via_mt: false,
        state: AsListedState::Waiting,
        reason: None,
        quote_record_id: None,
        created_at_secs: now,
    };
    put_row(host, AS_LISTED, enquiry_record, &row).await.expect("store tap");
}

async fn held_tap(host: &TestHost, enquiry_record: &str) -> AsListedRow {
    get_row(host, AS_LISTED, enquiry_record).await.expect("read").expect("tap row")
}

#[tokio::test]
async fn a_quote_with_the_listed_terms_is_accepted_with_no_person() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &owner,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now,
    )
    .await;

    let outcome =
        auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("auto accept");
    assert_eq!(outcome, AutoAccept::Accepted);
    assert_eq!(held_tap(&f.host, &enquiry_record).await.state, AsListedState::Accepted);

    let agreement: AgreementRow =
        get_row(&f.host, AGREEMENTS, &quote_record).await.expect("read").expect("agreement");
    assert!(agreement.counterparty.is_some(), "the counterparty half is signed");
}

#[tokio::test]
async fn a_quote_that_differs_in_any_field_is_shown() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;

    let mut different = tap.terms.clone();
    different.commitments[1].payee = Some("Someone Else".to_string());
    assert!(!same_terms(&tap.terms, &different, now, now));

    let quote_record =
        store_quote(&f.host, &f.master_did, &owner, &enquiry_record, different, now + 3_600, now)
            .await;
    let outcome =
        auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("auto accept");
    assert_eq!(outcome, AutoAccept::Shown("terms-differ".to_string()));
    let row = held_tap(&f.host, &enquiry_record).await;
    assert_eq!(row.state, AsListedState::Shown);
    assert_eq!(row.reason.as_deref(), Some("terms-differ"));
}

/// Every field of the terms and of a commitment, changed one at a time. A
/// field added to either type and not named here fails the coverage check.
const TERM_FIELDS: &[&str] = &[
    "address",
    "after",
    "area",
    "cancellation_terms",
    "code",
    "commitment_id",
    "commitments",
    "currency",
    "data",
    "declared_value_minor",
    "description",
    "details",
    "dispute_path",
    "earliest_secs",
    "eep",
    "extension",
    "extra",
    "fees_minor",
    "giver",
    "home_exchange",
    "id",
    "kind",
    "label",
    "lang",
    "latest_secs",
    "listing_id",
    "location",
    "payment_methods",
    "payee",
    "quote_expires_at_secs",
    "refund_terms",
    "resource_kind",
    "schema",
    "scope",
    "tax_minor",
    "text",
    "translations",
    "value",
    "version",
    "where",
    "window",
];

/// The keys of a serialised value, not descending into the two free-form
/// blocks whose contents are the extension's, not the terms'.
fn keys(value: &Value, out: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                out.insert(key.clone());
                if key != "data" && key != "eep" {
                    keys(inner, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                keys(item, out);
            }
        }
        _ => {}
    }
}

fn each_text(terms: &mut DealTerms) -> Vec<&mut LocalText> {
    let mut texts = vec![
        &mut terms.scope,
        &mut terms.cancellation_terms,
        &mut terms.refund_terms,
        &mut terms.dispute_path,
    ];
    for commitment in &mut terms.commitments {
        texts.push(&mut commitment.description);
    }
    texts
}

/// Changes the named field of every commitment, and says whether the name is
/// a field this test knows.
fn change_field(terms: &mut DealTerms, field: &str) -> bool {
    match field {
        "currency" => terms.currency.push('X'),
        "quote_expires_at_secs" => terms.quote_expires_at_secs = 0,
        "home_exchange" => terms.home_exchange = Some("other".to_string()),
        "commitments" => {
            let first = terms.commitments[0].clone();
            terms.commitments.push(first);
        }
        "listing_id" => terms.listing_id = Some("lst_other".to_string()),
        _ => {
            return change_text_field(terms, field)
                || change_commitment_field(terms, field)
                || change_details_field(terms, field)
                || change_place_field(terms, field);
        }
    }
    true
}

fn change_text_field(terms: &mut DealTerms, field: &str) -> bool {
    match field {
        "lang" => {
            for text in each_text(terms) {
                text.lang = lang("de");
                for tr in &mut text.translations {
                    tr.lang = lang("de");
                }
            }
        }
        "text" => {
            for text in each_text(terms) {
                text.text.push('X');
                for tr in &mut text.translations {
                    tr.text.push('X');
                }
            }
        }
        "translations" => {
            for text in each_text(terms) {
                text.translations
                    .push(TextTranslation { lang: lang("fr"), text: "Autre".to_string() });
            }
        }
        "scope" => terms.scope.text.push('X'),
        "cancellation_terms" => terms.cancellation_terms.text.push('X'),
        "refund_terms" => terms.refund_terms.text.push('X'),
        "dispute_path" => terms.dispute_path.text.push('X'),
        "description" => terms.commitments[0].description.text.push('X'),
        _ => return false,
    }
    true
}

fn change_commitment_field(terms: &mut DealTerms, field: &str) -> bool {
    match field {
        "commitment_id" => terms.commitments[0].commitment_id.push('X'),
        "giver" => terms.commitments[0].giver = DealRole::Counterparty,
        "resource_kind" => terms.commitments[0].resource_kind = ResourceKind::Currency,
        "kind" => terms.commitments[0].kind = CommitmentKind::Currency,
        "extension" => {
            for commitment in &mut terms.commitments {
                commitment.extension = Some(ExtensionRef { id: "other".to_string(), version: 2 });
            }
        }
        "after" => terms.commitments[0].after.push("x".to_string()),
        "declared_value_minor" => terms.commitments[0].declared_value_minor += 1,
        "payee" => terms.commitments[0].payee = Some("other".to_string()),
        "payment_methods" => {
            for commitment in &mut terms.commitments {
                if let Some(methods) = &mut commitment.payment_methods {
                    methods.push("other".to_string());
                }
            }
        }
        "tax_minor" => terms.commitments[0].tax_minor = Some(2),
        "fees_minor" => terms.commitments[0].fees_minor = Some(2),
        "eep" => terms.commitments[0].eep = Some(json!({"k": 2})),
        _ => return false,
    }
    true
}

fn change_details_field(terms: &mut DealTerms, field: &str) -> bool {
    match field {
        "details" => {
            for commitment in &mut terms.commitments {
                if let Some(details) = &mut commitment.details {
                    details.schema.push('X');
                }
            }
        }
        "id" => {
            for commitment in &mut terms.commitments {
                if let Some(ext) = &mut commitment.extension {
                    ext.id.push('X');
                }
            }
        }
        "version" => {
            for commitment in &mut terms.commitments {
                if let Some(ext) = &mut commitment.extension {
                    ext.version += 1;
                }
            }
        }
        "schema" => {
            for commitment in &mut terms.commitments {
                if let Some(details) = &mut commitment.details {
                    details.schema.push('X');
                }
            }
        }
        "data" => {
            for commitment in &mut terms.commitments {
                if let Some(details) = &mut commitment.details {
                    details.data = json!({"x": 2});
                }
            }
        }
        "extra" => {
            for commitment in &mut terms.commitments {
                if let Some(details) = &mut commitment.details {
                    details
                        .extra
                        .push(DetailsExtra { label: "x".to_string(), value: "y".to_string() });
                }
            }
        }
        "value" => {
            for commitment in &mut terms.commitments {
                if let Some(details) = &mut commitment.details {
                    for extra in &mut details.extra {
                        extra.value.push('X');
                    }
                }
            }
        }
        _ => return false,
    }
    true
}

fn change_place_field(terms: &mut DealTerms, field: &str) -> bool {
    match field {
        "window" => {
            for commitment in &mut terms.commitments {
                if let Some(window) = &mut commitment.window {
                    window.earliest_secs += 1;
                }
            }
        }
        "location" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location {
                    location.address = Some("other".to_string());
                }
            }
        }
        "earliest_secs" => {
            for commitment in &mut terms.commitments {
                if let Some(window) = &mut commitment.window {
                    window.earliest_secs += 1;
                }
            }
        }
        "latest_secs" => {
            for commitment in &mut terms.commitments {
                if let Some(window) = &mut commitment.window {
                    window.latest_secs += 1;
                }
            }
        }
        "where" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location {
                    location.where_ = ServiceLocation::AtReceiver;
                }
            }
        }
        "area" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location {
                    location.area = Some(Area::Named {
                        label: "other".to_string(),
                        code: Some("x".to_string()),
                    });
                }
            }
        }
        "address" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location {
                    location.address = Some("other".to_string());
                }
            }
        }
        "label" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location
                    && let Some(Area::Named { label, .. }) = &mut location.area
                {
                    label.push('X');
                }
            }
        }
        "code" => {
            for commitment in &mut terms.commitments {
                if let Some(location) = &mut commitment.location
                    && let Some(Area::Named { code, .. }) = &mut location.area
                {
                    *code = Some("other".to_string());
                }
            }
        }
        _ => return false,
    }
    true
}

/// A value with every field set, so the coverage walk sees every key.
fn full_terms(base: &DealTerms, now: u64) -> DealTerms {
    let mut terms = base.clone();
    terms.quote_expires_at_secs = now + 3_600;
    terms.home_exchange = Some("RE".to_string());
    terms.listing_id = Some("lst_full".to_string());
    for text in each_text(&mut terms) {
        text.translations =
            vec![TextTranslation { lang: lang("fr"), text: "Traduction".to_string() }];
    }
    for commitment in &mut terms.commitments {
        commitment.extension = Some(ExtensionRef { id: "scheduling".to_string(), version: 1 });
        commitment.after = vec!["c1".to_string()];
        commitment.details = Some(Details {
            extension: "scheduling".to_string(),
            schema: "scheduling/slot@1".to_string(),
            data: json!({"slot_id": "slot_1"}),
            extra: vec![DetailsExtra { label: "label".to_string(), value: "value".to_string() }],
        });
        commitment.window = Some(TimeWindow { earliest_secs: now, latest_secs: now + 3_600 });
        commitment.location = Some(QuoteLocation {
            where_: ServiceLocation::AtGiver,
            area: Some(Area::Named { label: "area".to_string(), code: Some("code".to_string()) }),
            address: Some("address".to_string()),
        });
        commitment.payee = Some("payee".to_string());
        commitment.payment_methods = Some(vec!["cash".to_string()]);
        commitment.tax_minor = Some(1);
        commitment.fees_minor = Some(1);
        commitment.eep = Some(json!({"k": 1}));
    }
    terms
}

#[test]
fn a_changed_field_anywhere_in_the_terms_fails_the_check() {
    let now = 1_700_000_000;
    let owner = Identity::generate().expect("owner");
    let tap = tap(&owner, now);
    let base = full_terms(&tap.terms, now);
    assert!(same_terms(&base, &base, now, now));

    let mut seen = std::collections::BTreeSet::new();
    keys(&serde_json::to_value(&base).expect("terms json"), &mut seen);
    let expected: std::collections::BTreeSet<&str> = TERM_FIELDS.iter().copied().collect();
    assert_eq!(seen, expected.into_iter().map(str::to_string).collect());

    for field in TERM_FIELDS {
        let mut changed = base.clone();
        assert!(change_field(&mut changed, field), "no change for field '{field}'");
        assert!(!same_terms(&base, &changed, now, now), "field '{field}' must fail the check");
    }
}

#[tokio::test]
async fn a_quote_signed_by_another_identity_is_shown() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let other = Identity::generate().expect("other signer");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;

    // Another key signs the listed terms. The owner of the offering is not
    // the quote's issuer, so the counterparty must show the quote, not let it
    // bind this person with no screen.
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &other,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now,
    )
    .await;
    let outcome =
        auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("auto accept");
    assert_eq!(outcome, AutoAccept::Shown("not-complete".to_string()));
    assert_eq!(held_tap(&f.host, &enquiry_record).await.state, AsListedState::Shown);
}

#[tokio::test]
async fn a_quote_issued_after_the_consent_time_is_shown() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 60, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 60, now).await;
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &owner,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now + 61,
    )
    .await;

    let outcome =
        auto_accept(&f.host, &enquiry_record, &quote_record, now + 61).await.expect("auto accept");
    assert_eq!(outcome, AutoAccept::Shown("consent-expired".to_string()));
}

#[tokio::test]
async fn the_acceptance_carries_the_stored_viewed_fields() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;
    let mut row = held_tap(&f.host, &enquiry_record).await;
    row.viewed_lang = "hi".to_string();
    row.viewed_via_mt = true;
    put_row(&f.host, AS_LISTED, &enquiry_record, &row).await.expect("update tap");
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &owner,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now,
    )
    .await;

    auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("auto accept");
    let agreement: AgreementRow =
        get_row(&f.host, AGREEMENTS, &quote_record).await.expect("read").expect("agreement");
    let half = agreement.counterparty.expect("half");
    let verdict = transaction::verify_agreement_receipt(&half.envelope, now);
    let payload = verdict.payload.expect("payload");
    assert_eq!(payload.viewed_lang, lang("hi"));
    assert!(payload.viewed_via_mt);
}

#[tokio::test]
async fn a_signing_error_keeps_the_row_waiting_and_the_next_drive_accepts() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &owner,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now,
    )
    .await;

    let certificate: signing::StoredCertificate =
        get_row(&f.host, signing::CERTIFICATES, signing::CERTIFICATE_ID)
            .await
            .expect("read")
            .expect("certificate");
    delete_row(&f.host, signing::CERTIFICATES, signing::CERTIFICATE_ID)
        .await
        .expect("remove certificate");
    let first =
        auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("auto accept");
    assert_eq!(first, AutoAccept::Waiting);
    let row = held_tap(&f.host, &enquiry_record).await;
    assert_eq!(row.state, AsListedState::Waiting);
    assert_eq!(row.reason.as_deref(), Some("signing-not-ready"));

    put_row(&f.host, signing::CERTIFICATES, signing::CERTIFICATE_ID, &certificate)
        .await
        .expect("restore certificate");
    let mut warnings = Vec::new();
    accept_waiting(&f.host, now, &mut warnings).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(held_tap(&f.host, &enquiry_record).await.state, AsListedState::Accepted);
    let agreement: AgreementRow =
        get_row(&f.host, AGREEMENTS, &quote_record).await.expect("read").expect("agreement");
    assert!(agreement.counterparty.is_some(), "the retry signs the acceptance half");
}

#[tokio::test]
async fn one_tap_accepts_at_most_one_quote() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &owner,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now,
    )
    .await;

    put_row(
        &f.host,
        LEDGER,
        &auto_accept_id(&enquiry_record),
        &AutoAcceptRow {
            kind: crate::app::ledger::LedgerKind::AutoAccept,
            enquiry_record_id: enquiry_record.clone(),
            quote_record_id: "rec_other_quote".to_string(),
            created_at_secs: now,
        },
    )
    .await
    .expect("fence");

    let outcome =
        auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("auto accept");
    assert_eq!(outcome, AutoAccept::Shown("already-accepted".to_string()));
}

#[tokio::test]
async fn the_import_checks_autoaccept_rows() {
    let f = fixture();
    let now = f.host.now();
    let owner = Identity::generate().expect("owner");
    let peer = Identity::generate().expect("peer");
    let tap = tap(&owner, now);
    let enquiry_record = hold_enquiry(&f.host, &peer, &tap, now + 3_600, now, &f.master_did).await;
    hold_tap(&f.host, &tap, &enquiry_record, now + 3_600, now).await;
    let quote_record = store_quote(
        &f.host,
        &f.master_did,
        &owner,
        &enquiry_record,
        tap.terms.clone(),
        now + 3_600,
        now,
    )
    .await;
    auto_accept(&f.host, &enquiry_record, &quote_record, now).await.expect("accept");

    let bundle = backup::export(&f.host, NoParams {}).await.expect("export");
    let mut sections = bundle.sections.clone();
    let rows = sections.get_mut(SECTION_LEDGER).expect("ledger");
    let row = rows
        .iter_mut()
        .find(|row| row["id"].as_str().is_some_and(|id| id.starts_with("autoaccept:")))
        .expect("autoaccept row");
    row["payload"]["quote_record_id"] = json!("rec_unknown");
    let mut tampered =
        Bundle::new(f.master_did.clone(), crate::app::SCHEMA_VERSION, sections).expect("bundle");
    signing::sign_bundle(&f.host, &mut tampered, now).await.expect("sign");

    let err = backup::import(&f.host, ImportInput { bundle: tampered }).await.expect_err("refused");
    let message = err.error.expect("error").message;
    assert!(message.contains("autoaccept:"), "{message}");
    assert!(message.contains("unheld quote"), "{message}");
}
